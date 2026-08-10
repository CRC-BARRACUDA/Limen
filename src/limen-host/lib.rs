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
mod elevate;
mod files;
mod notify;
mod sdk;
mod spec;
mod supervisor;
mod module;
mod native;
pub mod runtimes;

pub use broker::Broker;
pub use connection::ModuleConnection;
pub use host::Host;
pub use spec::{Launch, ModuleSpec};

// Reachable so the supervisor's own test can start one and watch it die with
// its parent — which is the whole behaviour, and cannot be observed from
// outside. Hidden from the docs: these are internals the test is allowed to
// see, not an offer.
#[doc(hidden)]
pub use supervisor::{sup_accept, sup_cleanup, supervised};
pub use module::{stderr_logger, IncomingHandler, Logger, Module};
pub use native::NativeModule;
pub use runtimes::{Runtime, RuntimeStatus};
