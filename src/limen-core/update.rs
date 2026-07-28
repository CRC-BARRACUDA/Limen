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

use limen_proto::NoConsole;
use std::sync::Mutex;

/// The GitHub repo the app updates from.
pub const APP_REPO: &str = "CRC-BARRACUDA/Limen";

/// Dev-mode override: a local directory of `Limen-<ver>-<platform>.tar.gz` builds
/// to source app updates from instead of GitHub. Session-only (see below).
static UPDATE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Point the app self-update at a local directory (or `None` to clear). Drop
/// `Limen-<ver>-<platform>.tar.gz` builds there and the update check/apply read
/// from the newest one. Set from the in-app **dev mode** (available to everyone);
/// not persisted, so it resets when the app restarts. A relative path resolves
/// against the working directory.
pub fn set_update_dir(dir: Option<PathBuf>) {
    if let Ok(mut slot) = UPDATE_DIR.lock() {
        *slot = dir;
    }
}

/// The active dev-mode update dir this session, if one was set.
fn update_dir() -> Option<PathBuf> {
    UPDATE_DIR.lock().ok().and_then(|slot| slot.clone())
}

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
///
/// Getting this list wrong is destructive, not merely unhelpful: anything not
/// recognised here is taken for a raw binary and renamed straight over the
/// running executable. A `.7z` slipping through did exactly that — the installed
/// `Limen.exe` became a 7-Zip archive, and Windows then refused to start it with
/// "Unsupported 16-Bit Application". Every archive format the packaging scripts
/// can emit must appear here.
fn is_extractable_archive(name: &str) -> bool {
    const EXTS: [&str; 5] = [".tar.gz", ".tgz", ".tar", ".zip", ".7z"];
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

/// Testing variant of [`check_update`]: find the newest `Limen-<ver>-<platform>`
/// archive in `dir` and, if it's newer than `current`, return it as the update —
/// its local path becomes the "asset" that [`apply_update`] copies in.
fn check_update_local(dir: &Path, current: &str) -> Option<UpdateInfo> {
    let (os, arch) = platform_needle();
    let mut best: Option<((u32, u32, u32), String, String)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let fname = entry.file_name().to_string_lossy().to_lowercase();
        if !fname.starts_with("limen-")
            || !is_extractable_archive(&fname)
            || !fname.contains(os)
            || !fname.contains(arch)
        {
            continue;
        }
        let ver = fname.trim_start_matches("limen-").split('-').next().unwrap_or("");
        if ver.is_empty() {
            continue;
        }
        let vt = version_tuple(ver);
        if best.as_ref().is_none_or(|(b, _, _)| vt > *b) {
            best = Some((vt, ver.to_string(), entry.path().to_string_lossy().into_owned()));
        }
    }
    let (_, latest, path) = best?;
    if !is_newer(&latest, current) {
        return None;
    }
    Some(UpdateInfo {
        current: current.to_string(),
        latest,
        url: String::new(),
        notes: format!("Local test build: {path}"),
        asset_url: Some(path),
    })
}

/// Check GitHub for a newer release than `current` (e.g. `env!("CARGO_PKG_VERSION")`).
/// Returns `None` if up to date, offline, or there's no release. When dev mode set
/// an update dir (see [`set_update_dir`]), it sources from there instead.
pub fn check_update(current: &str) -> Option<UpdateInfo> {
    if let Some(dir) = update_dir() {
        return check_update_local(&dir, current);
    }
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
        .no_console()
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

    // Fetch next to the current executable — over the network, or (testing) copy
    // from a local path when the "asset" is a filesystem path rather than a URL.
    let staged = exe.with_extension("update-download");
    let _ = std::fs::remove_file(&staged);
    if asset.starts_with("http://") || asset.starts_with("https://") {
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&staged)
            .arg(asset)
            .no_console()
            .status()
            .map_err(|e| format!("running curl: {e}"))?;
        if !status.success() {
            let _ = std::fs::remove_file(&staged);
            return Err("download failed".into());
        }
    } else {
        std::fs::copy(asset, &staged).map_err(|e| format!("copying local update: {e}"))?;
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

    // Relaunch the *captured* exe path — after the swap `current_exe()` points at
    // the replaced (deleted) inode, so re-deriving it would fail to spawn.
    relaunch(&exe);
}

