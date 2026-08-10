//! What is installed, recorded so a reinstall is reproducible.

use limen_registry::*;

#[test]
fn old_lockfile_without_git_fields_still_parses() {
    // Lockfiles written before branch/commit existed must load (serde default),
    // so upgrading Limen never breaks an existing install.
    let text = r#"
version = 1
[[module]]
name = "m"
version = "0.1.0"
source = "git"
reference = "owner/repo"
digest = "sha256:ab"
"#;
    let lock: Lockfile = toml::from_str(text).unwrap();
    assert_eq!(lock.modules.len(), 1);
    assert_eq!(lock.modules[0].branch, "");
    assert_eq!(lock.modules[0].commit, "");
}

#[test]
fn git_fields_round_trip_through_toml() {
    let mut lock = Lockfile::default();
    lock.upsert(LockEntry {
        name: "m".into(),
        version: "0.2.0".into(),
        source: "git".into(),
        reference: "owner/repo".into(),
        digest: "sha256:xy".into(),
        branch: "main".into(),
        commit: "abc1234".into(),
    });
    let text = toml::to_string_pretty(&lock).unwrap();
    let back: Lockfile = toml::from_str(&text).unwrap();
    assert_eq!(back.modules[0].branch, "main");
    assert_eq!(back.modules[0].commit, "abc1234");
}

#[test]
fn upsert_replaces_and_remove_deletes() {
    let mut lock = Lockfile::default();
    let mk = |v: &str| LockEntry {
        name: "m".into(),
        version: v.into(),
        source: "git".into(),
        reference: "o/r".into(),
        digest: "sha256:0".into(),
        branch: String::new(),
        commit: String::new(),
    };
    lock.upsert(mk("0.1.0"));
    lock.upsert(mk("0.2.0")); // same name → replace, not duplicate
    assert_eq!(lock.modules.len(), 1);
    assert_eq!(lock.modules[0].version, "0.2.0");
    assert!(lock.remove("m"));
    assert!(!lock.remove("m"));
    assert!(lock.modules.is_empty());
}
