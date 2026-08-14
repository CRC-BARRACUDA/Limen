//! Finding the right binary in a release, and getting it onto disk.
//!
//! Which asset belongs to this machine, whether it is checksummed, and how to
//! unpack it - separate from the registry proper, which is about what is
//! installed rather than about how a file arrives.

use std::path::{Path};
use anyhow::{bail, Context, Result};
use limen_proto::{NoConsole};

use crate::registry::*;

/// The latest release tag for a repo (`owner/repo` or URL), version-normalized
/// (leading `v` stripped). `None` if there's no release, or on any error.
pub fn latest_release_version(repo: &str) -> Option<String> {
    if let Some(dir) = update_modules_dir() {
        return local_module_version(&dir, repo);
    }
    let slug = github_slug(repo);
    let url = format!("https://api.github.com/repos/{slug}/releases/latest");
    let out = crate::http::get(&["-sSL"], Some("application/vnd.github+json"), &url).ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    Some(tag.trim_start_matches('v').to_string())
}

/// The GitHub `owner/repo` slug from a repo ref (short form or full URL).
pub(crate) fn github_slug(repo: &str) -> &str {
    repo.strip_prefix("https://github.com/")
        .or_else(|| repo.strip_prefix("http://github.com/"))
        .unwrap_or(repo)
        .trim_end_matches(".git")
}

/// The filename the host expects for the loadable library, e.g.
/// `liblocal_devices.so` (unix) / `local_devices.dll` (windows).
pub(crate) fn native_lib_filename(entry: &str) -> String {
    format!(
        "{}{entry}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Fetch the list of assets `(name, download_url)` from a GitHub release. Uses
/// the release for `version` (a git tag) if given, else the latest release.
pub(crate) fn release_assets(slug: &str, version: Option<&str>) -> Result<Vec<(String, String)>> {
    let url = match version {
        Some(tag) => format!("https://api.github.com/repos/{slug}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{slug}/releases/latest"),
    };
    let output = crate::http::get(&["-sSL"], Some("application/vnd.github+json"), &url)
        .context("running curl")?;
    if !output.status.success() {
        bail!("curl failed fetching release");
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parsing release JSON")?;
    if let Some(msg) = json.get("message").and_then(|m| m.as_str()) {
        bail!("GitHub: {msg}"); // e.g. "Not Found" when there's no release
    }
    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some((
                        a.get("name")?.as_str()?.to_string(),
                        a.get("browser_download_url")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(assets)
}

/// Whether an asset name is a loadable library for this platform.
pub(crate) fn is_lib(name: &str) -> bool {
    name.ends_with(std::env::consts::DLL_SUFFIX)
}

/// Whether an asset name is a distribution archive we can extract a lib from.
pub(crate) fn is_archive(name: &str) -> bool {
    [".tar.gz", ".tgz", ".tar", ".zip"].iter().any(|e| name.ends_with(e))
}

/// Pick the release asset for this platform. Prefer a raw library
/// (`.so`/`.dll`/`.dylib`, arch-tagged first); otherwise a distribution archive
/// (`.tar.gz`/`.zip`) that we extract the library out of.
pub(crate) fn match_platform_asset(assets: &[(String, String)]) -> Option<&(String, String)> {
    let arch = std::env::consts::ARCH;
    assets
        .iter()
        .find(|(n, _)| is_lib(n) && n.contains(arch))
        .or_else(|| assets.iter().find(|(n, _)| is_lib(n)))
        .or_else(|| assets.iter().find(|(n, _)| is_archive(n) && n.contains(arch)))
        .or_else(|| assets.iter().find(|(n, _)| is_archive(n)))
}

/// Filename-token aliases for the **running** CPU architecture, so a release
/// asset named for this arch is recognized regardless of the convention used
/// (`x86_64` / `amd64` / `x64`, `aarch64` / `arm64`, …). Distinct bitnesses are
/// kept separate so a 32-bit build is never accepted on a 64-bit host and vice
/// versa.
pub(crate) fn running_arch_aliases() -> Vec<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => vec!["x86_64", "x86-64", "amd64", "x64"],
        "x86" => vec!["x86", "i386", "i486", "i586", "i686", "win32", "x32"],
        "aarch64" => vec!["aarch64", "arm64"],
        "arm" => vec!["arm", "armv6", "armv7", "armv7l", "armhf", "armel"],
        "riscv64" => vec!["riscv64"],
        "powerpc64" => vec!["powerpc64", "ppc64", "ppc64le"],
        "s390x" => vec!["s390x"],
        other => vec![other],
    }
}

/// Every architecture token we know how to recognize — used to tell "this asset
/// names a *different* arch" (reject) apart from "this asset names no arch at
/// all" (can't tell — don't reject on that basis).
pub(crate) const KNOWN_ARCH_TOKENS: &[&str] = &[
    "x86_64", "x86-64", "amd64", "x64", "x86", "i386", "i486", "i586", "i686", "win32", "x32",
    "aarch64", "arm64", "armv6", "armv7", "armv7l", "armhf", "armel", "arm", "riscv64",
    "powerpc64", "ppc64", "ppc64le", "s390x", "mips64", "loongarch64",
];

/// Whether an asset filename is compatible with the running CPU architecture
/// (and bit depth). True if it names our arch; false if it names a *different*
/// known arch; true if it carries no recognizable arch token (undecidable).
/// Tokenized on `-`/`.`/`/`/space (not `_`, so `x86_64` stays one token).
pub(crate) fn arch_compatible(name: &str) -> bool {
    let lower = name.to_lowercase();
    let tokens: Vec<&str> = lower.split(['-', '.', '/', '\\', ' ']).collect();
    let ours = running_arch_aliases();
    if tokens.iter().any(|t| ours.contains(t)) {
        return true;
    }
    if tokens.iter().any(|t| KNOWN_ARCH_TOKENS.contains(t)) {
        return false; // names some other arch — not for us
    }
    true // no arch token → can't tell; don't over-hide
}

/// Whether `name` is a loadable library for the running **OS** (the library
/// extension `.so`/`.dll`/`.dylib` is OS-specific) *and* CPU arch/bitness.
pub(crate) fn is_platform_lib(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(std::env::consts::DLL_SUFFIX) && arch_compatible(&lower)
}

/// Whether the asset set contains a checksum file paired with `lib` (e.g.
/// `foo.so` → `foo.so.sha256`, or the extension-stripped `foo.sha256`).
pub(crate) fn has_checksum(lib: &str, assets: &[(String, String)]) -> bool {
    let lib = lib.to_lowercase();
    let stem = lib.strip_suffix(std::env::consts::DLL_SUFFIX).unwrap_or(&lib);
    let wanted = [
        format!("{lib}.sha256"),
        format!("{lib}.sha256sum"),
        format!("{stem}.sha256"),
    ];
    assets
        .iter()
        .any(|(n, _)| wanted.iter().any(|w| w == &n.to_lowercase()))
}

/// List `repo`'s release files, pick the one for this platform, and download it
/// into `dest_dir` named as the host expects. Returns the filename on success.
pub(crate) fn fetch_native_asset(
    repo: &str,
    entry: &str,
    version: Option<&str>,
    dest_dir: &Path,
) -> Result<String> {
    let slug = github_slug(repo);
    let assets = release_assets(slug, version)?;
    if assets.is_empty() {
        bail!("release has no attached files");
    }
    let (name, url) = match_platform_asset(&assets).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} or archive asset among: {}",
            std::env::consts::DLL_SUFFIX,
            assets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
        )
    })?;

    let filename = native_lib_filename(entry);
    let dest = dest_dir.join(&filename);

    if is_archive(&name.to_lowercase()) {
        // Download the archive, extract, and move the library into place.
        let staged = dest_dir.join(".native-download");
        let _ = std::fs::remove_file(&staged);
        crate::http::download(url, &staged)?;
        let lib = extract_lib(&staged, name, dest_dir).inspect_err(|_| {
            let _ = std::fs::remove_file(&staged);
        })?;
        let _ = std::fs::remove_file(&dest);
        std::fs::rename(&lib, &dest).context("installing extracted native library")?;
        let _ = std::fs::remove_file(&staged);
        let _ = std::fs::remove_dir_all(dest_dir.join(".native-extract"));
        Ok(format!("{name} → {filename} (extracted)"))
    } else {
        crate::http::download(url, &dest)?;
        Ok(format!("{name} → {filename}"))
    }
}

