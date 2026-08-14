//! Picking a release, and unpacking what it points at.

use std::process::Command;

use limen_core::update::*;
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
