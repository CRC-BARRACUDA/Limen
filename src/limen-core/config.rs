//! Engine settings, persisted to `~/.limen/settings.json`.
//!
//! Everything has a sensible default, so the file is optional. Only fields the
//! user changes need to be present.
//!
//! The file says which version of Limen last wrote it (`version`). When that
//! is not this version — Limen was updated, or the file predates the field —
//! [`Config::load`] fills in every field the file lacks with its default and
//! writes the file back once, so the user can see (and edit) what the new
//! version reads. Values already there are kept as they are, and so are keys
//! this version does not know.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::paths;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The version of Limen that last wrote this file. Set on every save.
    #[serde(default)]
    pub version: Option<String>,

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

    /// The platform's API root to sign in to (`https://auth…/api`). Absent =
    /// this build's [environment](crate::environment) — local for a debug
    /// build, production for a release one. `LIMEN_PLATFORM_URL` wins.
    #[serde(default)]
    pub platform_url: Option<String>,

    /// Limen's public OAuth2 client id on the platform. Absent = the one built
    /// in for that platform (production has one); with none, signing in is
    /// off. `LIMEN_PLATFORM_CLIENT_ID` wins.
    #[serde(default)]
    pub platform_client_id: Option<String>,

    /// Who is signed in, as the platform last said — shown at once on start,
    /// before the session is checked. Not a secret: the refresh token is in the
    /// OS keychain, never here.
    #[serde(default)]
    pub account: Option<crate::account::Account>,

    /// The Avatar Decoration service's API. Absent =
    /// this build's [environment](crate::environment). `LIMEN_AVATAR_URL` wins.
    #[serde(default)]
    pub avatar_url: Option<String>,

    /// The files host avatars and frames are served from. Absent =
    /// this build's [environment](crate::environment). `LIMEN_AVATAR_ASSET_BASE` wins.
    #[serde(default)]
    pub avatar_asset_base: Option<String>,

    /// Lock the vault after this many minutes without use. Absent = 15;
    /// 0 = never.
    #[serde(default)]
    pub vault_lock_minutes: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: None,
            module_dirs: Vec::new(),
            default_org: None,
            ui_scale_percent: 0,
            animations: true,
            alerts: true,
            language: None,
            favorites: Vec::new(),
            categories: BTreeMap::new(),
            github_token: None,
            platform_url: None,
            platform_client_id: None,
            account: None,
            avatar_url: None,
            avatar_asset_base: None,
            vault_lock_minutes: None,
        }
    }
}

/// The org used when none is configured.
pub const DEFAULT_ORG: &str = "CRC-BARRACUDA";

/// This build's version, as the settings file records it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// When `raw` was written by another version, add each field it lacks, with
/// the value `cfg` (what it loaded as) has for it — its default — and stamp it
/// with this version. Present keys, known or not, are left alone. Whether
/// anything changed.
fn upgrade(raw: &mut Value, cfg: &Config) -> Result<bool> {
    let Some(file) = raw.as_object_mut() else {
        return Ok(false);
    };
    if file.get("version").and_then(Value::as_str) == Some(VERSION) {
        return Ok(false);
    }
    if let Value::Object(full) = serde_json::to_value(cfg)? {
        for (key, value) in full {
            file.entry(key).or_insert(value);
        }
    }
    file.insert("version".into(), Value::from(VERSION));
    Ok(true)
}

fn write(raw: &Value) -> Result<()> {
    let path = paths::settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(raw)?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

impl Config {
    /// Load settings, or defaults if the file is absent. The first load after
    /// an update fills in the fields the file lacks and writes it back.
    pub fn load() -> Result<Self> {
        let path = paths::settings_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut raw: Value =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let cfg: Self =
            serde_json::from_value(raw.clone()).with_context(|| format!("parsing {}", path.display()))?;
        if upgrade(&mut raw, &cfg)? {
            // Settings that load but cannot be written back are still settings.
            let _ = write(&raw);
        }
        Ok(cfg)
    }

    /// Write settings back to `~/.limen/settings.json`, stamped with this
    /// version.
    pub fn save(&self) -> Result<()> {
        let mut raw = serde_json::to_value(self)?;
        raw["version"] = Value::from(VERSION);
        write(&raw)
    }

    /// The directories to search for modules: the default modules dir first,
    /// then any configured extras.
    pub fn search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![paths::modules_dir()];
        dirs.extend(self.module_dirs.iter().cloned());
        dirs
    }
}
