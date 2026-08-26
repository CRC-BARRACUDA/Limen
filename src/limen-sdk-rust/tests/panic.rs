//! A module that panics must not take the host down with it.
//!
//! The generated entry points are `extern "C"`. A panic cannot unwind out of
//! one — the runtime aborts instead, and the host is a GUI holding every other
//! module and the analyst's unsaved work. These tests fail by killing the test
//! process rather than by printing a diff.

use core::ffi::c_void;

use limen_sdk_rust::{json, Handler, Host, RpcError, SinkFn, Value};

#[derive(Default)]
struct Fragile;

impl Handler for Fragile {
    fn capabilities(&self) -> Vec<String> {
        vec!["fragile".to_string()]
    }

    // The unwrap below is the point of the test, not an oversight.
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn invoke(&mut self, _cap: &str, method: &str, _p: Value, _h: &Host) -> Result<Value, RpcError> {
        match method {
            "boom" => panic!("tripped over its own state"),
            "unwrap" => {
                let nothing: Option<u8> = None;
                Ok(json!(nothing.unwrap()))
            }
            _ => Ok(json!("fine")),
        }
    }
}

/// A module whose `Default` cannot run — the panic lands before there is a
/// handle for the host to hold.
struct Stillborn;

impl Default for Stillborn {
    fn default() -> Self {
        panic!("nothing to build on")
    }
}

impl Handler for Stillborn {
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }
    fn invoke(&mut self, _c: &str, _m: &str, _p: Value, _h: &Host) -> Result<Value, RpcError> {
        Ok(Value::Null)
    }
}

/// One that panics on the way out, when the host is already tearing it down.
#[derive(Default)]
struct Grumpy;

impl Drop for Grumpy {
    fn drop(&mut self) {
        panic!("not going quietly");
    }
}

impl Handler for Grumpy {
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }
    fn invoke(&mut self, _c: &str, _m: &str, _p: Value, _h: &Host) -> Result<Value, RpcError> {
        Ok(Value::Null)
    }
}

#[derive(Default)]
struct Captured {
    is_error: i32,
    body: Vec<u8>,
}

unsafe extern "C" fn sink(ctx: *mut c_void, is_error: i32, ptr: *const u8, len: usize) {
    let out = unsafe { &mut *(ctx as *mut Captured) };
    out.is_error = is_error;
    out.body = if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    };
}

unsafe extern "C" fn host_call(
    _ctx: *mut c_void,
    _m: *const u8,
    _ml: usize,
    _p: *const u8,
    _pl: usize,
    sink: SinkFn,
    sink_ctx: *mut c_void,
) {
    let out = b"null";
    unsafe { sink(sink_ctx, 0, out.as_ptr(), out.len()) };
}

/// Drive one call the way the host does, and read what came back.
fn call(handle: *mut c_void, method: &str, params: &str) -> (i32, Value) {
    let mut got = Captured::default();
    unsafe {
        limen_sdk_rust::__rt::call::<Fragile>(
            handle,
            method.as_ptr(),
            method.len(),
            params.as_ptr(),
            params.len(),
            sink,
            &mut got as *mut Captured as *mut c_void,
        );
    }
    let v = serde_json::from_slice(&got.body).unwrap_or(Value::Null);
    (got.is_error, v)
}

/// Quiet, so a deliberate panic does not read as a failing test run.
fn without_panic_output<R>(f: impl FnOnce() -> R) -> R {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(previous);
    out
}

#[test]
fn a_panicking_method_answers_with_an_error() {
    let handle = unsafe { limen_sdk_rust::__rt::init::<Fragile>(std::ptr::null_mut(), host_call) };
    assert!(!handle.is_null());

    let (is_error, v) = without_panic_output(|| {
        call(handle, "invoke", r#"{"capability":"fragile","method":"boom"}"#)
    });

    assert_eq!(is_error, 1, "the host must be told this failed");
    assert_eq!(v["code"], json!(limen_sdk_rust::rpc::MODULE_PANIC));
    let message = v["message"].as_str().unwrap_or_default();
    assert!(message.contains("panicked"), "{message}");
    assert!(message.contains("tripped over its own state"), "{message}");
    assert!(message.contains("invoke"), "which call it was: {message}");

    unsafe { limen_sdk_rust::__rt::shutdown::<Fragile>(handle) };
}

/// An `unwrap()` on a `None` carries no message of its own, and it is the way
/// this actually happens in the field.
#[test]
fn an_unwrap_is_caught_too() {
    let handle = unsafe { limen_sdk_rust::__rt::init::<Fragile>(std::ptr::null_mut(), host_call) };
    let (is_error, v) = without_panic_output(|| {
        call(handle, "invoke", r#"{"capability":"fragile","method":"unwrap"}"#)
    });
    assert_eq!(is_error, 1);
    assert!(v["message"].as_str().unwrap_or_default().contains("panicked"), "{v}");
    unsafe { limen_sdk_rust::__rt::shutdown::<Fragile>(handle) };
}

/// The panic poisons the mutex it was holding. Unwrapping that poison would
/// abort on the *next* call — the crash simply moved one step later.
#[test]
fn the_module_still_answers_after_a_panic() {
    let handle = unsafe { limen_sdk_rust::__rt::init::<Fragile>(std::ptr::null_mut(), host_call) };

    without_panic_output(|| call(handle, "invoke", r#"{"capability":"fragile","method":"boom"}"#));

    let (is_error, v) = call(handle, "invoke", r#"{"capability":"fragile","method":"ok"}"#);
    assert_eq!(is_error, 0, "a poisoned lock must not end the module: {v}");
    assert_eq!(v, json!("fine"));

    let (is_error, v) = call(handle, "initialize", "{}");
    assert_eq!(is_error, 0);
    assert_eq!(v["capabilities"], json!(["fragile"]));

    unsafe { limen_sdk_rust::__rt::shutdown::<Fragile>(handle) };
}

#[test]
fn an_ordinary_call_is_untouched() {
    let handle = unsafe { limen_sdk_rust::__rt::init::<Fragile>(std::ptr::null_mut(), host_call) };
    assert_eq!(call(handle, "describe", "{}").0, 0);
    assert_eq!(
        call(handle, "nosuch", "{}").1["code"],
        json!(-32601),
        "a method that does not exist is still not-found, not a panic"
    );
    unsafe { limen_sdk_rust::__rt::shutdown::<Fragile>(handle) };
}

/// `limen_module_init` returning null is how the host already reads "this one
/// failed to load" (native.rs bails on it), so a panic in `Default` becomes
/// a load failure rather than an abort.
#[test]
fn a_module_that_cannot_be_built_returns_null() {
    let handle =
        without_panic_output(|| unsafe {
            limen_sdk_rust::__rt::init::<Stillborn>(std::ptr::null_mut(), host_call)
        });
    assert!(handle.is_null());
}

#[test]
fn a_panic_on_the_way_out_is_survived() {
    let handle = unsafe { limen_sdk_rust::__rt::init::<Grumpy>(std::ptr::null_mut(), host_call) };
    without_panic_output(|| unsafe { limen_sdk_rust::__rt::shutdown::<Grumpy>(handle) });
    // Reaching this line is the assertion.
}
