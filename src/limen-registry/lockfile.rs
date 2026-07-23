//! `~/.limen/limen.lock` — the pinned, reproducible record of what's installed.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One installed module, pinned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    /// `git` or `path`.
    pub source: String,
    /// Repo (for git) or path (for local) — enough to re-fetch on update.
    pub reference: String,
    /// `sha256:<hex>` of the installed tree.
    pub digest: String,
}

/// The whole lockfile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lockfile {
    /// Lockfile schema version.
    #[serde(default)]
    pub version: u32,
    #[serde(default, rename = "module")]
    pub modules: Vec<LockEntry>,
}

impl Lockfile {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut doc = self.clone();
        doc.version = 1;
        doc.modules.sort_by(|a, b| a.name.cmp(&b.name));
        let text = toml::to_string_pretty(&doc)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Insert or replace the entry with the same name.
    pub fn upsert(&mut self, entry: LockEntry) {
        if let Some(existing) = self.modules.iter_mut().find(|m| m.name == entry.name) {
            *existing = entry;
        } else {
            self.modules.push(entry);
        }
    }

    /// Remove an entry by name; returns whether it was present.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.modules.len();
        self.modules.retain(|m| m.name != name);
        self.modules.len() != before
    }
}
