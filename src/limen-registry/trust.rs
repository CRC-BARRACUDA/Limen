//! The trust store — `~/.limen/trust.json`.
//!
//! Downloading and running third-party modules that can execute scripts across a
//! fleet is a real supply-chain surface. The trust store records an explicit
//! operator approval per module, **pinned to a content digest**: if the module's
//! files later change, its recorded approval no longer matches and it counts as
//! untrusted again (trust-on-first-use, revoked on change).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    /// module name -> approved `sha256:` digest.
    #[serde(default)]
    approvals: BTreeMap<String, String>,
}

impl TrustStore {
    fn path(home: &Path) -> std::path::PathBuf {
        home.join("trust.json")
    }

    pub fn load(home: &Path) -> Result<Self> {
        let path = Self::path(home);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, home: &Path) -> Result<()> {
        let path = Self::path(home);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// True if `name` is approved at exactly `digest`.
    pub fn is_trusted(&self, name: &str, digest: &str) -> bool {
        self.approvals.get(name).map(String::as_str) == Some(digest)
    }

    /// Approve `name` at `digest` (replacing any prior approval).
    pub fn approve(&mut self, name: &str, digest: &str) {
        self.approvals.insert(name.to_string(), digest.to_string());
    }

    /// Revoke approval; returns whether it existed.
    pub fn revoke(&mut self, name: &str) -> bool {
        self.approvals.remove(name).is_some()
    }

    pub fn approvals(&self) -> &BTreeMap<String, String> {
        &self.approvals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_is_pinned_to_digest() {
        let mut trust = TrustStore::default();
        assert!(!trust.is_trusted("crowdstrike", "sha256:aaa"));

        trust.approve("crowdstrike", "sha256:aaa");
        assert!(trust.is_trusted("crowdstrike", "sha256:aaa"));
        // A changed digest invalidates the approval (trust revoked on change).
        assert!(!trust.is_trusted("crowdstrike", "sha256:bbb"));

        assert!(trust.revoke("crowdstrike"));
        assert!(!trust.is_trusted("crowdstrike", "sha256:aaa"));
        assert!(!trust.revoke("crowdstrike"));
    }
}