/// Extract `archive` (whose original `name` gives its type) into a scratch dir
/// next to `dest_dir` and return the path to the first library inside.
pub(crate) fn extract_lib(archive: &Path, name: &str, dest_dir: &Path) -> Result<std::path::PathBuf> {
    let dir = dest_dir.join(".native-extract");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).context("creating extract dir")?;

    let lower = name.to_lowercase();
    let run = |c: &mut std::process::Command| c.no_console().status().map(|s| s.success()).unwrap_or(false);
    let ok = if lower.ends_with(".zip") {
        run(std::process::Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&dir))
            || run(std::process::Command::new("unzip").arg("-oq").arg(archive).arg("-d").arg(&dir))
    } else if lower.ends_with(".tar") {
        run(std::process::Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&dir))
    } else {
        run(std::process::Command::new("tar").arg("-xzf").arg(archive).arg("-C").arg(&dir))
    };
    if !ok {
        bail!("extracting native archive failed");
    }
    locate_lib(&dir).ok_or_else(|| {
        anyhow::anyhow!("no {} library inside the archive", std::env::consts::DLL_SUFFIX)
    })
}

/// Recursively find the first file that is a loadable library for this platform.
pub(crate) fn locate_lib(dir: &Path) -> Option<std::path::PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| is_lib(&n.to_string_lossy())) {
                return Some(path);
            }
        }
    }
    None
}
