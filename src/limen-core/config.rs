//! Engine settings, persisted to `~/.limen/settings.json`.
//!
//! Everything has a sensible default, so the file is optional. Only fields the
//! user changes need to be present.

use std::collections::BTreeMap;
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

    /// Global UI scale, as a percentage (100 = default). 0/absent means default.
    #[serde(default)]
    pub ui_scale_percent: u32,

    /// Whether UI animations are enabled. Defaults to on.
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Whether passing notices are shown in the corner. Defaults to on.
    #[serde(default = "default_true")]
    pub alerts: bool,

    /// UI language code (e.g. `"en"`, `"uk"`). Absent = detect from the OS locale,
    /// falling back to English.
    #[serde(default)]
    pub language: Option<String>,

    /// Modules the user marked as favourites, by name.
    ///
    /// Kept as a list rather than a flag on the module because a module is a
    /// directory on disk that installs, updates and is removed — it is not the
    /// place for a preference *about* it. A name that no longer resolves to an
    /// installed module is simply not shown; it is left in the file so that
    /// removing a module and putting it back does not silently lose the mark.
    #[serde(default)]
    pub favorites: Vec<String>,

    /// User-made categories: category name -> the modules in it.
    ///
    /// The grouping lives here rather than in each module's manifest because it
    /// is the operator's, not the author's: two people running the same fleet
    /// will group the same modules differently, and a category has to be able to
    /// hold modules from repositories that have never heard of each other.
    ///
    /// A `BTreeMap` so the file has a stable order and stops churning between
    /// saves, and so the menus list categories the same way every time.
    #[serde(default)]
    pub categories: BTreeMap<String, Vec<String>>,

    /// A GitHub token (personal access token) for the module registry, set by an
    /// administrator in Developer mode. When present, registry requests are
    /// authenticated — raising the rate limit from 60/hour (unauthenticated, per
    /// IP) to 5,000/hour and enabling free conditional (304) requests. Absent =
    /// unauthenticated (the default). Only a read/public-repo scope is needed.
    #[serde(default)]
    pub github_token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            module_dirs: Vec::new(),
            default_org: None,
            ui_scale_percent: 0,
            animations: true,
            alerts: true,
            language: None,
            favorites: Vec::new(),
            categories: BTreeMap::new(),
            github_token: None,
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
