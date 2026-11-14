//! The vault: what it keeps, what it refuses, and that what it writes says
//! nothing without the password. The keychain is a map here — the real one is
//! whoever runs the tests' own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use limen_core::vault::*;

#[derive(Clone, Default)]
struct MemKeys(Arc<Mutex<HashMap<String, String>>>);

impl KeyStore for MemKeys {
    fn get(&self, id: &str) -> Option<String> {
        self.0.lock().unwrap().get(id).cloned()
    }
    fn set(&mut self, id: &str, value: &str) -> Result<(), VaultError> {
        self.0.lock().unwrap().insert(id.into(), value.into());
        Ok(())
    }
    fn delete(&mut self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

/// A fresh directory per test, under the system's temp dir.
fn place(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("limen-vault-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("vault.json")
}

fn new_vault(name: &str, keys: &MemKeys) -> (Vault, PathBuf) {
    let path = place(name);
    let mut v = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(!v.exists());
    v.create("correct horse", Kdf::TEST).unwrap();
    (v, path)
}

#[test]
fn what_is_kept_comes_back_only_with_the_password() {
    let keys = MemKeys::default();
    let (mut v, path) = new_vault("roundtrip", &keys);
    v.set("module:crowdstrike", "client_secret", "s3cr3t-value-xyz").unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("s3cr3t-value-xyz") && !text.contains("client_secret") && !text.contains("crowdstrike"),
        "the file names nothing and shows nothing");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "its owner's alone");
    }
    assert!(!path.with_extension("json.tmp").exists(), "written in one step, nothing left over");

    let mut again = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(again.exists() && !again.is_open());
    assert_eq!(again.get("module:crowdstrike", "client_secret"), Err(VaultError::Locked));
    assert_eq!(again.unlock("wrong horse"), Err(VaultError::WrongPassword));
    again.unlock("correct horse").unwrap();
    assert_eq!(again.get("module:crowdstrike", "client_secret").unwrap().as_deref(), Some("s3cr3t-value-xyz"));
    again.lock();
    assert_eq!(again.get("module:crowdstrike", "client_secret"), Err(VaultError::Locked), "locking closes it");
    assert_eq!(again.check_password("correct horse"), Ok(true));
    assert_eq!(again.check_password("wrong horse"), Ok(false));
    assert!(!again.is_open(), "checking the password opens nothing");
}

#[test]
fn compartments_keep_to_themselves() {
    let keys = MemKeys::default();
    let (mut v, _) = new_vault("compartments", &keys);
    v.set(&module_ns("a"), "token", "A").unwrap();
    v.set(&module_ns("b"), "token", "B").unwrap();
    v.set(LIMEN, "github_token", "G").unwrap();
    assert_eq!(v.get(&module_ns("a"), "token").unwrap().as_deref(), Some("A"));
    assert_eq!(v.get(&module_ns("b"), "token").unwrap().as_deref(), Some("B"));
    assert_eq!(v.get(&module_ns("a"), "github_token").unwrap(), None, "a module does not see Limen's");
    assert_eq!(v.names(&module_ns("a")).unwrap(), ["token"]);
    assert_eq!(v.contents().unwrap().len(), 3);
    assert!(v.delete(&module_ns("a"), "token").unwrap());
    assert!(!v.delete(&module_ns("a"), "token").unwrap(), "already gone");
    assert!(v.remove_ns(&module_ns("b")).unwrap(), "a removed module's compartment goes with it");
    assert_eq!(v.contents().unwrap(), [(LIMEN.to_string(), vec!["github_token".to_string()])]);
}

