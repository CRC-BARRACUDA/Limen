//! The C ABI a native module is loaded through, and the macro that writes it.
//!
//! A module author never names anything in here - `export_module!` does.

use core::ffi::c_void;
use std::sync::Mutex;

use crate::*;

/// Internal runtime the [`export_module!`] macro calls into. Not part of the
/// stable API — do not use directly.
#[doc(hidden)]
pub mod __rt {
    use super::*;

    /// Boxed per-module state, stored behind the opaque ABI handle.
    pub struct State<H: Handler> {
        handler: Mutex<H>,
        host: Host,
    }

    /// # Safety
    /// Called by the generated `limen_module_init`. `host_call` must be the
    /// host's real callback and `host_ctx` must stay valid for the module's life.
    pub unsafe fn init<H: Handler + 'static>(
        host_ctx: *mut c_void,
        host_call: HostCallFn,
    ) -> *mut c_void {
        let state = Box::new(State::<H> {
            handler: Mutex::new(H::default()),
            host: Host { host_ctx, host_call },
        });
        Box::into_raw(state) as *mut c_void
    }

    /// # Safety
    /// `handle` must have come from [`init`] for the same `H`.
    pub unsafe fn call<H: Handler + 'static>(
        handle: *mut c_void,
        method_ptr: *const u8,
        method_len: usize,
        params_ptr: *const u8,
        params_len: usize,
        sink: SinkFn,
        sink_ctx: *mut c_void,
    ) { unsafe {
        let state = &*(handle as *const State<H>);
        let method = String::from_utf8_lossy(bytes(method_ptr, method_len)).into_owned();
        let params: Value =
            serde_json::from_slice(bytes(params_ptr, params_len)).unwrap_or(Value::Null);

        let result = {
            let mut handler = state.handler.lock().unwrap();
            dispatch(&mut *handler, &state.host, &method, params)
        };
        let (is_error, out) = match result {
            Ok(v) => (0, serde_json::to_vec(&v).unwrap_or_default()),
            Err(e) => (1, serde_json::to_vec(&e).unwrap_or_default()),
        };
        sink(sink_ctx, is_error, out.as_ptr(), out.len());
    }}

    /// # Safety
    /// `handle` must have come from [`init`] for the same `H`; not used after.
    pub unsafe fn shutdown<H: Handler + 'static>(handle: *mut c_void) { unsafe {
        drop(Box::from_raw(handle as *mut State<H>));
    }}

    /// Translate the host's lifecycle methods into [`Handler`] calls.
    fn dispatch<H: Handler>(
        handler: &mut H,
        host: &Host,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(json!({ "capabilities": handler.capabilities() })),
            "describe" => Ok(json!({ "capabilities": handler.capabilities() })),
            "invoke" => {
                let cap = params.get("capability").and_then(Value::as_str).unwrap_or("");
                let m = params.get("method").and_then(Value::as_str).unwrap_or("");
                let p = params.get("params").cloned().unwrap_or(Value::Null);
                handler.invoke(cap, m, p, host)
            }
            "shutdown" => Ok(Value::Null),
            other => Err(RpcError::new(
                rpc::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        }
    }

    unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] { unsafe {
        if ptr.is_null() || len == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(ptr, len)
        }
    }}
}

/// Emit the C-ABI symbols the host looks up, wiring them to your [`Handler`].
#[macro_export]
macro_rules! export_module {
    ($ty:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn limen_abi_version() -> u32 {
            $crate::ABI_VERSION
        }

        /// # Safety: called only by the Limen host per the ABI contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn limen_module_init(
            host_ctx: *mut ::core::ffi::c_void,
            host_call: $crate::HostCallFn,
        ) -> *mut ::core::ffi::c_void {
            $crate::__rt::init::<$ty>(host_ctx, host_call)
        }

        /// # Safety: called only by the Limen host per the ABI contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn limen_module_call(
            handle: *mut ::core::ffi::c_void,
            method_ptr: *const u8,
            method_len: usize,
            params_ptr: *const u8,
            params_len: usize,
            sink: $crate::SinkFn,
            sink_ctx: *mut ::core::ffi::c_void,
        ) {
            $crate::__rt::call::<$ty>(
                handle, method_ptr, method_len, params_ptr, params_len, sink, sink_ctx,
            )
        }

        /// # Safety: called only by the Limen host per the ABI contract.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn limen_module_shutdown(handle: *mut ::core::ffi::c_void) {
            $crate::__rt::shutdown::<$ty>(handle)
        }
    };
}
