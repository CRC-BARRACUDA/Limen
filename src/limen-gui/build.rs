//! Capture the git commit + branch at build time so the About page can show
//! which revision this binary was built from.

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
}
