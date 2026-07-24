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
    /// Git branch/ref the module was installed from (empty for path installs).
    #[serde(default)]
    pub branch: String,
    /// Git short commit the module was installed from (empty for path installs).
    #[serde(default)]
    pub commit: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_lockfile_without_git_fields_still_parses() {
        // Lockfiles written before branch/commit existed must load (serde default),
        // so upgrading Limen never breaks an existing install.
        let text = r#"
version = 1
[[module]]
name = "m"
version = "0.1.0"
source = "git"
reference = "owner/repo"
digest = "sha256:ab"
"#;
        let lock: Lockfile = toml::from_str(text).unwrap();
        assert_eq!(lock.modules.len(), 1);
        assert_eq!(lock.modules[0].branch, "");
        assert_eq!(lock.modules[0].commit, "");
    }

    #[test]
    fn git_fields_round_trip_through_toml() {
        let mut lock = Lockfile::default();
        lock.upsert(LockEntry {
            name: "m".into(),
            version: "0.2.0".into(),
            source: "git".into(),
            reference: "owner/repo".into(),
            digest: "sha256:xy".into(),
            branch: "main".into(),
            commit: "abc1234".into(),
        });
        let text = toml::to_string_pretty(&lock).unwrap();
        let back: Lockfile = toml::from_str(&text).unwrap();
        assert_eq!(back.modules[0].branch, "main");
        assert_eq!(back.modules[0].commit, "abc1234");
    }

    #[test]
    fn upsert_replaces_and_remove_deletes() {
        let mut lock = Lockfile::default();
        let mk = |v: &str| LockEntry {
            name: "m".into(),
            version: v.into(),
            source: "git".into(),
            reference: "o/r".into(),
            digest: "sha256:0".into(),
            branch: String::new(),
            commit: String::new(),
        };
        lock.upsert(mk("0.1.0"));
        lock.upsert(mk("0.2.0")); // same name → replace, not duplicate
        assert_eq!(lock.modules.len(), 1);
        assert_eq!(lock.modules[0].version, "0.2.0");
        assert!(lock.remove("m"));
        assert!(!lock.remove("m"));
        assert!(lock.modules.is_empty());
    }
}
