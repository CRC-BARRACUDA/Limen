//! A module's own compartment in Limen's vault: `host.secret_get`,
//! `host.secret_set`, `host.secret_delete`, `host.secret_names`.
//!
//! The host knows which module is calling, so a module only ever reaches its
//! own secrets — it names a secret, never an owner. It must also declare the
//! `secrets` permission, which the user saw when they trusted it.
//!
//! The vault itself lives above the host (in limen-core, opened by the GUI);
//! it is plugged in here with [`set_secret_store`]. Until it is, or while it
//! is locked, the calls answer with an error a module can tell apart —
//! [`rpc::VAULT_LOCKED`] / [`rpc::VAULT_MISSING`] — and try again once the
//! user has unlocked it.

use std::sync::{Arc, RwLock};

use limen_proto::rpc::{self, RpcError};
use serde_json::{json, Value};

use crate::Logger;

/// Why a secret could not be read or kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The vault is locked; the user is being asked to unlock it.
    Locked,
    /// There is no vault yet; the user is being asked to create one.
    NoVault,
    Failed(String),
}

/// Where modules' secrets are kept: Limen's vault, seen per module.
pub trait SecretStore: Send + Sync {
    fn get(&self, module: &str, name: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, module: &str, name: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, module: &str, name: &str) -> Result<bool, SecretError>;
    fn names(&self, module: &str) -> Result<Vec<String>, SecretError>;
}

static STORE: RwLock<Option<Arc<dyn SecretStore>>> = RwLock::new(None);

/// Plug the vault in (or take it out).
pub fn set_secret_store(store: Option<Arc<dyn SecretStore>>) {
    if let Ok(mut s) = STORE.write() {
        *s = store;
    }
}

fn store() -> Option<Arc<dyn SecretStore>> {
    STORE.read().ok().and_then(|s| s.clone())
}

/// A secret's name: what the module calls it. Plain, short and printable — it
/// shows on the Vault page.
fn name_of(params: &Value) -> Result<String, RpcError> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
        return Err(RpcError::new(rpc::INVALID_PARAMS, "a secret needs a name: 1 to 128 printable characters"));
    }
    Ok(name.to_string())
}

fn rpc_error(e: SecretError) -> RpcError {
    match e {
        SecretError::Locked => RpcError::new(rpc::VAULT_LOCKED, "Limen's vault is locked: unlock it, then try again"),
        SecretError::NoVault => {
            RpcError::new(rpc::VAULT_MISSING, "Limen has no vault yet: create one, then try again")
        }
        SecretError::Failed(why) => RpcError::new(rpc::INTERNAL_ERROR, why),
    }
}

