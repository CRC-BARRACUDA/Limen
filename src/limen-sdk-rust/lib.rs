//! SDK for writing **native (in-process) Limen modules** in Rust.
//!
//! Implement [`Handler`], then invoke [`export_module!`] with your type. The
//! macro emits the C-ABI symbols the host looks up ([`limen_proto::abi`]) and
//! this crate handles all the unsafe marshalling, so a module is essentially
//! just its business logic:
//!
//! ```ignore
//! use limen_sdk_rust::{export_module, json, rpc, Handler, Host, RpcError, Value};
//!
//! #[derive(Default)]
//! struct MyModule;
//!
//! impl Handler for MyModule {
//!     fn capabilities(&self) -> Vec<String> { vec!["my.thing".into()] }
//!     fn invoke(&mut self, _cap: &str, method: &str, params: Value, host: &Host)
//!         -> Result<Value, RpcError>
//!     {
//!         match method {
//!             "hello" => Ok(json!({ "ok": true })),
//!             other => Err(RpcError::new(rpc::METHOD_NOT_FOUND, format!("no {other}"))),
//!         }
//!     }
//! }
//!
//! export_module!(MyModule);
//! ```

use core::ffi::c_void;
use std::sync::Mutex;

pub mod catalog;
pub mod ui;

pub use catalog::Catalog;
pub use limen_proto::abi::{HostCallFn, SinkFn, ABI_VERSION};
pub use limen_proto::{rpc, RpcError};
pub use serde_json::{json, Value};

/// A native module's logic. The host calls `invoke` for each request; `host`
/// lets the module reach other modules through the broker.
pub trait Handler: Default + Send {
    /// The capabilities this module provides (reported on `initialize`).
    fn capabilities(&self) -> Vec<String>;

    /// Handle an invocation of `capability`.`method` with `params`.
    fn invoke(
        &mut self,
        capability: &str,
        method: &str,
        params: Value,
        host: &Host,
    ) -> Result<Value, RpcError>;
}

/// A bridge back to the host, for calling other modules via the broker.
pub struct Host {
    host_ctx: *mut c_void,
    host_call: HostCallFn,
}

impl Host {
    /// Call `method` on whichever module provides `capability`.
    pub fn call(
        &self,
        capability: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        let payload = json!({ "capability": capability, "method": method, "params": params });
        self.raw("host.call", payload)
    }

    /// Every capability currently provided by a loaded module. Use this to
    /// discover **optional** integrations — e.g. only show a "Make Report" button
    /// when a `report.*` provider is present.
    pub fn capabilities(&self) -> Vec<String> {
        self.raw("host.capabilities", Value::Null)
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    /// Whether some loaded module provides `capability` (exact match).
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities().iter().any(|c| c == capability)
    }

    /// Emit a log line to the host console.
    pub fn log(&self, message: &str) {
        let _ = self.raw("host.log", Value::String(message.to_string()));
    }

    /// Ask the host to open something in the OS on the user's behalf. `target`
    /// is one of `"path"`, `"url"`, `"registry"`, or `"device_manager"`; `value`
    /// is the path / URL / registry key (ignored for `device_manager`).
    /// Best-effort — registry/device_manager are Windows-only.
    pub fn open(&self, target: &str, value: &str) {
        let _ = self.raw("host.open", json!({ "target": target, "value": value }));
    }

    /// Show a native "open file" dialog on the host; returns the chosen path, or
    /// `None` if the user cancelled.
    pub fn pick_file(&self) -> Option<String> {
        self.raw("host.pick_file", Value::Null)
            .ok()
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(String::from))
    }

    /// The user's active UI language code (e.g. `"en"`, `"uk"`). Query this while
    /// building a view and translate your own strings (see [`Catalog`]) so the
    /// module's screens match the rest of the app. Defaults to `"en"`.
    pub fn locale(&self) -> String {
        self.raw("host.locale", Value::Null)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "en".to_string())
    }

    fn raw(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let params_bytes = serde_json::to_vec(&params).unwrap_or_default();
        let mut captured: Option<(i32, Vec<u8>)> = None;
        unsafe {
            (self.host_call)(
                self.host_ctx,
                method.as_ptr(),
                method.len(),
                params_bytes.as_ptr(),
                params_bytes.len(),
                sink,
                (&mut captured as *mut Option<(i32, Vec<u8>)>) as *mut c_void,
            );
        }
        match captured {
            Some((0, bytes)) => serde_json::from_slice(&bytes)
                .map_err(|e| RpcError::new(rpc::INTERNAL_ERROR, format!("decode host result: {e}"))),
            Some((_, bytes)) => Err(serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| RpcError::new(rpc::INTERNAL_ERROR, "host error"))),
            None => Err(RpcError::new(rpc::INTERNAL_ERROR, "host produced no result")),
        }
    }
}

unsafe extern "C" fn sink(ctx: *mut c_void, is_error: i32, ptr: *const u8, len: usize) { unsafe {
    let out = &mut *(ctx as *mut Option<(i32, Vec<u8>)>);
    let bytes = if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(ptr, len).to_vec()
    };
    *out = Some((is_error, bytes));
}}

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
