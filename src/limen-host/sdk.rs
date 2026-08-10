//! The script SDKs the host writes out for module runtimes.

use std::path::{PathBuf};
use anyhow::{Context, Result};
use limen_proto::{Language};

use crate::host::*;

pub(crate) const LUA_SDK: &str = include_str!("../../sdk/lua/limen.lua");

/// Paths to the extracted SDKs, and the env each language needs to find them.
pub(crate) struct SdkPaths {
    python: PathBuf,
    js: PathBuf,
    lua: PathBuf,
}

impl SdkPaths {
    /// The env vars to set when spawning a module of `language` so its runtime
    /// can find the injected SDK.
    pub(crate) fn env_for(&self, language: Language) -> Vec<(String, String)> {
        match language {
            Language::Python => vec![(
                "PYTHONPATH".to_string(),
                self.python.to_string_lossy().into_owned(),
            )],
            // Node searches each NODE_PATH entry like a node_modules dir, so
            // `require("limen")` resolves to <js>/limen.js.
            Language::Js => vec![(
                "NODE_PATH".to_string(),
                self.js.to_string_lossy().into_owned(),
            )],
            // Lua's require uses package.path patterns; `require("limen")` maps
            // to <lua>/limen.lua.
            Language::Lua => vec![(
                "LUA_PATH".to_string(),
                format!("{}/?.lua;;", self.lua.to_string_lossy()),
            )],
            Language::Native => Vec::new(),
        }
    }
}

/// Extract the embedded SDKs under `<limen home>/sdk/` and return their paths.
pub(crate) fn install_sdks() -> Result<SdkPaths> {
    let base = limen_home().join("sdk");
    let python = base.join("python");
    let js = base.join("js");
    let lua = base.join("lua");
    for dir in [&python, &js, &lua] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(python.join("limen_sdk.py"), PY_SDK).context("writing python SDK")?;
    std::fs::write(js.join("limen.js"), JS_SDK).context("writing js SDK")?;
    std::fs::write(lua.join("limen.lua"), LUA_SDK).context("writing lua SDK")?;
    Ok(SdkPaths { python, js, lua })
}

/// The Limen base dir: `$LIMEN_HOME`, else the executable's directory (portable).
/// Kept local so limen-host needn't depend on limen-core (must match
/// `limen_core::paths::home`).
pub(crate) fn limen_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("LIMEN_HOME") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}
