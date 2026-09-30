//! Which machine a module is for, and what the host does about it.

use std::path::PathBuf;

use limen_host::*;
use limen_proto::Manifest;

/// A module folder holding only a manifest, under a name of its own.
fn manifest_only(test: &str, toml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("limen-host-platform").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("limen.toml"), toml).unwrap();
    dir
}

/// A manifest for a module declaring `os` — or, given `None`, declaring nothing.
fn manifest(name: &str, os: Option<&str>) -> String {
    let line = os.map(|o| format!("os = {o}\n")).unwrap_or_default();
    format!(
        r#"
[module]
name = "{name}"
version = "0.1.0"
language = "native"
entry = "{name}"
abi = "native"
{line}
[provides]
capabilities = ["{name}.demo"]
"#
    )
}

/// The default is every platform. A module that says nothing about where it
/// runs must keep running everywhere — most modules say nothing, and a filter
/// that treated silence as "nowhere" would empty the app.
#[test]
fn a_module_that_names_no_platform_runs_everywhere() {
    let m = Manifest::from_toml_str(&manifest("quiet", None)).expect("it parses");
    assert!(m.module.os.is_empty(), "nothing declared");
    for os in ["windows", "linux", "macos", "freebsd"] {
        assert!(m.module.runs_on(os), "silence must not exclude {os}");
    }
    assert!(m.module.runs_here());
}

/// Naming a platform includes it and excludes the others.
#[test]
fn naming_a_platform_excludes_the_rest() {
    let m = Manifest::from_toml_str(&manifest("winonly", Some(r#"["windows"]"#))).unwrap();
    assert!(m.module.runs_on("windows"));
    assert!(!m.module.runs_on("linux"));
    assert!(!m.module.runs_on("macos"));

    // Several platforms, all of them honoured.
    let m = Manifest::from_toml_str(&manifest("two", Some(r#"["linux", "macos"]"#))).unwrap();
    assert!(m.module.runs_on("linux"));
    assert!(m.module.runs_on("macos"));
    assert!(!m.module.runs_on("windows"));
}

/// The spellings a manifest author actually writes.
///
/// Matching `std::env::consts::OS` exactly would fail silently: `os =
/// ["Windows"]` would drop the module on every platform, Windows included, and
/// nothing anywhere would say why. A capital letter must not cost a module its
/// place in the list.
#[test]
fn the_platform_name_is_read_as_written() {
    for spelling in ["windows", "Windows", "WINDOWS", " win32 ", "Win"] {
        assert!(
            limen_proto::os_matches(spelling, "windows"),
            "{spelling:?} names Windows"
        );
    }
    for spelling in ["macos", "macOS", "osx", "darwin", "Mac"] {
        assert!(
            limen_proto::os_matches(spelling, "macos"),
            "{spelling:?} names macOS"
        );
    }
    // And a name that is not this platform is still not this platform.
    assert!(!limen_proto::os_matches("linux", "windows"));
}

/// The point of the whole thing: a module for another machine is not loaded,
/// not listed, and not reported as broken.
///
/// Not broken is the part worth pinning. Recording it as a failure would put a
/// card in the module manager saying a module the user cannot use went wrong —
/// when nothing went wrong, it is simply for a different operating system.
#[test]
fn a_module_for_another_platform_is_not_listed_at_all() {
    let elsewhere = if cfg!(windows) { "linux" } else { "windows" };
    let dir = manifest_only(
        "foreign",
        &manifest("foreign", Some(&format!(r#"["{elsewhere}"]"#))),
    );
    let host = Host::load(&[dir]).expect("loading must not fail over it");

    assert!(
        host.module_specs().iter().all(|s| s.name != "foreign"),
        "a module for {elsewhere} must not be listed on {}",
        std::env::consts::OS
    );
    assert!(
        !host.failed_modules().contains_key("foreign"),
        "it did not fail — it is for another machine: {:?}",
        host.failed_modules()
    );
}

/// ...and one that names this platform is loaded normally, so the filter is a
/// filter and not a blanket refusal.
#[test]
fn a_module_for_this_platform_is_listed() {
    let here = std::env::consts::OS;
    let dir = manifest_only("native", &manifest("native", Some(&format!(r#"["{here}"]"#))));
    let host = Host::load(&[dir]).expect("it loads");

    assert!(
        host.module_specs().iter().any(|s| s.name == "native"),
        "a module for {here} must be listed on {here}"
    );
}
