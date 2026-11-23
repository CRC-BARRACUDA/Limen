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

// ---- factors: a key file, a security key -------------------------------- //

/// A security key in a test: answers with a digest of its secret, the slot and
/// the challenge — standing in for HMAC-SHA1 — and notes what it was asked.
struct FakeKey {
    secret: &'static str,
    asked: Mutex<Vec<(u8, Vec<u8>)>>,
}

impl FakeKey {
    fn new(secret: &'static str) -> Self {
        Self { secret, asked: Mutex::new(Vec::new()) }
    }
}

impl ChallengeResponse for FakeKey {
    fn respond(&self, slot: u8, challenge: &[u8]) -> Result<zeroize::Zeroizing<[u8; 20]>, VaultError> {
        use sha2::{Digest, Sha256};
        self.asked.lock().unwrap().push((slot, challenge.to_vec()));
        let d = Sha256::new().chain_update(self.secret).chain_update([slot]).chain_update(challenge).finalize();
        Ok(zeroize::Zeroizing::new(d[..20].try_into().unwrap()))
    }
}

fn header(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn reload(path: &std::path::Path) -> Vault {
    Vault::load(path, Box::new(MemKeys::default())).unwrap()
}

#[test]
fn a_vault_made_by_version_1_still_opens_and_moves_to_2_with_a_new_key() {
    let path = place("v1");
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/vault-v1.json"), &path).unwrap();
    assert_eq!(header(&path)["version"], 1);

    let mut v = reload(&path);
    assert_eq!(v.factors(), Some(&Factors::default()), "version 1 needs only the password");
    assert_eq!(v.unlock("wrong horse"), Err(VaultError::WrongPassword));
    v.unlock("correct horse").unwrap();
    assert_eq!(v.get("module:crowdstrike", "client_secret").unwrap().as_deref(), Some("kept in version 1"));

    // Keeping a secret leaves it a version-1 file, still opening as one.
    v.set("limen", "github_token", "t").unwrap();
    assert_eq!(header(&path)["version"], 1);
    reload(&path).unlock("correct horse").unwrap();

    // A new key writes it anew, as version 2.
    v.change_key(&"correct horse".into(), &Credentials { keyfile: Some(b"a key file"), ..Credentials::password("battery staple") }).unwrap();
    assert_eq!(header(&path)["version"], 2);
    let mut again = reload(&path);
    again.unlock(Credentials { keyfile: Some(b"a key file"), ..Credentials::password("battery staple") }).unwrap();
    assert_eq!(again.get("module:crowdstrike", "client_secret").unwrap().as_deref(), Some("kept in version 1"));
}

#[test]
fn a_key_file_is_needed_and_has_to_be_the_one() {
    let path = place("keyfile");
    let with = |file: &'static [u8]| Credentials { keyfile: Some(file), ..Credentials::password("correct horse") };
    let mut v = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    v.create(with(b"the key file"), Kdf::TEST).unwrap();
    v.set("a", "x", "1").unwrap();
    assert_eq!(header(&path)["factors"]["keyfile"], true);

    let mut v = reload(&path);
    assert!(v.factors().unwrap().keyfile);
    assert_eq!(v.unlock("correct horse"), Err(VaultError::NeedsFactor(Factor::KeyFile)), "the password alone is not enough");
    assert_eq!(v.unlock(with(b"another file")), Err(VaultError::WrongKey));
    assert_eq!(v.unlock(Credentials { password: "wrong horse", ..with(b"the key file") }), Err(VaultError::WrongKey));
    assert_eq!(v.check_password(with(b"the key file")), Ok(true));
    assert_eq!(v.check_password(with(b"another file")), Ok(false));
    assert!(!v.is_open());
    v.unlock(with(b"the key file")).unwrap();
    assert_eq!(v.get("a", "x").unwrap().as_deref(), Some("1"));
}

#[test]
fn a_security_key_answers_the_vaults_own_challenge() {
    let path = place("security-key");
    let mine = FakeKey::new("mine");
    let mut v = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    v.create(Credentials { security_key: Some((&mine, 2)), ..Credentials::password("correct horse") }, Kdf::TEST).unwrap();
    v.set("a", "x", "1").unwrap();
    let challenge = header(&path)["factors"]["security_key"]["challenge"].as_str().unwrap().to_string();
    assert_eq!(header(&path)["factors"]["security_key"]["slot"], 2);
    assert_eq!(challenge.len(), 128, "64 random bytes");
    assert_eq!(mine.asked.lock().unwrap().last().unwrap().0, 2, "asked on the slot it was set up on");

    let mut v = reload(&path);
    assert_eq!(v.unlock("correct horse"), Err(VaultError::NeedsFactor(Factor::SecurityKey)));
    let other = FakeKey::new("someone else's");
    assert_eq!(v.unlock(Credentials { security_key: Some((&other, 2)), ..Credentials::password("correct horse") }), Err(VaultError::WrongKey));
    // The slot asked is the vault's, whatever the caller says.
    mine.asked.lock().unwrap().clear();
    v.unlock(Credentials { security_key: Some((&mine, 1)), ..Credentials::password("correct horse") }).unwrap();
    let asked = mine.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "one question, so one touch");
    assert_eq!(asked[0].0, 2);
    assert_eq!(asked[0].1.len(), 64);
    assert_eq!(v.get("a", "x").unwrap().as_deref(), Some("1"));
}

