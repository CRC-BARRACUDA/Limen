//! The Limen module runtime.
//!
//! The [`Host`] loads modules from their manifests, resolves the dependency
//! graph (topological order + semver checks), launches each module, and wires
//! them to a capability [`Broker`]. Every module reaches its dependencies the
//! same way regardless of language or transport: it sends a `host.call` naming a
//! *capability*, and the broker routes it to whichever module provides it.
//!
//! Two transports implement the [`Module`] trait, and are interchangeable to the
//! broker and everything above it:
//!
//! * [`ModuleConnection`] — a subprocess speaking JSON-RPC over stdio (any
//!   language: Python, Lua, JS, or a compiled binary).
//! * [`NativeModule`] — a dynamic library loaded in-process via the C ABI (the
//!   fast path for compiled Rust/C/Go modules).

mod broker;
mod connection;
mod host;
mod module;
mod native;

pub use broker::Broker;
pub use connection::ModuleConnection;
pub use host::{Host, Launch, ModuleSpec};
pub use module::{IncomingHandler, Module};
pub use native::NativeModule;
