//! The [`Engine`]: discover modules, start them, and run/inspect capabilities.

use std::path::PathBuf;

use anyhow::{Context, Result};
use limen_host::{Host, Logger, ModuleSpec};
use serde_json::Value;

pub struct Engine {
    host: Host,
    started: bool,
}

impl Engine {
    /// Discover and load every module found under `search_dirs`. Each entry may
    /// be a modules root (with module subdirectories) or a single module dir.
    /// Does not launch anything — call [`Engine::start`].
    pub fn load(search_dirs: &[PathBuf]) -> Result<Self> {
        let module_dirs = collect_module_dirs(search_dirs)?;
        // Zero modules is fine — a fresh or portable install may have none yet.
        // The app starts empty rather than failing.
        let host = Host::load(&module_dirs)?;
        Ok(Self { host, started: false })
    }

    /// Install a log sink for host + module logs (call before [`Engine::start`]).
    pub fn set_logger(&mut self, logger: Logger) {
        self.host.set_logger(logger);
    }

    /// Launch all modules (idempotent).
    pub fn start(&mut self) -> Result<()> {
        if !self.started {
            self.host.start().context("starting modules")?;
            self.started = true;
        }
        Ok(())
    }

    /// Installed modules, in dependency (startup) order.
    pub fn modules(&self) -> &[ModuleSpec] {
        self.host.module_specs()
    }

    /// Runtimes a module needed but that were unavailable at start (module
    /// skipped) — drives Quick Setup.
    pub fn missing_runtimes(&self) -> &[limen_host::runtimes::Runtime] {
        self.host.missing_runtimes()
    }

    /// Runtimes needed by loaded scripted modules that aren't bundled yet.
    /// Installing them makes the app portable (no system-interpreter dependency).
    pub fn unbundled_runtimes(&self) -> Vec<limen_host::runtimes::Runtime> {
        self.host.unbundled_runtimes()
    }

    /// Modules that failed to start (name -> error). Isolated failures — the rest
    /// of the engine still runs; the GUI shows the error in the module's tab.
    pub fn failed_modules(&self) -> &std::collections::HashMap<String, String> {
        self.host.failed_modules()
    }

    /// The self-description of the module providing `capability`.
    pub fn describe(&self, capability: &str) -> Result<Value> {
        self.host.describe(capability)
    }

    /// Invoke `method` on whichever module provides `capability`.
    pub fn run(&self, capability: &str, method: &str, params: Value) -> Result<Value> {
        self.host.invoke(capability, method, params)
    }

    /// Stop all modules.
    pub fn shutdown(&mut self) {
        self.host.shutdown();
    }
}

/// Expand search roots into concrete module directories (those with a
/// `limen.toml`). A root that is itself a module is taken as-is; otherwise its
/// immediate subdirectories are scanned. Missing roots are skipped.
#[doc(hidden)]
pub fn collect_module_dirs(search_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for root in search_dirs {
        if !root.exists() {
            continue;
        }
        if root.join("limen.toml").is_file() {
            found.push(root.clone());
            continue;
        }
        let entries =
            std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
        for entry in entries {
            let path = entry?.path();
            if path.join("limen.toml").is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}
