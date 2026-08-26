//! What a module is, read from its manifest.

use std::collections::BTreeMap;
use std::path::PathBuf;

use limen_proto::{Language, Permissions};
use limen_host::*;

use crate::runtimes::Runtime;

fn spec(launch: Launch) -> ModuleSpec {
    ModuleSpec {
        name: "m".into(),
        display_name: None,
        version: "0".into(),
        description: None,
        authors: vec![],
        tags: vec![],
        repo: None,
        capabilities: vec![],
        requires: BTreeMap::new(),
        optional: BTreeMap::new(),
        permissions: Permissions::default(),
        language: Language::Native,
        launch,
        cwd: PathBuf::from("."),
    }
}

#[test]
fn is_native_lib_only_for_in_process_libraries() {
    // Only a dynamic library loaded in-process needs a restart to update.
    assert!(spec(Launch::Native("lib.so".into())).is_native_lib());
    // A compiled RPC binary runs as a subprocess — no restart needed.
    assert!(!spec(Launch::Binary("bin".into())).is_native_lib());
    // A scripted module re-runs its source — no restart needed.
    assert!(!spec(Launch::Script {
        runtime: Runtime::Python,
        script: "m.py".into(),
    })
    .is_native_lib());
}

/// Write a module folder holding only a manifest, under a name of its own.
fn manifest_only(test: &str, toml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("limen-host-tests").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("limen.toml"), toml).unwrap();
    dir
}

const WONTSTART: &str = r#"
[module]
name = "wontstart"
version = "0.1.0"
language = "native"
entry = "wontstart"
abi = "native"

[provides]
capabilities = ["wontstart.demo"]
"#;

/// A module whose library was never built is still a module. It used to abort
/// the load: `from_manifest_dir` returned the error, `Host::load` passed it up,
/// and the GUI showed a fatal screen — one un-built folder and the whole app
/// had no modules at all.
#[test]
fn a_module_with_no_library_is_listed_rather_than_fatal() {
    let dir = manifest_only("no_library", WONTSTART);
    let spec = ModuleSpec::from_manifest_dir(&dir).expect("the manifest reads fine");

    assert_eq!(spec.name, "wontstart");
    assert_eq!(spec.capabilities, vec!["wontstart.demo".to_string()]);
    match &spec.launch {
        Launch::Unavailable(why) => {
            assert!(why.contains("could not find native library"), "{why}");
        }
        other => panic!("expected Unavailable, got {other:?}"),
    }
    // It is not a library that can be loaded, so nothing offers to hot-swap it.
    assert!(!spec.is_native_lib());
}

/// And loading a folder of modules survives both ways a module can be broken.
/// This used to return `Err`, which the GUI turns into a fatal screen: an app
/// with no modules at all because one of them was mid-update.
#[test]
fn one_broken_module_does_not_take_the_others_with_it() {
    let unbuilt = manifest_only("isolation_broken", WONTSTART);
    let unreadable = manifest_only("isolation_unreadable", "this is not toml {{{");

    let host = Host::load(&[unbuilt, unreadable]).expect("load must not fail");

    // An unparseable manifest yields no module at all, so it is recorded now,
    // under the only name there is — its folder's.
    let failed = host.failed_modules();
    assert!(
        failed.contains_key("isolation_unreadable"),
        "recorded by folder name: {failed:?}"
    );
    assert!(failed["isolation_unreadable"].contains("TOML parse error"));

    // A manifest that reads but has no library still describes a module, so it
    // is listed — carrying the reason it cannot run, which `start` records.
    let listed = host.module_specs();
    let spec = listed
        .iter()
        .find(|s| s.name == "wontstart")
        .expect("the un-built module is still listed");
    assert!(matches!(spec.launch, Launch::Unavailable(_)));
}
