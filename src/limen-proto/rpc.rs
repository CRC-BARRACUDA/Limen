//! JSON-RPC 2.0 message types.
//!
//! The transport is newline-delimited JSON ("JSON Lines"): each frame is one
//! JSON object followed by `\n`. The channel is **bidirectional** — the host
//! sends [`Request`]s to a module (`initialize`, `invoke`, `shutdown`, …) and a
//! module sends [`Request`]s back to the host (`host.call` to reach another
//! module via the broker). A [`Message`] is therefore either a request or a
//! response, disambiguated by the presence of the `method` field.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only protocol version we speak.
pub const JSONRPC_VERSION: &str = "2.0";

fn default_version() -> String {
    JSONRPC_VERSION.to_string()
}

// Standard JSON-RPC error codes (see the JSON-RPC 2.0 spec) plus Limen's own
// application range (-32000..).
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// A module failed / disconnected mid-call.
pub const MODULE_ERROR: i64 = -32000;
/// No installed module provides the requested capability.
pub const NO_PROVIDER: i64 = -32004;

/// A request. `id` is absent for a fire-and-forget notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// A call that expects a matching [`Response`].
    pub fn new(id: u64, method: String, params: Value) -> Self {
        Self { jsonrpc: default_version(), id: Some(id), method, params }
    }

    /// A fire-and-forget notification (no response expected).
    pub fn notification(method: String, params: Value) -> Self {
        Self { jsonrpc: default_version(), id: None, method, params }
    }
}

/// A response to a [`Request`], carrying exactly one of `result` or `error`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(default = "default_version")]
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self { jsonrpc: default_version(), id, result: Some(result), error: None }
    }

    pub fn error(id: u64, error: RpcError) -> Self {
        Self { jsonrpc: default_version(), id, result: None, error: Some(error) }
    }
}

/// A structured error returned in a [`Response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

/// Either side of the conversation. Deserialization is `untagged`: a frame with
/// a `method` field parses as a [`Request`], otherwise as a [`Response`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
