//! Build script: capture the git commit + branch so the About page can show
//! which revision this binary was built from, and (on Windows) embed the app
//! icon + version metadata into the .exe.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=LIMEN_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=LIMEN_GIT_BRANCH={branch}");

    // Rebuild when HEAD moves (branch switch / new commit).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");

    embed_windows_icon();
}

/// On Windows, embed the app icon (and version metadata from Cargo) into the
/// .exe so Explorer, the taskbar, and Alt-Tab show Limen's mark. No-op elsewhere.
#[cfg(windows)]
fn embed_windows_icon() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let icon = std::path::Path::new(&manifest).join("../../resources/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    let mut res = winresource::WindowsResource::new();
    res.set_icon(&icon.to_string_lossy());
    if let Err(e) = res.compile() {
        // Don't fail the build if the resource compiler is unavailable — the app
        // still runs, just without an embedded file icon.
        println!("cargo:warning=embedding Windows icon failed: {e}");
    }
}

#[cfg(not(windows))]
fn embed_windows_icon() {}
