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

mod host;
mod rt;

// `export_module!` is #[macro_export], so it lands at the crate root wherever
// it is written; the rest is re-exported so `limen_sdk_rust::Host` still means
// what it always meant.
pub use host::*;
#[doc(hidden)]
pub use rt::*;
