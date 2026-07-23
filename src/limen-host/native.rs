//! The in-process native transport: load a dynamic library and call it directly.
//!
//! This is the "fast path" — no subprocess, no pipe. The host dlopen's the
//! module, checks its ABI version, and calls its exported functions. The module
//! can call back into the host (to reach other modules via the broker) through a
//! trampoline we hand it at init time. Everything crosses the boundary as JSON
//! bytes via the sink pattern documented in [`limen_proto::abi`].
//!
//! Calls into a given module are serialized by a `Mutex`, so a module need not
//! be internally thread-safe. (One caveat: a module that re-enters *itself*
//! through the broker while a call is in flight would deadlock on that mutex —
//! cross-module calls are fine.)

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use libloading::{Library, Symbol};
use limen_proto::abi::{
    AbiVersionFn, ModuleCallFn, ModuleInitFn, ModuleShutdownFn, SinkFn, ABI_VERSION,
    SYM_ABI_VERSION, SYM_CALL, SYM_INIT, SYM_SHUTDOWN,
};
use limen_proto::rpc::INTERNAL_ERROR;
use limen_proto::RpcError;
use serde_json::Value;

use crate::module::{IncomingHandler, Module};

/// Passed to the module as `host_ctx`; the trampoline recovers it to route
/// module→host calls through the shared handler.
struct HostCtx {
    handler: Arc<IncomingHandler>,
}

/// Makes the opaque module handle `Send + Sync`. Justified because every call
/// into the module is serialized by [`NativeModule::lock`].
struct Handle(*mut c_void);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

pub struct NativeModule {
    name: String,
    handle: Handle,
    call_fn: ModuleCallFn,
    shutdown_fn: ModuleShutdownFn,
    shutdown_done: AtomicBool,
    lock: Mutex<()>,
    // Kept alive for the module's lifetime. `_host_ctx` must outlive the module
    // (the trampoline dereferences it); `_lib` must be dropped last (it unloads
    // the code). Declared after the call fields so Drop can run first.
    _host_ctx: Box<HostCtx>,
    _lib: Library,
}

impl NativeModule {
    /// dlopen `lib_path`, verify its ABI, initialize it, and return a handle.
    pub fn load(name: String, lib_path: &str, handler: Arc<IncomingHandler>) -> Result<Arc<Self>> {
        let lib = unsafe { Library::new(lib_path) }
            .with_context(|| format!("loading native module {name} from {lib_path}"))?;

        unsafe {
            let abi_version: Symbol<AbiVersionFn> = lib
                .get(SYM_ABI_VERSION)
                .with_context(|| format!("{name}: missing limen_abi_version"))?;
            let found = abi_version();
            if found != ABI_VERSION {
                bail!("native module {name} is ABI v{found}, host speaks v{ABI_VERSION}");
            }

            let init: Symbol<ModuleInitFn> = lib
                .get(SYM_INIT)
                .with_context(|| format!("{name}: missing limen_module_init"))?;
            let call_sym: Symbol<ModuleCallFn> = lib
                .get(SYM_CALL)
                .with_context(|| format!("{name}: missing limen_module_call"))?;
            let shutdown_sym: Symbol<ModuleShutdownFn> = lib
                .get(SYM_SHUTDOWN)
                .with_context(|| format!("{name}: missing limen_module_shutdown"))?;
            let call_fn = *call_sym;
            let shutdown_fn = *shutdown_sym;

            // Box the context so its address is stable, then hand its pointer to
            // the module. We keep the Box in the struct to keep it alive.
            let host_ctx = Box::new(HostCtx { handler });
            let host_ctx_ptr = (&*host_ctx as *const HostCtx) as *mut c_void;

            let handle = init(host_ctx_ptr, host_call_trampoline);
            if handle.is_null() {
                bail!("native module {name}: limen_module_init returned null");
            }

            Ok(Arc::new(Self {
                name,
                handle: Handle(handle),
                call_fn,
                shutdown_fn,
                shutdown_done: AtomicBool::new(false),
                lock: Mutex::new(()),
                _host_ctx: host_ctx,
                _lib: lib,
            }))
        }
    }

    fn do_shutdown(&self) {
        if !self.shutdown_done.swap(true, Ordering::SeqCst) {
            let _guard = self.lock.lock().unwrap();
            unsafe { (self.shutdown_fn)(self.handle.0) };
        }
    }
}

impl Module for NativeModule {
    fn name(&self) -> &str {
        &self.name
    }

    fn call(&self, method: &str, params: Value) -> std::result::Result<Value, RpcError> {
        let params_bytes = serde_json::to_vec(&params)
            .map_err(|e| RpcError::new(INTERNAL_ERROR, format!("serialize params: {e}")))?;

        let mut captured: Option<(i32, Vec<u8>)> = None;
        {
            let _guard = self.lock.lock().unwrap();
            unsafe {
                (self.call_fn)(
                    self.handle.0,
                    method.as_ptr(),
                    method.len(),
                    params_bytes.as_ptr(),
                    params_bytes.len(),
                    capture_sink,
                    (&mut captured as *mut Option<(i32, Vec<u8>)>) as *mut c_void,
                );
            }
        }

        match captured {
            Some((0, bytes)) => serde_json::from_slice(&bytes)
                .map_err(|e| RpcError::new(INTERNAL_ERROR, format!("decode result: {e}"))),
            Some((_, bytes)) => Err(serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| RpcError::new(INTERNAL_ERROR, "native module error"))),
            None => Err(RpcError::new(
                INTERNAL_ERROR,
                format!("native module {} produced no result", self.name),
            )),
        }
    }

    fn shutdown(&self) {
        self.do_shutdown();
    }
}

impl Drop for NativeModule {
    fn drop(&mut self) {
        // Ensure the module tears down before its library is unloaded.
        self.do_shutdown();
    }
}

/// Sink used by [`NativeModule::call`] to capture the module's result bytes.
unsafe extern "C" fn capture_sink(ctx: *mut c_void, is_error: i32, ptr: *const u8, len: usize) {
    let out = &mut *(ctx as *mut Option<(i32, Vec<u8>)>);
    *out = Some((is_error, bytes(ptr, len).to_vec()));
}

/// Handed to the module as its `host_call`. Recovers the [`HostCtx`], runs the
/// shared handler, and returns the result through the module's sink.
unsafe extern "C" fn host_call_trampoline(
    host_ctx: *mut c_void,
    method_ptr: *const u8,
    method_len: usize,
    params_ptr: *const u8,
    params_len: usize,
    sink: SinkFn,
    sink_ctx: *mut c_void,
) {
    let ctx = &*(host_ctx as *const HostCtx);
    let method = String::from_utf8_lossy(bytes(method_ptr, method_len)).into_owned();
    let params: Value = serde_json::from_slice(bytes(params_ptr, params_len)).unwrap_or(Value::Null);

    let (is_error, out) = match (ctx.handler)(&method, params) {
        Ok(v) => (0, serde_json::to_vec(&v).unwrap_or_default()),
        Err(e) => (1, serde_json::to_vec(&e).unwrap_or_default()),
    };
    sink(sink_ctx, is_error, out.as_ptr(), out.len());
}

unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}
