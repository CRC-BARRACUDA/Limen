//! The transport-agnostic module abstraction.
//!
//! A module might be a subprocess speaking JSON-RPC over stdio
//! ([`crate::ModuleConnection`]) or a dynamic library loaded in-process
//! ([`crate::NativeModule`]). The broker and host only ever see them as
//! `Arc<dyn Module>`, so adding a transport changes nothing above this line.

use std::sync::Arc;

use serde_json::Value;

use limen_proto::RpcError;

/// Handles requests a module makes *back to the host* (`host.call` broker
/// routing, `host.log`). Shared across every transport, so it is `Send + Sync`.
pub type IncomingHandler =
    dyn Fn(&str, Value) -> std::result::Result<Value, RpcError> + Send + Sync;

/// A sink for host + module log lines. Defaults to stderr; the GUI installs one
/// that forwards to its debug console.
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// The default logger: write to stderr.
pub fn stderr_logger() -> Logger {
    Arc::new(|line: &str| eprintln!("{line}"))
}

/// A live module. `call` issues a request and blocks for the result; it must be
/// safe to call concurrently from multiple threads.
pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn call(&self, method: &str, params: Value) -> std::result::Result<Value, RpcError>;
    /// Fire-and-forget: send `method`/`params` to the module without waiting for
    /// a response. Used to push events to subscribers.
    fn notify(&self, method: &str, params: Value);
    fn shutdown(&self);
}
