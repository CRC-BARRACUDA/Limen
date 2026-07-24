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
fn collect_module_dirs(search_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("limen-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_module(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("limen.toml"),
            format!(
                "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\nlanguage = \"python\"\nentry = \"main.py\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn discovers_module_subdirs_and_skips_missing_roots() {
        let root = tmpdir("root");
        write_module(&root, "alpha");
        write_module(&root, "beta");

        let dirs = collect_module_dirs(&[root.clone(), PathBuf::from("/no/such/dir")]).unwrap();
        assert_eq!(dirs.len(), 2);
        assert!(dirs.iter().all(|d| d.join("limen.toml").is_file()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn treats_a_root_that_is_itself_a_module_as_one_module() {
        let root = tmpdir("single");
        // The root itself holds the manifest.
        std::fs::write(
            root.join("limen.toml"),
            "[module]\nname = \"solo\"\nversion = \"0.1.0\"\nlanguage = \"lua\"\nentry = \"m.lua\"\n",
        )
        .unwrap();

        let dirs = collect_module_dirs(std::slice::from_ref(&root)).unwrap();
        assert_eq!(dirs, vec![root.clone()]);

        std::fs::remove_dir_all(&root).ok();
    }
}
