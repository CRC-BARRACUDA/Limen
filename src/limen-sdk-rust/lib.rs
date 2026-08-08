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

/// Where a started elevation has got to.
#[derive(Debug, Clone)]
pub enum ElevateState {
    /// The operating system is asking the user, and nothing has run yet.
    Authorizing,
    /// It was authorized and the command is running.
    Running,
    /// It is over — well or badly.
    Done(Elevated),
}

/// What [`Host::elevate`] came back with.
///
/// `ran` distinguishes "the user refused" from "it ran and failed", which a bare
/// exit code cannot: a dismissed prompt and a command that exited non-zero are
/// different things to report.
#[derive(Debug, Clone)]
pub struct Elevated {
    /// Whether the command actually started with privileges.
    pub ran: bool,
    /// Its exit status, where the platform could report one.
    pub code: Option<i32>,
    /// A fixed word, so the module can act on it *and* say it in the user's
    /// language: `""` (ran cleanly), `"unavailable"`, `"refused"`, `"failed"`,
    /// `"error"`.
    pub reason: String,
    /// The English detail, for logs and as a fallback.
    pub message: String,
}

impl Elevated {
    /// It ran and exited cleanly.
    pub fn ok(&self) -> bool {
        self.ran && matches!(self.code, Some(0) | None)
    }
    /// Nothing on this machine can ask for privileges — no polkit, no graphical
    /// sudo. Worth telling the user, since the fix is theirs: install one, or
    /// start Limen as root.
    pub fn unavailable(&self) -> bool {
        self.reason == "unavailable"
    }
    /// The user was asked and said no.
    pub fn refused(&self) -> bool {
        self.reason == "refused"
    }
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

    /// This module's own directory on disk.
    ///
    /// A module that manages content of its own — a downloaded scanner, a rule
    /// set — keeps it under `tools/` in here. That subdirectory is deliberately
    /// excluded from the module's trust digest, so filling it does not revoke
    /// the module's approval; it is also removed with the module, and wiped when
    /// the module updates, so a new module version starts from a clean slate and
    /// can require a different tool version.
    pub fn module_dir(&self) -> Option<String> {
        self.raw("host.module_dir", Value::Null)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .filter(|s| !s.is_empty())
    }

    /// Raise a desktop notification on the machine running Limen.
    ///
    /// For work the user is not watching — a long scan finishing, an install
    /// landing. `urgency` is `"low"`, `"normal"` or `"critical"`; anything else
    /// is treated as `"normal"`. Best-effort: a session with no notification
    /// daemon simply shows nothing, and that is not an error.
    pub fn notify(&self, title: &str, body: &str, urgency: &str) {
        let _ = self.raw(
            "host.notify",
            json!({ "title": title, "body": body, "urgency": urgency }),
        );
    }

    /// Run a command with administrator / root privileges, letting the operating
    /// system ask the user for them.
    ///
    /// Limen stays unprivileged — what is elevated is the command. Requires
    /// `elevate = true` under `[permissions]`; without it the host refuses,
    /// which is the point: a module cannot reach for root without having said so
    /// where the user can read it.
    ///
    /// **Blocks** until the command finishes, so call it from a worker thread
    /// rather than from a method that has to return a view.
    ///
    /// Pass absolute paths. Elevation replaces the environment with a minimal
    /// one, so anything resolved through `PATH` will not be found.
    pub fn elevate(&self, argv: &[&str], cwd: Option<&str>) -> Elevated {
        let mut params = json!({ "argv": argv });
        if let Some(d) = cwd {
            params["cwd"] = json!(d);
        }
        self.elevate_call(params)
    }