#[test]
fn a_changed_file_opens_for_no_one() {
    let keys = MemKeys::default();
    let (mut v, path) = new_vault("tamper", &keys);
    v.set(LIMEN, "k", "v").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();

    // One flipped byte in the secrets: the right password, a damaged vault.
    let mut t = json.clone();
    let ct = t["data"]["ct"].as_str().unwrap().to_string();
    let flipped = format!("{}{}", if &ct[..1] == "0" { "1" } else { "0" }, &ct[1..]);
    t["data"]["ct"] = flipped.into();
    std::fs::write(&path, t.to_string()).unwrap();
    let mut x = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(matches!(x.unlock("correct horse"), Err(VaultError::Corrupt(_))), "says the file was changed");

    // A field that plays no part in deriving the key — the vault's id — is
    // still bound to what it guards: changed, nothing opens.
    let mut i = json.clone();
    i["id"] = "0".repeat(32).into();
    std::fs::write(&path, i.to_string()).unwrap();
    let mut z = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert_eq!(z.unlock("correct horse"), Err(VaultError::WrongPassword), "the id is part of the seal");

    // A weaker KDF written into the header: the right password no longer works.
    json["kdf"]["m_kib"] = 8.into();
    std::fs::write(&path, json.to_string()).unwrap();
    let mut y = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert_eq!(y.unlock("correct horse"), Err(VaultError::WrongPassword), "the header is bound to what it guards");

    std::fs::write(&path, "{\"format\":\"something-else\",\"version\":9}").unwrap();
    assert!(matches!(Vault::load(&path, Box::new(keys)), Err(VaultError::Corrupt(_))));
}

#[test]
fn this_machine_can_remember_and_forget() {
    let keys = MemKeys::default();
    let (mut v, path) = new_vault("remember", &keys);
    v.set(LIMEN, "k", "v").unwrap();
    assert!(!v.remembered());
    v.lock();
    assert_eq!(v.remember(true), Err(VaultError::Locked), "only an open vault can be remembered");
    v.unlock("correct horse").unwrap();
    v.remember(true).unwrap();
    assert!(v.remembered());

    let mut next = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(next.unlock_remembered().unwrap(), "opens at start, no password");
    assert_eq!(next.get(LIMEN, "k").unwrap().as_deref(), Some("v"));

    // Another machine — another keychain — has to ask.
    let mut elsewhere = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    assert!(!elsewhere.unlock_remembered().unwrap());
    elsewhere.unlock("correct horse").unwrap();

    next.remember(false).unwrap();
    let mut later = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(!later.unlock_remembered().unwrap(), "forgotten");

    // A stale key in the keychain is not trusted, and is dropped.
    let id = keys.0.lock().unwrap().keys().next().cloned();
    assert!(id.is_none());
    v.remember(true).unwrap();
    let id = keys.0.lock().unwrap().keys().next().cloned().unwrap();
    keys.0.lock().unwrap().insert(id.clone(), "00".repeat(32));
    let mut stale = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(!stale.unlock_remembered().unwrap());
    assert!(keys.0.lock().unwrap().get(&id).is_none(), "the wrong key is forgotten");
}

#[test]
fn a_new_password_replaces_the_old_and_keeps_what_is_remembered() {
    let keys = MemKeys::default();
    let (mut v, path) = new_vault("password", &keys);
    v.set(LIMEN, "k", "v").unwrap();
    v.remember(true).unwrap();
    assert_eq!(v.change_password("nope", "battery staple"), Err(VaultError::WrongPassword));
    v.change_password("correct horse", "battery staple").unwrap();

    let mut a = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    assert_eq!(a.unlock("correct horse"), Err(VaultError::WrongPassword));
    a.unlock("battery staple").unwrap();
    assert_eq!(a.get(LIMEN, "k").unwrap().as_deref(), Some("v"));
    let mut b = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert!(b.unlock_remembered().unwrap(), "the data key did not change");
}

#[test]
fn there_is_one_vault_and_a_missing_one_says_so() {
    let keys = MemKeys::default();
    let (mut v, path) = new_vault("one", &keys);
    assert!(v.create("again", Kdf::TEST).is_err(), "never written over");
    let mut none = Vault::load(&path.with_file_name("absent.json"), Box::new(keys)).unwrap();
    assert_eq!(none.unlock("x"), Err(VaultError::Missing));
    assert_eq!(none.unlock_remembered(), Err(VaultError::Missing));
}

#[test]
fn the_default_cost_is_a_real_one() {
    // 64 MiB, 3 passes: valid parameters, and it opens again.
    let path = place("default-kdf");
    let mut v = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    let t = std::time::Instant::now();
    v.create("pw", Kdf::DEFAULT).unwrap();
    let took = t.elapsed();
    let mut again = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    again.unlock("pw").unwrap();
    assert!(took.as_millis() >= 20, "Argon2id at 64 MiB is not free: {took:?}");
}
