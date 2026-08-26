//! The JSON-RPC envelope modules speak.

use limen_proto::rpc::*;

use serde_json::json;

#[test]
fn request_roundtrips_and_is_detected() {
    let req = Request::new(7, "invoke".into(), json!({"method": "greet"}));
    let wire = serde_json::to_string(&Message::Request(req)).unwrap();
    match serde_json::from_str::<Message>(&wire).unwrap() {
        Message::Request(r) => {
            assert_eq!(r.id, Some(7));
            assert_eq!(r.method, "invoke");
        }
        Message::Response(_) => panic!("should have parsed as a request"),
    }
}

#[test]
fn response_without_method_is_detected() {
    let wire = r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#;
    match serde_json::from_str::<Message>(wire).unwrap() {
        Message::Response(r) => assert_eq!(r.id, 7),
        Message::Request(_) => panic!("should have parsed as a response"),
    }
}

/// A module panic has to be told apart from every other failure, and by the
/// time it reaches the UI it is a formatted string on the far side of a
/// thread — so the code is read back out of the text.
#[test]
fn a_module_panic_is_recognised_by_its_code() {
    let rendered = RpcError::new(
        MODULE_PANIC,
        "module panicked in invoke: called `Option::unwrap()` on a `None` value",
    )
    .to_string();
    assert_eq!(
        panic_detail(&rendered),
        Some("module panicked in invoke: called `Option::unwrap()` on a `None` value")
    );

    // As the host wraps it on the way up, with its own context in front.
    let wrapped = format!("invoke eml.triage.dashboard: {rendered}");
    assert!(panic_detail(&wrapped).unwrap().starts_with("module panicked"));
}

/// Every other failure is an ordinary one: a module that answers with an error
/// is working exactly as intended, and must not be marked as dead.
#[test]
fn an_ordinary_error_is_not_a_panic() {
    for code in [METHOD_NOT_FOUND, INVALID_PARAMS, INTERNAL_ERROR, MODULE_ERROR, NO_PROVIDER] {
        let text = RpcError::new(code, "no").to_string();
        assert_eq!(panic_detail(&text), None, "code {code}");
    }
    assert_eq!(panic_detail("no provider for capability eml.triage"), None);
    assert_eq!(panic_detail(""), None);
}

/// The exact line the host produced for a real panicking module, kept verbatim
/// so a change to either side's wording cannot quietly break detection.
#[test]
fn the_text_a_real_host_produces_is_detected() {
    let observed = "invoke eml.triage.dashboard: [-32005] module panicked in invoke: \
                    called `Option::unwrap()` on a `None` value";
    let detail = panic_detail(observed).expect("this is what limen-cli printed");
    assert!(detail.starts_with("module panicked in invoke:"));
}
