//! Self-update: check the app's GitHub releases and (best-effort) apply.
//!
//! The check compares the running version against the latest release tag. Apply
//! downloads the platform asset, and — if it's a `.tar.gz`/`.zip` distribution
//! archive — extracts the executable out of it, then swaps it in over the running
//! executable and restarts. A raw binary asset (no extension) is used directly.
//! Uses `curl` + `tar`/`unzip`; no extra Rust dependencies.
//!
//! NOTE: the apply/replace/restart path is platform-sensitive and hard to verify
//! automatically — treat it as best-effort until tested on real releases.

use std::path::{Path, PathBuf};
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

/// Whether an asset name is a distribution archive we can extract a binary from.
fn is_extractable_archive(name: &str) -> bool {
    const EXTS: [&str; 4] = [".tar.gz", ".tgz", ".tar", ".zip"];
    EXTS.iter().any(|e| name.ends_with(e))
}

/// Whether an asset name is a checksum/metadata file (never an update payload).
fn is_sidecar(name: &str) -> bool {
    const EXTS: [&str; 3] = [".sha256", ".txt", ".sig"];
    EXTS.iter().any(|e| name.ends_with(e))
}

/// From `(lowercased_name, url)` release assets, pick the one for this platform.
/// Prefer a raw binary (used directly); else a distribution archive (extracted).
/// Checksums / signatures are never chosen. `None` if nothing matches.
fn select_asset(assets: &[(String, String)]) -> Option<String> {
    let (os, arch) = platform_needle();
    let matches_platform = |n: &str| n.contains(os) && n.contains(arch);
    assets
        .iter()
        .find(|(n, _)| matches_platform(n) && !is_extractable_archive(n) && !is_sidecar(n))
        .or_else(|| assets.iter().find(|(n, _)| matches_platform(n) && is_extractable_archive(n)))
        .map(|(_, url)| url.clone())
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

    // Pick the asset for this OS + arch (see `select_asset`).
    let assets: Vec<(String, String)> = json
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some((
                        a.get("name")?.as_str()?.to_lowercase(),
                        a.get("browser_download_url")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let asset_url = select_asset(&assets);

    Some(UpdateInfo {
        current: current.to_string(),
        latest: tag.trim_start_matches('v').to_string(),
        url: json.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        notes: json.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        asset_url,
    })
}

/// Download the new build, swap it in over the running executable, and restart.
/// Best-effort and platform-sensitive; returns an error string on failure.
pub fn apply_update(info: &UpdateInfo) -> Result<(), String> {
    let asset = info
        .asset_url
        .as_deref()
        .ok_or("this release has no build for your platform — download it manually")?;
    let exe = std::env::current_exe().map_err(|e| format!("locating executable: {e}"))?;
    let exe_name = exe
        .file_name()
        .ok_or("executable has no file name")?
        .to_owned();

    // Download next to the current executable.
    let staged = exe.with_extension("update-download");
    let _ = std::fs::remove_file(&staged);
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

    // If the asset is a distribution archive, extract the executable out of it;
    // otherwise the download *is* the raw binary.
    let name = asset.rsplit('/').next().unwrap_or("").to_lowercase();
    let new_binary = if is_extractable_archive(&name) {
        let extracted = extract_binary(&staged, &exe, &exe_name, &name);
        let _ = std::fs::remove_file(&staged); // archive no longer needed
        extracted?
    } else {
        staged.clone()
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_binary, std::fs::Permissions::from_mode(0o755));
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
    std::fs::rename(&new_binary, &exe).map_err(|e| format!("installing update: {e}"))?;
    // Best-effort cleanup of the extraction scratch dir.
    let _ = std::fs::remove_dir_all(exe.with_extension("update-extract"));

    // Restart into the new binary.
    let args: Vec<String> = std::env::args().skip(1).collect();
    Command::new(&exe)
        .args(&args)
        .spawn()
        .map_err(|e| format!("restarting: {e}"))?;
    std::process::exit(0);
}

/// Extract `archive` into a scratch dir next to the exe and return the path to
/// the executable inside it (matched by file name, e.g. `Limen`/`Limen.exe`).
fn extract_binary(
    archive: &Path,
    exe: &Path,
    exe_name: &std::ffi::OsStr,
    lower_name: &str,
) -> Result<PathBuf, String> {
    let dir = exe.with_extension("update-extract");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating extract dir: {e}"))?;

    let ok = if lower_name.ends_with(".zip") {
        // bsdtar (default `tar` on Windows/macOS) reads zips; fall back to unzip.
        run_ok(Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&dir))
            || run_ok(Command::new("unzip").arg("-oq").arg(archive).arg("-d").arg(&dir))
    } else if lower_name.ends_with(".tar") {
        run_ok(Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&dir))
    } else {
        run_ok(Command::new("tar").arg("-xzf").arg(archive).arg("-C").arg(&dir))
    };
    if !ok {
        return Err("extracting archive failed".into());
    }

    locate(&dir, exe_name).ok_or_else(|| {
        format!("no `{}` found inside the release archive", exe_name.to_string_lossy())
    })
}

/// Run a command, returning whether it exited successfully.
fn run_ok(cmd: &mut Command) -> bool {
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Recursively find a file named `wanted` under `dir`.
fn locate(dir: &Path, wanted: &std::ffi::OsStr) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name() == Some(wanted) {
                return Some(path);
            }
        }
    }
    None
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

    #[test]
    fn asset_selection_prefers_raw_then_archive() {
        let (os, arch) = platform_needle();
        let raw = format!("limen-{os}-{arch}");
        let tar = format!("limen-{os}-{arch}.tar.gz");
        let sha = format!("limen-{os}-{arch}.tar.gz.sha256");

        // Only a tarball + checksum → the tarball is chosen (not the checksum).
        let assets = vec![(tar.clone(), "u_tar".into()), (sha.clone(), "u_sha".into())];
        assert_eq!(select_asset(&assets).as_deref(), Some("u_tar"));

        // A raw binary present → preferred over the archive.
        let assets = vec![
            (tar.clone(), "u_tar".into()),
            (raw.clone(), "u_raw".into()),
            (sha.clone(), "u_sha".into()),
        ];
        assert_eq!(select_asset(&assets).as_deref(), Some("u_raw"));

        // Nothing for this platform → None.
        let assets = vec![("limen-other-arch.tar.gz".into(), "x".into())];
        assert_eq!(select_asset(&assets), None);
    }

    #[test]
    fn extracts_binary_from_tarball() {
        // Build a tarball shaped like our release: <stem>/<exe> inside.
        let base = std::env::temp_dir().join(format!("limen-upd-test-{}", std::process::id()));
        let stem = base.join("Limen-9.9.9-linux-x86_64");
        std::fs::create_dir_all(&stem).unwrap();
        std::fs::write(stem.join("Limen"), b"#!/bin/true\n").unwrap();
        std::fs::write(stem.join("LICENSE"), b"gpl").unwrap();
        let archive = base.join("Limen-9.9.9-linux-x86_64.tar.gz");
        assert!(run_ok(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&base)
            .arg("Limen-9.9.9-linux-x86_64")));

        // Pretend the running exe is base/Limen; extract should find the inner one.
        let fake_exe = base.join("Limen");
        let exe_name = std::ffi::OsStr::new("Limen");
        let found = extract_binary(&archive, &fake_exe, exe_name, "limen.tar.gz").unwrap();
        assert_eq!(found.file_name().unwrap(), "Limen");
        assert_eq!(std::fs::read(&found).unwrap(), b"#!/bin/true\n");

        let _ = std::fs::remove_dir_all(&base);
    }
}
