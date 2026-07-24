//! Self-update: check the app's GitHub releases and (best-effort) apply.
//!
//! The check compares the running version against the latest release tag. Apply
//! downloads the platform binary asset, swaps it in over the running executable,
//! and restarts. Uses `curl`; no extra Rust dependencies.
//!
//! NOTE: the apply/replace/restart path is platform-sensitive and hard to verify
//! automatically — treat it as best-effort until tested on real releases.

use std::process::Command;

/// The GitHub repo the app updates from.
pub const APP_REPO: &str = "CRC-BARRACUDA/Limen";

/// An available update.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// The release page (for "view details").
    pub url: String,
    /// Release notes (may be empty).
    pub notes: String,
    /// Direct download URL of the binary for this platform, if the release has one.
    pub asset_url: Option<String>,
}

/// Parse `1.2.3` / `v1.2.3` into a comparable `(major, minor, patch)`.
fn version_tuple(s: &str) -> (u32, u32, u32) {
    let mut it = s.trim().trim_start_matches('v').split('.').map(|p| {
        p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
    });
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Whether `latest` is a newer version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    version_tuple(latest) > version_tuple(current)
}

/// The expected substrings in a release asset name for this platform's binary.
fn platform_needle() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH) // e.g. ("linux", "x86_64")
}

/// Check GitHub for a newer release than `current` (e.g. `env!("CARGO_PKG_VERSION")`).
/// Returns `None` if up to date, offline, or there's no release.
pub fn check_update(current: &str) -> Option<UpdateInfo> {
    let url = format!("https://api.github.com/repos/{APP_REPO}/releases/latest");
    let out = Command::new("curl")
        .args([
            "-sSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    if !is_newer(tag, current) {
        return None;
    }

    // Find a binary asset matching this OS + arch.
    let (os, arch) = platform_needle();
    let asset_url = json
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|a| {
                let name = a.get("name")?.as_str()?.to_lowercase();
                (name.contains(os) && name.contains(arch))
                    .then(|| a.get("browser_download_url")?.as_str().map(str::to_string))
                    .flatten()
            })
        });

    Some(UpdateInfo {
        current: current.to_string(),
        latest: tag.trim_start_matches('v').to_string(),
        url: json.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        notes: json.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        asset_url,
    })
}

/// Download the new binary, swap it in over the running executable, and restart.
/// Best-effort and platform-sensitive; returns an error string on failure.
pub fn apply_update(info: &UpdateInfo) -> Result<(), String> {
    let asset = info
        .asset_url
        .as_deref()
        .ok_or("this release has no binary for your platform — download it manually")?;
    let exe = std::env::current_exe().map_err(|e| format!("locating executable: {e}"))?;

    // Download next to the current executable.
    let staged = exe.with_extension("update-download");
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&staged)
        .arg(asset)
        .status()
        .map_err(|e| format!("running curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&staged);
        return Err("download failed".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }

    // Swap the new binary in. On Unix the running process keeps the old inode, so
    // renaming over it is safe. On Windows the running .exe can't be replaced
    // directly, so move it aside first.
    #[cfg(windows)]
    {
        let backup = exe.with_extension("old");
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(&exe, &backup).map_err(|e| format!("backing up current exe: {e}"))?;
    }
    std::fs::rename(&staged, &exe).map_err(|e| format!("installing update: {e}"))?;

    // Restart into the new binary.
    let args: Vec<String> = std::env::args().skip(1).collect();
    Command::new(&exe)
        .args(&args)
        .spawn()
        .map_err(|e| format!("restarting: {e}"))?;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
    }
}
