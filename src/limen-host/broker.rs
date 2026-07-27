//! The capability broker.
//!
//! Modules address each other by **capability**, never by name or transport. The
//! broker keeps a `capability -> connection` map and routes a `host.call` to the
//! module that provides the named capability. This is the single indirection
//! that lets, say, a Python `usb` module call a native `crowdstrike` module
//! without knowing anything about it beyond the capability contract.
//!
//! It also carries a lightweight **pub/sub layer** for callbacks: a module
//! `host.subscribe`s to a topic, another `host.emit`s to it, and the broker
//! pushes an `event` notification to every subscriber. Events are fire-and-forget
//! — the emitter never blocks on subscribers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use limen_proto::rpc::{INVALID_PARAMS, NO_PROVIDER};
use limen_proto::RpcError;
use serde_json::{json, Value};

use crate::module::Module;

#[derive(Default)]
pub struct Broker {
    by_capability: Mutex<HashMap<String, Arc<dyn Module>>>,
    /// module name -> connection (for pushing events to subscribers).
    by_name: Mutex<HashMap<String, Arc<dyn Module>>>,
    /// topic -> subscriber module names.
    subscriptions: Mutex<HashMap<String, Vec<String>>>,
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

    /// Record a module by name (so events can be pushed to it).
    pub fn register_name(&self, name: &str, conn: Arc<dyn Module>) {
        self.by_name.lock().unwrap().insert(name.to_string(), conn);
    }

    /// The module providing `capability`, if any.
    pub fn get(&self, capability: &str) -> Option<Arc<dyn Module>> {
        self.by_capability.lock().unwrap().get(capability).cloned()
    }

    /// Every capability currently provided by a loaded module, sorted. Lets a
    /// module discover optional integrations (e.g. "is a report provider here?").
    pub fn capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = self.by_capability.lock().unwrap().keys().cloned().collect();
        caps.sort();
        caps
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

    /// Handle `host.subscribe`. `params` is `{ subscriber, topic }`: register the
    /// named module as a subscriber of the topic.
    pub fn subscribe(&self, params: Value) -> std::result::Result<Value, RpcError> {
        let subscriber = str_field(&params, "subscriber")?;
        let topic = str_field(&params, "topic")?;
        let mut subs = self.subscriptions.lock().unwrap();
        let list = subs.entry(topic.to_string()).or_default();
        if !list.iter().any(|n| n == subscriber) {
            list.push(subscriber.to_string());
        }
        Ok(Value::Null)
    }

    /// Handle `host.emit`. `params` is `{ topic, payload }`: push an `event`
    /// notification to every subscriber of the topic. Never blocks on delivery.
    /// Returns the number of subscribers notified.
    pub fn emit(&self, params: Value) -> std::result::Result<Value, RpcError> {
        let topic = str_field(&params, "topic")?;
        let payload = params.get("payload").cloned().unwrap_or(Value::Null);

        let names = self
            .subscriptions
            .lock()
            .unwrap()
            .get(topic)
            .cloned()
            .unwrap_or_default();

        let mut delivered = 0;
        for name in &names {
            let conn = self.by_name.lock().unwrap().get(name).cloned();
            if let Some(conn) = conn {
                conn.notify("event", json!({ "topic": topic, "payload": payload }));
                delivered += 1;
            }
        }
        Ok(json!({ "delivered": delivered }))
    }
}

fn str_field<'a>(params: &'a Value, key: &str) -> std::result::Result<&'a str, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("missing '{key}'")))
}
