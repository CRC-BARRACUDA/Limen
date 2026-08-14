//! Hashing a directory, and the rules for what counts.

use std::path::Path;

use limen_registry::*;

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