#[test]
fn all_three_together() {
    let path = place("all-three");
    let key = FakeKey::new("k");
    let full = Credentials { password: "correct horse", keyfile: Some(b"file"), security_key: Some((&key, 1)) };
    let mut v = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    v.create(full, Kdf::TEST).unwrap();
    let mut v = reload(&path);
    assert_eq!(v.unlock(Credentials { keyfile: None, ..full }), Err(VaultError::NeedsFactor(Factor::KeyFile)));
    assert_eq!(v.unlock(Credentials { security_key: None, ..full }), Err(VaultError::NeedsFactor(Factor::SecurityKey)));
    v.unlock(full).unwrap();
}

#[test]
fn taking_a_factor_out_of_the_file_opens_nothing() {
    let path = place("stripped");
    let key = FakeKey::new("k");
    let creds = Credentials { keyfile: Some(b"file"), security_key: Some((&key, 2)), ..Credentials::password("correct horse") };
    Vault::load(&path, Box::new(MemKeys::default())).unwrap().create(creds, Kdf::TEST).unwrap();
    let original = std::fs::read_to_string(&path).unwrap();

    let edit = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut e: serde_json::Value = serde_json::from_str(&original).unwrap();
        f(&mut e);
        std::fs::write(&path, serde_json::to_string(&e).unwrap()).unwrap();
        reload(&path)
    };
    // No factors at all: only the password is asked for — and it opens nothing.
    let mut v = edit(&|e| { e.as_object_mut().unwrap().remove("factors"); });
    assert!(v.unlock("correct horse").is_err());
    assert!(!v.is_open());
    // The key file dropped, the security key kept.
    let mut v = edit(&|e| { e["factors"].as_object_mut().unwrap().remove("keyfile"); });
    assert!(v.unlock(Credentials { keyfile: None, ..creds }).is_err());
    // Another slot, or another challenge.
    let mut v = edit(&|e| e["factors"]["security_key"]["slot"] = 1.into());
    assert!(v.unlock(creds).is_err());
    let mut v = edit(&|e| e["factors"]["security_key"]["challenge"] = "00".repeat(64).into());
    assert!(v.unlock(creds).is_err());
    // As written, it opens.
    let mut v = edit(&|_| {});
    v.unlock(creds).unwrap();
}

#[test]
fn a_new_key_draws_a_new_challenge_and_keeps_what_is_remembered() {
    let keys = MemKeys::default();
    let path = place("new-challenge");
    let key = FakeKey::new("k");
    let with_key = Credentials { security_key: Some((&key, 2)), ..Credentials::password("correct horse") };
    let mut v = Vault::load(&path, Box::new(keys.clone())).unwrap();
    v.create(with_key, Kdf::TEST).unwrap();
    v.remember(true).unwrap();
    let first = header(&path)["factors"]["security_key"]["challenge"].clone();

    assert_eq!(v.change_key(&"correct horse".into(), &with_key), Err(VaultError::NeedsFactor(Factor::SecurityKey)), "the old key is asked for too");
    v.change_key(&with_key, &with_key).unwrap();
    assert_ne!(header(&path)["factors"]["security_key"]["challenge"], first, "a new challenge");
    reload(&path).unlock(with_key).unwrap();

    // Down to a password alone: the factors leave the header.
    v.change_key(&with_key, &"correct horse".into()).unwrap();
    assert!(header(&path).get("factors").is_none());
    reload(&path).unlock("correct horse").unwrap();

    // Remembered on this machine all along: the data key never changed.
    let mut here = Vault::load(&path, Box::new(keys.clone())).unwrap();
    assert_eq!(here.unlock_remembered(), Ok(true));
}

#[test]
fn a_new_key_file_is_random() {
    let (a, b) = (new_keyfile().unwrap(), new_keyfile().unwrap());
    assert_ne!(*a, *b);
    assert_eq!(a.len(), 65, "32 bytes as hex, and a newline");
    assert!(a[..64].iter().all(u8::is_ascii_hexdigit));
}

/// A real vault, locked with a real YubiKey: made, closed, read back from the
/// disk and opened again. Needs one plugged in with `LIMEN_YUBIKEY_SLOT` set
/// up for challenge-response.
#[test]
#[ignore = "needs a YubiKey plugged in; set LIMEN_YUBIKEY_SLOT"]
fn a_vault_locked_with_a_real_yubikey_opens_with_it() {
    let slot: u8 = std::env::var("LIMEN_YUBIKEY_SLOT").expect("LIMEN_YUBIKEY_SLOT").parse().unwrap();
    let key = limen_core::security_key::find().unwrap().expect("no YubiKey plugged in");
    let path = place("real-yubikey");
    let creds = Credentials { password: "correct horse", keyfile: Some(b"file"), security_key: Some((&key, slot)) };
    let mut v = Vault::load(&path, Box::new(MemKeys::default())).unwrap();
    v.create(creds, Kdf::TEST).unwrap();
    v.set("a", "x", "kept").unwrap();
    let mut v = reload(&path);
    assert_eq!(v.unlock(Credentials { security_key: None, ..creds }), Err(VaultError::NeedsFactor(Factor::SecurityKey)));
    v.unlock(creds).unwrap();
    assert_eq!(v.get("a", "x").unwrap().as_deref(), Some("kept"));
}
