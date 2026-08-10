//! Reading a module's source spec: a repo, a branch, a tag.

use limen_registry::*;

#[test]
fn from_lock_pins_version_but_update_does_not() {
    // Normal reconstruction pins to the recorded version (reproducible).
    match SourceSpec::from_lock("git", "owner/repo", "0.1.0") {
        SourceSpec::Git { repo, version } => {
            assert_eq!(repo, "owner/repo");
            assert_eq!(version.as_deref(), Some("0.1.0"));
        }
        _ => panic!("expected a git spec"),
    }
    // Update must be UNPINNED so a module can move past its installed version
    // (the bug fixed in 0.5.2 — pinned update could never reach a newer tag).
    match SourceSpec::from_lock_latest("git", "owner/repo") {
        SourceSpec::Git { repo, version } => {
            assert_eq!(repo, "owner/repo");
            assert_eq!(version, None, "update must not pin to the installed version");
        }
        _ => panic!("expected a git spec"),
    }
    // Path sources are identical either way.
    assert!(matches!(
        SourceSpec::from_lock_latest("path", "/some/dir"),
        SourceSpec::Path { .. }
    ));
}
