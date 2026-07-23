//! The Limen module contract.
//!
//! Every module — whether a Python script over stdio or a native `.dll` loaded
//! in-process — speaks the same protocol defined here. This crate is the single
//! source of truth for:
//!
//! * [`rpc`] — the JSON-RPC 2.0 message types exchanged over the transport, and
//! * [`manifest`] — the `limen.toml` schema that declares a module's identity,
//!   the capabilities it *provides*, and the capabilities it *requires* from
//!   other modules.
//!
//! Keeping this crate dependency-light means module authors (and future
//! language SDKs) can depend on it without pulling in the host runtime.

pub mod abi;
pub mod manifest;
pub mod rpc;

pub use abi::ABI_VERSION;
pub use manifest::{
    Abi, DepSpec, DetailedDep, Language, Manifest, ModuleMeta, Permissions, Provides, Requires,
};
pub use rpc::{Message, Request, Response, RpcError};