/// Relaunch `exe` with the current arguments and exit. Never returns.
fn relaunch(exe: &Path) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = Command::new(exe).args(&args).no_console().spawn();
    std::process::exit(0);
}

/// Relaunch the running executable and exit — for cases without a binary swap
/// (e.g. reloading native modules whose code can't hot-swap in-process).
pub fn restart_app() -> ! {
    match std::env::current_exe() {
        Ok(exe) => relaunch(&exe),
        Err(_) => std::process::exit(0),
    }
}

/// The app icon written to disk, so a notifier can point at it.
///
/// Notifiers take an image path, but the icon ships compiled into the binary, so
/// it is extracted to `<home>/state/icon.png` on first use. Rewritten only when
/// missing or a different size, so repeat notifications don't touch the disk.
/// `None` means the notification simply goes out without an icon.
fn icon_file() -> Option<PathBuf> {
    const ICON: &[u8] = include_bytes!("../../resources/icon.png");
    let path = crate::paths::icon_path();
    let current = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if current as usize == ICON.len() {
        return Some(path);
    }
    std::fs::create_dir_all(path.parent()?).ok()?;
    std::fs::write(&path, ICON).ok()?;
    Some(path)
}

/// Show a best-effort native desktop notification using the OS's own notifier
/// (`notify-send` / `osascript` / PowerShell toast) — no extra dependency, and a
/// silent no-op if the notifier isn't present.
pub fn notify(title: &str, body: &str) {
    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("notify-send");
        cmd.args(["--app-name", "Limen"]);
        if let Some(icon) = icon_file() {
            cmd.arg("--icon").arg(icon);
        }
        let _ = cmd.args([title, body]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        // `display notification` has no icon parameter — the notification always
        // carries the icon of whatever ran the script. Showing a custom one needs
        // a signed app bundle, so the icon is simply omitted here.
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let script = format!("display notification {} with title {}", q(body), q(title));
        let _ = Command::new("osascript").args(["-e", &script]).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // Escape for XML first (the toast payload is a document), then for the
        // single-quoted PowerShell string that carries it.
        let xml = |s: &str| {
            s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        };
        let ps = |s: String| s.replace('\'', "''");
        // An `appLogoOverride` image needs a URI, and the toast XML only accepts
        // forward slashes in a file:// path.
        let logo = icon_file()
            .map(|p| {
                format!(
                    "<image placement='appLogoOverride' src='file:///{}'/>",
                    xml(&p.to_string_lossy().replace('\\', "/"))
                )
            })
            .unwrap_or_default();
        // The stock ToastText02 template has no image slot, so build the document
        // by hand with the generic binding instead.
        let doc = format!(
            "<toast><visual><binding template='ToastGeneric'>\
             <text>{}</text><text>{}</text>{logo}\
             </binding></visual></toast>",
            xml(title),
            xml(body),
        );
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null; \
             [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom, ContentType=WindowsRuntime] > $null; \
             $x = New-Object Windows.Data.Xml.Dom.XmlDocument; \
             $x.LoadXml('{}'); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Limen').Show([Windows.UI.Notifications.ToastNotification]::new($x));",
            ps(doc),
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
            .no_console()
            .spawn();
    }
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
    } else if lower_name.ends_with(".tar") || lower_name.ends_with(".7z") {
        // bsdtar reads 7-Zip archives too (libarchive), so no 7z.exe is needed.
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
    cmd.no_console().status().map(|s| s.success()).unwrap_or(false)
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
    fn local_update_dir_picks_newest_matching_archive() {
        let (os, arch) = platform_needle();
        let dir = std::env::temp_dir().join(format!("limen-updtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for v in ["0.8.3", "9.9.9", "0.1.0"] {
            std::fs::write(dir.join(format!("Limen-{v}-{os}-{arch}.tar.gz")), b"x").unwrap();
        }
        // Wrong platform → ignored even though its version is higher.
        std::fs::write(dir.join("Limen-99.0.0-otheros-otherarch.tar.gz"), b"x").unwrap();

        let info = check_update_local(&dir, "0.8.4").expect("update found");
        assert_eq!(info.latest, "9.9.9");
        assert!(info
            .asset_url
            .as_deref()
            .unwrap()
            .ends_with(&format!("Limen-9.9.9-{os}-{arch}.tar.gz")));

        // Already up to date → nothing.
        assert!(check_update_local(&dir, "9.9.9").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
    fn version_comparison_edge_cases() {
        // Differing arity — missing components read as 0.
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
        // Components past patch are ignored.
        assert!(is_newer("1.2.4", "1.2.3.99"));
        // Pre-release-ish suffixes: only the leading digits count.
        assert!(is_newer("1.2.3-rc1", "1.2.2"));
        assert!(!is_newer("1.2.3-rc1", "1.2.3"));
        // Major dominates minor/patch.
        assert!(is_newer("2.0.0", "1.99.99"));
        // `v` prefix on either/both sides is normalized.
        assert!(is_newer("v2.0.0", "v1.0.0"));
        // Garbage never claims to be newer than a real version.
        assert!(!is_newer("", "0.0.1"));
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

    /// A `.7z` must never be mistaken for a raw binary. It once was, and the
    /// updater renamed the archive over the running `Limen.exe` — which Windows
    /// then refused to launch as an "Unsupported 16-Bit Application".
    #[test]
    fn archive_is_never_mistaken_for_a_raw_binary() {
        let (os, arch) = platform_needle();
        let sevenz = format!("limen-1.0.0-{os}-{arch}.7z");
        let sha = format!("{sevenz}.sha256");
        let exe = format!("limen-1.0.0-{os}-{arch}.exe");

        for name in [&sevenz, &format!("limen-{os}-{arch}.zip"), &format!("limen-{os}-{arch}.tgz")] {
            assert!(is_extractable_archive(name), "{name} must count as an archive");
        }

        // Archive + checksum only → the archive is chosen, to be extracted.
        let assets = vec![(sevenz.clone(), "u_7z".into()), (sha.clone(), "u_sha".into())];
        assert_eq!(select_asset(&assets).as_deref(), Some("u_7z"));

        // With a real binary alongside it, the binary wins — and the packaging
        // script names it with the platform tokens so it is visible at all.
        let assets = vec![
            (sevenz, "u_7z".into()),
            (exe, "u_exe".into()),
            (sha, "u_sha".into()),
        ];
        assert_eq!(select_asset(&assets).as_deref(), Some("u_exe"));
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

    /// The Windows packaging script emits a `.7z`, so the updater has to be able
    /// to get the binary back out of one — otherwise it falls through to treating
    /// the archive itself as the new executable.
    #[test]
    fn extracts_binary_from_7z() {
        let base = std::env::temp_dir().join(format!("limen-7z-test-{}", std::process::id()));
        let stem = base.join("Limen-9.9.9-windows-x86_64");
        std::fs::create_dir_all(&stem).unwrap();
        std::fs::write(stem.join("Limen.exe"), b"MZ fake binary").unwrap();
        std::fs::write(stem.join("LICENSE"), b"gpl").unwrap();
        let archive = base.join("Limen-9.9.9-windows-x86_64.7z");
        // bsdtar writes 7z via libarchive; skip rather than fail where it can't.
        let built = run_ok(
            Command::new("tar")
                .arg("-a")
                .arg("-cf")
                .arg(&archive)
                .arg("--format")
                .arg("7zip")
                .arg("-C")
                .arg(&base)
                .arg("Limen-9.9.9-windows-x86_64"),
        );
        if !built {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }

        let fake_exe = base.join("Limen.exe");
        let exe_name = std::ffi::OsStr::new("Limen.exe");
        let found =
            extract_binary(&archive, &fake_exe, exe_name, "limen-9.9.9-windows-x86_64.7z").unwrap();
        assert_eq!(found.file_name().unwrap(), "Limen.exe");
        assert_eq!(std::fs::read(&found).unwrap(), b"MZ fake binary");

        let _ = std::fs::remove_dir_all(&base);
    }
}