/// Answer one of the `host.secret_*` calls for module `who`.
pub(crate) fn host_secret(
    method: &str,
    params: Value,
    may: bool,
    log: &Logger,
    who: &str,
) -> std::result::Result<Value, RpcError> {
    if !may {
        log(&format!("[vault] {who}: refused, no `secrets` permission"));
        return Err(RpcError::new(rpc::INVALID_REQUEST, "this module does not declare the `secrets` permission"));
    }
    let Some(store) = store() else {
        return Err(rpc_error(SecretError::NoVault));
    };
    match method {
        "host.secret_get" => {
            let name = name_of(&params)?;
            store.get(who, &name).map(|v| json!(v)).map_err(rpc_error)
        }
        "host.secret_set" => {
            let name = name_of(&params)?;
            let value = params.get("value").and_then(Value::as_str).ok_or_else(|| {
                RpcError::new(rpc::INVALID_PARAMS, "a secret's value is a string")
            })?;
            store.set(who, &name, value).map_err(rpc_error)?;
            // What was kept, never what it is.
            log(&format!("[vault] {who}: kept {name}"));
            Ok(Value::Null)
        }
        "host.secret_delete" => {
            let name = name_of(&params)?;
            let gone = store.delete(who, &name).map_err(rpc_error)?;
            if gone {
                log(&format!("[vault] {who}: removed {name}"));
            }
            Ok(json!(gone))
        }
        "host.secret_names" => store.names(who).map(|n| json!(n)).map_err(rpc_error),
        other => Err(RpcError::new(rpc::METHOD_NOT_FOUND, format!("no {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A vault of maps, keyed by (module, name), that can be "locked".
    #[derive(Default)]
    struct Fake {
        kept: Mutex<HashMap<(String, String), String>>,
        locked: Mutex<bool>,
    }

    impl SecretStore for Fake {
        fn get(&self, module: &str, name: &str) -> Result<Option<String>, SecretError> {
            if *self.locked.lock().unwrap() {
                return Err(SecretError::Locked);
            }
            Ok(self.kept.lock().unwrap().get(&(module.into(), name.into())).cloned())
        }
        fn set(&self, module: &str, name: &str, value: &str) -> Result<(), SecretError> {
            if *self.locked.lock().unwrap() {
                return Err(SecretError::Locked);
            }
            self.kept.lock().unwrap().insert((module.into(), name.into()), value.into());
            Ok(())
        }
        fn delete(&self, module: &str, name: &str) -> Result<bool, SecretError> {
            Ok(self.kept.lock().unwrap().remove(&(module.into(), name.into())).is_some())
        }
        fn names(&self, module: &str) -> Result<Vec<String>, SecretError> {
            let mut n: Vec<String> =
                self.kept.lock().unwrap().keys().filter(|(m, _)| m == module).map(|(_, n)| n.clone()).collect();
            n.sort();
            Ok(n)
        }
    }

    /// One test at a time: the store is one global.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn logged() -> (Logger, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = lines.clone();
        (Arc::new(move |s: &str| l.lock().unwrap().push(s.to_string())), lines)
    }

    #[test]
    fn a_module_reaches_its_own_compartment_and_needs_the_permission() {
        let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let fake = Arc::new(Fake::default());
        set_secret_store(Some(fake.clone()));
        let (log, lines) = logged();
        let call = |m: &str, p: Value, may: bool, who: &str| host_secret(m, p, may, &log, who);

        let refused = call("host.secret_get", json!({"name": "token"}), false, "a").unwrap_err();
        assert_eq!(refused.code, rpc::INVALID_REQUEST, "no permission, no vault");

        call("host.secret_set", json!({"name": "token", "value": "A-s3cret"}), true, "a").unwrap();
        call("host.secret_set", json!({"name": "token", "value": "B"}), true, "b").unwrap();
        assert_eq!(call("host.secret_get", json!({"name": "token"}), true, "a").unwrap(), json!("A-s3cret"));
        assert_eq!(call("host.secret_get", json!({"name": "token"}), true, "b").unwrap(), json!("B"));
        assert_eq!(call("host.secret_get", json!({"name": "other"}), true, "a").unwrap(), Value::Null);
        // Asking with someone else's name in the params changes nothing: the
        // owner is who is calling.
        assert_eq!(
            call("host.secret_get", json!({"name": "token", "module": "b", "owner": "b"}), true, "a").unwrap(),
            json!("A-s3cret")
        );
        assert_eq!(call("host.secret_names", Value::Null, true, "a").unwrap(), json!(["token"]));
        assert_eq!(call("host.secret_delete", json!({"name": "token"}), true, "a").unwrap(), json!(true));
        assert_eq!(call("host.secret_delete", json!({"name": "token"}), true, "a").unwrap(), json!(false));

        let lines = lines.lock().unwrap();
        assert!(lines.iter().any(|l| l.contains("a: kept token")), "{lines:?}");
        assert!(lines.iter().all(|l| !l.contains("A-s3cret")), "never the value in the log: {lines:?}");
        set_secret_store(None);
    }

    #[test]
    fn locked_and_missing_say_so_and_names_are_checked() {
        let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        let (log, _) = logged();
        set_secret_store(None);
        let missing = host_secret("host.secret_get", json!({"name": "x"}), true, &log, "a").unwrap_err();
        assert_eq!(missing.code, rpc::VAULT_MISSING);

        let fake = Arc::new(Fake::default());
        *fake.locked.lock().unwrap() = true;
        set_secret_store(Some(fake));
        let locked = host_secret("host.secret_get", json!({"name": "x"}), true, &log, "a").unwrap_err();
        assert_eq!(locked.code, rpc::VAULT_LOCKED);

        for bad in [json!({}), json!({"name": ""}), json!({"name": "a\nb"}), json!({"name": "x".repeat(129)})] {
            let e = host_secret("host.secret_get", bad.clone(), true, &log, "a").unwrap_err();
            assert_eq!(e.code, rpc::INVALID_PARAMS, "{bad}");
        }
        let e = host_secret("host.secret_set", json!({"name": "x", "value": 5}), true, &log, "a").unwrap_err();
        assert_eq!(e.code, rpc::INVALID_PARAMS, "a value is a string");
        set_secret_store(None);
    }
}
