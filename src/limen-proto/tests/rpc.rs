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
