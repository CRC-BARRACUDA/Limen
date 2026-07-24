//! Filesystem helpers: recursive copy and a deterministic content digest.

use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Directories excluded from copying and digesting: version-control metadata and
/// build/cache output. Skipping these keeps installs small and — crucially —
/// keeps a module's digest stable across rebuilds (build artifacts aren't part
/// of the module's identity), and avoids hashing hundreds of MB of `target/`.
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "__pycache__"];

fn is_skipped(name: &std::ffi::OsStr) -> bool {
    SKIP_DIRS.iter().any(|s| name == *s)
}

/// Recursively copy `src` into `dst`, skipping build/cache dirs ([`SKIP_DIRS`]).
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if is_skipped(&name) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// A stable SHA-256 over a directory's contents (sorted relative paths + bytes),
/// returned as `sha256:<hex>`. Used to lock and later verify installs.
pub fn digest_dir(dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for rel in &files {
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        let data = std::fs::read(dir.join(rel))
            .with_context(|| format!("reading {}", dir.join(rel).display()))?;
        hasher.update((data.len() as u64).to_le_bytes());
        hasher.update(&data);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Collect file paths relative to `base`, using forward slashes for stability.
fn collect_files(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if is_skipped(&entry.file_name()) {
                continue; // don't hash build/cache dirs (e.g. target/)
            }
            collect_files(base, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
