//! Which script runtimes are available, and from where.

use limen_host::runtimes::*;

#[test]
fn status_without_bundled_is_system_or_missing() {
    // An empty base has no bundled interpreter, so status must come from PATH
    // (System) or be Missing — never Bundled.
    let base = std::env::temp_dir().join("limen-rt-none");
    assert!(!matches!(
        status(&base, Runtime::Python),
        RuntimeStatus::Bundled(_)
    ));
}

/// Network + ~30MB download. Run explicitly:
///   cargo test -p limen-host --  --ignored quick_setup_python
#[test]
#[ignore]
fn quick_setup_python() {
    let base = std::env::temp_dir().join(format!("limen-rt-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    install(&base, Runtime::Python).expect("install python");

    // Now it should resolve as Bundled and actually run.
    let cmd = match status(&base, Runtime::Python) {
        RuntimeStatus::Bundled(p) => p,
        other => panic!("expected Bundled, got {other:?}"),
    };
    let out = std::process::Command::new(&cmd)
        .args(["-c", "print(1+1)"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2");

    std::fs::remove_dir_all(&base).ok();
}
