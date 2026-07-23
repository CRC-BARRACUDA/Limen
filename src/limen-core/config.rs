//! Engine settings, persisted to `~/.limen/settings.json`.
//!
//! Everything has a sensible default, so the file is optional. Only fields the
//! user changes need to be present.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Extra directories to search for modules, in addition to the default
    /// `~/.limen/modules`. Each entry may be a modules root (containing module
    /// subdirectories) or a single module directory.
    #[serde(default)]
    pub module_dirs: Vec<PathBuf>,
}

impl Config {
    /// Load settings, or defaults if the file is absent.
    pub fn load() -> Result<Self> {
        let path = paths::settings_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write settings back to `~/.limen/settings.json`.
    pub fn save(&self) -> Result<()> {
        let path = paths::settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// The directories to search for modules: the default modules dir first,
    /// then any configured extras.
    pub fn search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![paths::modules_dir()];
        dirs.extend(self.module_dirs.iter().cloned());
        dirs
    }
}