    /// Start an elevated command and return an id to poll with
    /// [`Host::elevate_status`], instead of waiting for it.
    ///
    /// For anything long: waiting blocks every other call this module makes,
    /// including the one that draws its own progress. The authorization prompt
    /// alone can sit there indefinitely — the user may be looking at a password
    /// dialog, or may have walked away.
    pub fn elevate_async(&self, argv: &[&str], cwd: Option<&str>) -> Option<u64> {
        let mut params = json!({ "argv": argv, "wait": false });
        if let Some(d) = cwd {
            params["cwd"] = json!(d);
        }
        self.raw("host.elevate", params)
            .ok()
            .and_then(|v| v.get("id").and_then(Value::as_u64))
    }

    /// Where a started elevation has got to.
    ///
    /// The distinction that matters is between *asking* and *running*: a caller
    /// showing "waiting for authorization" has to know when to stop, and the
    /// program's own output is a poor proxy — it can lag the prompt by seconds,
    /// leaving the message up long after the answer was given.
    pub fn elevate_state(&self, id: u64) -> ElevateState {
        let Ok(v) = self.raw("host.elevate_status", json!({ "id": id })) else {
            return ElevateState::Running;
        };
        if v.get("running").and_then(Value::as_bool).unwrap_or(false) {
            return match v.get("phase").and_then(Value::as_str) {
                Some("running") => ElevateState::Running,
                _ => ElevateState::Authorizing,
            };
        }
        ElevateState::Done(Self::elevated_from(&v))
    }

    /// Ask an elevation to stop.
    ///
    /// Best effort: once authorized the command runs as root, and an
    /// unprivileged process cannot signal one. Returns whether it actually
    /// stopped — `false` means it is still running and the user should be told
    /// so rather than shown a screen that implies otherwise.
    /// Returns `Ok(())` if it stopped, or the id of an elevation now asking the
    /// user for the privileges to end it — poll that with
    /// [`Host::elevate_state`], and show that it is asking.
    pub fn elevate_stop(&self, id: u64) -> Result<(), Option<u64>> {
        let Ok(v) = self.raw("host.elevate_stop", json!({ "id": id })) else {
            return Err(None);
        };
        if v.get("stopped").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        Err(v.get("pending").and_then(Value::as_u64))
    }

    /// Whether a started elevation has finished, and how it ended.
    ///
    /// `None` while it is still going. Prefer [`Host::elevate_state`] when the
    /// difference between waiting for the prompt and running matters.
    pub fn elevate_status(&self, id: u64) -> Option<Elevated> {
        match self.elevate_state(id) {
            ElevateState::Done(e) => Some(e),
            _ => None,
        }
    }

    fn elevated_from(v: &Value) -> Elevated {
        Elevated {
            ran: v.get("ran").and_then(Value::as_bool).unwrap_or(false),
            code: v.get("code").and_then(Value::as_i64).map(|c| c as i32),
            reason: v.get("reason").and_then(Value::as_str).unwrap_or("error").to_string(),
            message: v.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
        }
    }

    fn elevate_call(&self, params: Value) -> Elevated {
        match self.raw("host.elevate", params) {
            Ok(v) => Elevated {
                ran: v.get("ran").and_then(Value::as_bool).unwrap_or(false),
                code: v.get("code").and_then(Value::as_i64).map(|c| c as i32),
                reason: v
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string(),
                message: v
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            },
            // The host refused, most likely because the permission is missing.
            Err(e) => Elevated {
                ran: false,
                code: None,
                reason: "error".into(),
                message: e.to_string(),
            },
        }
    }

    /// Whether this machine can ask the user for privileges at all, and how.
    ///
    /// Ask before offering an action that needs elevation: a machine with
    /// neither polkit nor a graphical sudo can do nothing, and the user should
    /// hear that up front rather than after the attempt.
    pub fn can_elevate(&self) -> (bool, String) {
        match self.raw("host.can_elevate", Value::Null) {
            Ok(v) => (
                v.get("available").and_then(Value::as_bool).unwrap_or(false),
                v.get("how").and_then(Value::as_str).unwrap_or("").to_string(),
            ),
            Err(_) => (false, String::new()),
        }
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
