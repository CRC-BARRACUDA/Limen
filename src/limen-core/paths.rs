//! Limen's on-disk locations — **portable by default**.
//!
//! Limen is designed to run self-contained from a USB stick: everything (modules,
//! settings, the extracted SDKs, the lockfile) lives in one *base* directory next
//! to the executable, so nothing is written to the user's home. `$LIMEN_HOME`
//! overrides the base (used by tests and for a fixed install location).
//!
//! Nothing here creates directories — callers do that when they actually write.

use std::path::PathBuf;

/// The Limen base directory: `$LIMEN_HOME` if set, otherwise the directory the
/// executable lives in (portable). Falls back to the current directory only if
/// the executable path can't be determined.
pub fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("LIMEN_HOME") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
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

/// The app icon extracted to disk (`<home>/state/icon.png`).
///
/// OS notifiers take an image *path*, but the icon is compiled into the binary —
/// so it is written out here on demand. Derived, not user data: safe to delete.
pub fn icon_path() -> PathBuf {
    state_dir().join("icon.png")
}

/// Create the standard portable directories (currently the modules dir) so a
/// fresh install has a place to drop modules. Best-effort; ignores errors.
pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(modules_dir());
}
