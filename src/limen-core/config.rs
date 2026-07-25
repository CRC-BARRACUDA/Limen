//! Engine settings, persisted to `~/.limen/settings.json`.
//!
//! Everything has a sensible default, so the file is optional. Only fields the
//! user changes need to be present.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Extra directories to search for modules, in addition to the default
    /// `~/.limen/modules`. Each entry may be a modules root (containing module
    /// subdirectories) or a single module directory.
    #[serde(default)]
    pub module_dirs: Vec<PathBuf>,

    /// GitHub organization to browse/install modules from.
    #[serde(default)]
    pub default_org: Option<String>,

    /// Module names pinned to the sidebar, in display order.
    #[serde(default)]
    pub pinned_modules: Vec<String>,

    /// Global UI scale, as a percentage (100 = default). 0/absent means default.
    #[serde(default)]
    pub ui_scale_percent: u32,

    /// Whether UI animations are enabled. Defaults to on.
    #[serde(default = "default_true")]
    pub animations: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            module_dirs: Vec::new(),
            default_org: None,
            pinned_modules: Vec::new(),
            ui_scale_percent: 0,
            animations: true,
        }
    }
}

/// The org used when none is configured.
pub const DEFAULT_ORG: &str = "CRC-BARRACUDA";

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
