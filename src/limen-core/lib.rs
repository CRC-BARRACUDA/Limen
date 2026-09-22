//! The Limen engine.
//!
//! [`Engine`] sits on top of [`limen_host::Host`] and gives the frontends (CLI,
//! GUI) a small, stable surface: discover installed modules, start them, list
//! their capabilities, describe them, and run a capability method. It also owns
//! Limen's on-disk conventions — the `~/.limen` home ([`paths`]) and settings
//! ([`Config`]).
//!
//! Like v1, there is **no required config file**: defaults work out of the box,
//! and `~/.limen/settings.json` only holds what the user changes.

pub mod config;
pub mod engine;
pub mod paths;
pub mod setup;
pub mod update;

pub use config::Config;
pub use engine::Engine;
pub use limen_host::{runtimes::Runtime, Logger, ModuleSpec};
// Re-exported because *every* Limen binary must be able to be the elevated
// supervisor: it is elevated by path, and the only path safe to hand to `pkexec`
// is the running executable's own (see `limen_host::supervisor_bin`).
pub use limen_host::supervise;
pub use setup::{can_install, install_runtime, runtime_report, RuntimeInfo, RuntimeState};
pub use update::{
    apply_update, check_update, is_newer, notify, register_notifier, restart_app, set_update_dir,
    UpdateInfo,
};
