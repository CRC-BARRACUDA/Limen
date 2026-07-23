//! Limen's on-disk locations, all under a single home directory.
//!
//! The home is `$LIMEN_HOME` if set (handy for tests and portable installs),
//! otherwise `~/.limen`. Nothing here creates directories — callers do that when
//! they actually write.

use std::path::PathBuf;

/// The Limen home directory (`$LIMEN_HOME`, else `~/.limen`).
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("LIMEN_HOME") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".limen")
}

/// Where installed modules live (`<home>/modules`).
pub fn modules_dir() -> PathBuf {
    home().join("modules")
}

/// Per-module and engine state (`<home>/state`).
pub fn state_dir() -> PathBuf {
    home().join("state")
}

/// The settings file (`<home>/settings.json`).
pub fn settings_path() -> PathBuf {
    home().join("settings.json")
}
