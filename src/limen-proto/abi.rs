//! The stable C ABI for **in-process native modules** (Phase 2).
//!
//! A native module is a dynamic library (`.dll` / `.so` / `.dylib`) the host
//! loads with `libloading` and calls directly — no subprocess, no serialization
//! across a pipe beyond the JSON payload itself. It exposes the same *semantics*
//! as an RPC module (`invoke` a method, get a result; call back into the host to
//! reach other modules), so everything above the transport is identical.
//!
//! ## Memory model
//!
//! To avoid the classic "who frees this buffer" hazard across the FFI boundary,
//! results are delivered through a **sink callback** rather than returned
//! buffers. The producer of a result invokes the sink exactly once with a
//! pointer to JSON bytes that are valid **only for the duration of the call** —
//! the receiver copies them inside the callback. No allocation crosses the
//! boundary, so nothing has to be freed across it.
//!
//! ## Symbols a module must export
//!
//! ```c
//! uint32_t limen_abi_version(void);
//! void*    limen_module_init(void* host_ctx, limen_host_call_fn host_call);
//! void     limen_module_call(void* handle,
//!                            const uint8_t* method, size_t method_len,
//!                            const uint8_t* params, size_t params_len,
//!                            limen_sink_fn sink, void* sink_ctx);
//! void     limen_module_shutdown(void* handle);
//! ```

use core::ffi::c_void;

/// The ABI revision the host and modules must agree on.
pub const ABI_VERSION: u32 = 1;

/// One-shot result callback. `is_error` is 0 for success, non-zero for an error;
/// `ptr`/`len` point to JSON bytes valid only until this call returns.
pub type SinkFn =
    unsafe extern "C" fn(sink_ctx: *mut c_void, is_error: i32, ptr: *const u8, len: usize);

/// The callback a module uses to call back into the host (e.g. `host.call` to
/// reach another module via the broker). Same shape as a module call: method +
/// params in, result out through the sink.
pub type HostCallFn = unsafe extern "C" fn(
    host_ctx: *mut c_void,
    method_ptr: *const u8,
    method_len: usize,
    params_ptr: *const u8,
    params_len: usize,
    sink: SinkFn,
    sink_ctx: *mut c_void,
);

// ---- signatures of the symbols the host looks up in the library ------------ //

pub type AbiVersionFn = unsafe extern "C" fn() -> u32;

pub type ModuleInitFn =
    unsafe extern "C" fn(host_ctx: *mut c_void, host_call: HostCallFn) -> *mut c_void;

pub type ModuleCallFn = unsafe extern "C" fn(
    handle: *mut c_void,
    method_ptr: *const u8,
    method_len: usize,
    params_ptr: *const u8,
    params_len: usize,
    sink: SinkFn,
    sink_ctx: *mut c_void,
);

pub type ModuleShutdownFn = unsafe extern "C" fn(handle: *mut c_void);

// ---- exported symbol names ------------------------------------------------- //

pub const SYM_ABI_VERSION: &[u8] = b"limen_abi_version";
pub const SYM_INIT: &[u8] = b"limen_module_init";
pub const SYM_CALL: &[u8] = b"limen_module_call";
pub const SYM_SHUTDOWN: &[u8] = b"limen_module_shutdown";
