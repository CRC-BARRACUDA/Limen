//! Filesystem helpers: recursive copy and a deterministic content digest.

use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Directories excluded from copying and digesting: version-control metadata,
/// build/cache output, and `tools/`. Skipping these keeps installs small and —
/// crucially — keeps a module's digest stable across rebuilds (build artifacts
/// aren't part of the module's identity), and avoids hashing hundreds of MB of
/// `target/`.
///
/// `tools/` is where a module keeps content it fetches for itself: a scanner
/// binary, a rule set, anything too large or too licence-encumbered to ship. It
/// is excluded for the same reason as build output — it is not part of what the
/// module *is*, and it carries its own integrity check (a module downloading a
/// tool verifies its checksum before use). Were it hashed, a module would revoke
/// its own trust approval the moment it installed the tool it exists to drive,
/// and `verify` would report it as tampered.
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "__pycache__", "tools"];

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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// A module that downloads a tool into `tools/` must not thereby change its
    /// own identity. The digest pins trust approvals, so if fetched content were
    /// hashed, a module would revoke its own approval the moment it installed the
    /// tool it exists to drive — and `verify` would call it tampered.
    #[test]
    fn fetched_tools_do_not_change_a_modules_digest() {
        let root = std::env::temp_dir().join(format!("limen-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("limen.toml"), "[module]\nname = \"m\"\n");
        write(&root.join("lib.rs"), "fn main() {}\n");
        let before = digest_dir(&root).unwrap();

        // The module fetches a 'binary' for itself.
        write(&root.join("tools/loki-2.12.0/loki"), "ELF...");
        let after = digest_dir(&root).unwrap();
        assert_eq!(
            before, after,
            "tools/ must be outside the module's identity"
        );

        // ...while its actual content still is.
        write(&root.join("lib.rs"), "fn main() { changed() }\n");
        assert_ne!(before, digest_dir(&root).unwrap(), "real edits must show");
        let _ = std::fs::remove_dir_all(&root);
    }
}
