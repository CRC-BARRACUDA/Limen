//! Stopping a module's long job, through the real loading path.
//!
//! The module's own tests stop a walk in-process; this one goes through the
//! library boundary the application uses — dlopen, the ABI, the one call at a
//! time the host serialises on — because between the two is where a Stop that
//! works in a test and not in the app would be hiding.

use std::sync::Arc;

use limen_host::{IncomingHandler, NativeModule};
use limen_host::Module;
use serde_json::json;

/// The search module, if it has been built. Not an error when it has not: this
/// repository holds modules that are built separately.
fn search() -> Option<Arc<NativeModule>> {
    let lib = concat!(env!("CARGO_MANIFEST_DIR"), "/../../modules/search/libsearch.so");
    if !std::path::Path::new(lib).exists() {
        eprintln!("skipped: {lib} is not built");
        return None;
    }
    let handler: Arc<IncomingHandler> = Arc::new(|_method: &str, _params: serde_json::Value| {
        Err(limen_proto::rpc::RpcError::new(
            limen_proto::rpc::METHOD_NOT_FOUND,
            "the test host answers nothing",
        ))
    });
    NativeModule::load("search".into(), lib, handler).ok()
}

/// Press Stop on a running search and it ends, through the real ABI.
#[test]
fn a_search_stops_when_it_is_told_to() {
    let Some(module) = search() else { return };
    module
        .call(
            "initialize",
            json!({ "host": { "name": "limen", "version": "test" },
                    "module": { "name": "search", "version": "0" } }),
        )
        .expect("it initialises");

    // A search big enough to be stopped: the home directory, read for
    // everything, with what is inside files read too. A checkout is searched
    // in a blink on a warm cache, and a Stop nobody had time to press proves
    // nothing.
    let here = std::env::var("LIMEN_SEARCH_AT")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    // As the host calls a capability: one `invoke`, carrying which capability
    // and which of its methods.
    let ask = |method: &str, params: serde_json::Value| {
        module.call(
            "invoke",
            json!({ "capability": "search.local", "method": method, "params": params }),
        )
    };
    let waiting = ask(
        "search",
        // A term nothing answers to, so the walk runs to the end of the tree
        // instead of filling up and stopping on its own: a search that hits
        // its own cap in a blink cannot be stopped by hand either.
        json!({ "paths": here, "dict": "/^zzz-nothing-ever-matches-this$/",
                "inside": true, "in_zip": true }),
    )
    .expect("it starts");
    assert!(waiting.get("auto").is_some(), "it should be waiting: {waiting}");

    // Poll as the screen does for a moment, then stop.
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_millis(300) {
        let view = ask("poll", json!({})).expect("it polls");
        if view.get("auto").is_none() {
            eprintln!("skipped: the search finished before it could be stopped");
            return;
        }
    }

    let pressed = std::time::Instant::now();
    ask("stop", json!({})).expect("it stops");
    loop {
        let view = ask("poll", json!({})).expect("it polls");
        if view.get("auto").is_none() {
            break;
        }
        assert!(
            pressed.elapsed() < std::time::Duration::from_secs(20),
            "it was told to stop {:?} ago and is still going",
            pressed.elapsed()
        );
    }
    eprintln!("stopped in {:?}", pressed.elapsed());
}
