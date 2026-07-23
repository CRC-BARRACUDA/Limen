//! The capability broker.
//!
//! Modules address each other by **capability**, never by name or transport. The
//! broker keeps a `capability -> connection` map and routes a `host.call` to the
//! module that provides the named capability. This is the single indirection
//! that lets, say, a Python `usb` module call a native `crowdstrike` module
//! without knowing anything about it beyond the capability contract.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use limen_proto::rpc::{INVALID_PARAMS, NO_PROVIDER};
use limen_proto::RpcError;
use serde_json::Value;

use crate::module::Module;

#[derive(Default)]
pub struct Broker {
    by_capability: Mutex<HashMap<String, Arc<dyn Module>>>,
}

impl Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record that `conn` provides `capability`.
    pub fn register(&self, capability: &str, conn: Arc<dyn Module>) {
        self.by_capability
            .lock()
            .unwrap()
            .insert(capability.to_string(), conn);
    }

    /// The module providing `capability`, if any.
    pub fn get(&self, capability: &str) -> Option<Arc<dyn Module>> {
        self.by_capability.lock().unwrap().get(capability).cloned()
    }

    /// Route a `host.call`. `params` is `{ capability, method, params }`; the
    /// whole object is forwarded to the provider's `invoke` (it reads `method`
    /// and `params` from it, and ignores `capability`).
    pub fn route(&self, params: Value) -> std::result::Result<Value, RpcError> {
        let capability = params
            .get("capability")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "host.call missing 'capability'"))?;

        let conn = self.get(capability).ok_or_else(|| {
            RpcError::new(NO_PROVIDER, format!("no provider for capability {capability}"))
        })?;

        conn.call("invoke", params)
    }
}
