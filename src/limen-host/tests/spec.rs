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
