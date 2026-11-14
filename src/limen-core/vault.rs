//! Limen's vault: where Limen and its modules keep tokens and passwords.
//!
//! One file in Limen's home, `vault.json`, so it travels with a portable
//! Limen. Inside it:
//!
//! * the secrets — one compartment per owner (`limen`, `module:<name>`) —
//!   encrypted with a random 256-bit **data key** (XChaCha20-Poly1305);
//! * the data key, encrypted with a key derived from the **master password**
//!   (Argon2id), so the vault opens on any machine that knows the password.
//!
//! "Remember on this machine" keeps the data key in the OS keychain as well,
//! so that machine opens the vault at start without asking. A copy of the
//! folder elsewhere still needs the password.
//!
//! The file's header — format, version, the vault's id and the Argon2
//! parameters — is the associated data of both encryptions: change any of it
//! (to weaken the KDF, say) and nothing decrypts, rather than the vault
//! quietly accepting it.
//!
//! What it does not do: protect secrets from a program running as the same
//! user while the vault is open. Nothing in user space can.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const FORMAT: &str = "limen-vault";
const VERSION: u32 = 1;
/// Limen's own compartment.
pub const LIMEN: &str = "limen";

/// The namespace of a module's compartment.
pub fn module_ns(module: &str) -> String {
    format!("module:{module}")
}

/// What can go wrong, in words the GUI can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultError {
    /// There is no vault yet: one has to be created first.
    Missing,
    /// The vault is locked: unlock it first.
    Locked,
    WrongPassword,
    /// The file is damaged or was tampered with.
    Corrupt(String),
    Io(String),
    /// The OS keychain could not be used.
    Keychain(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::Missing => write!(f, "there is no vault yet"),
            VaultError::Locked => write!(f, "the vault is locked"),
            VaultError::WrongPassword => write!(f, "wrong master password"),
            VaultError::Corrupt(why) => write!(f, "the vault file is damaged: {why}"),
            VaultError::Io(why) => write!(f, "the vault file: {why}"),
            VaultError::Keychain(why) => write!(f, "the system keychain: {why}"),
        }
    }
}

impl std::error::Error for VaultError {}

type R<T> = Result<T, VaultError>;

/// Argon2id's cost. Stored in the file, so a vault keeps opening with the
/// parameters it was made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kdf {
    /// Memory, KiB.
    pub m_kib: u32,
    /// Passes.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl Kdf {
    /// What a new vault uses: 64 MiB, 3 passes — about half a second on a
    /// laptop, and that much for every guess an attacker makes.
    pub const DEFAULT: Kdf = Kdf { m_kib: 64 * 1024, t: 3, p: 1 };
    /// Cheap, for tests only.
    pub const TEST: Kdf = Kdf { m_kib: 64, t: 1, p: 1 };
}

/// A sealed box: a nonce and what was encrypted with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sealed {
    nonce: String,
    ct: String,
}

/// The file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    /// Random; names this vault's entry in the keychain.
    id: String,
    kdf: Kdf,
    salt: String,
    /// The data key, under the password-derived key.
    key: Sealed,
    /// The secrets, under the data key.
    data: Sealed,
}

impl Envelope {
    /// The associated data of both encryptions: everything that says how to
    /// open the rest.
    fn aad(&self) -> Vec<u8> {
        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.format, self.version, self.id, self.kdf.m_kib, self.kdf.t, self.kdf.p, self.salt
        )
        .into_bytes()
    }
}

/// The compartments: namespace → name → value.
type Secrets = BTreeMap<String, BTreeMap<String, String>>;

/// Where "remember on this machine" keeps the data key. The OS keychain in
/// Limen; a map in the tests.
pub trait KeyStore: Send {
    fn get(&self, id: &str) -> Option<String>;
    fn set(&mut self, id: &str, value: &str) -> R<()>;
    fn delete(&mut self, id: &str);
}

/// The OS keychain: Secret Service, Windows Credential Manager, macOS Keychain.
pub struct OsKeyStore;

const KEYCHAIN_SERVICE: &str = "limen-vault";

impl KeyStore for OsKeyStore {
    fn get(&self, id: &str) -> Option<String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, id).ok()?.get_password().ok()
    }

    fn set(&mut self, id: &str, value: &str) -> R<()> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id).map_err(|e| VaultError::Keychain(e.to_string()))?;
        entry.set_password(value).map_err(|e| VaultError::Keychain(e.to_string()))
    }

    fn delete(&mut self, id: &str) {
        if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, id) {
            let _ = e.delete_credential();
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> R<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return Err(VaultError::Corrupt("odd hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| VaultError::Corrupt("bad hex".into())))
        .collect()
}

fn random<const N: usize>() -> R<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| VaultError::Io(format!("no random source: {e}")))?;
    Ok(b)
}

fn derive(password: &str, salt: &[u8], kdf: Kdf) -> R<Zeroizing<[u8; 32]>> {
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32)).map_err(|e| VaultError::Corrupt(e.to_string()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .map_err(|e| VaultError::Corrupt(e.to_string()))?;
    Ok(out)
}

fn seal(key: &[u8; 32], msg: &[u8], aad: &[u8]) -> R<Sealed> {
    let nonce: [u8; 24] = random()?;
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let ct = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg, aad })
        .map_err(|_| VaultError::Io("encryption failed".into()))?;
    Ok(Sealed { nonce: hex(&nonce), ct: hex(&ct) })
}

/// Open a sealed box; `None` when the key is wrong or anything was changed.
fn open(key: &[u8; 32], sealed: &Sealed, aad: &[u8]) -> R<Option<Zeroizing<Vec<u8>>>> {
    let nonce: [u8; 24] = unhex(&sealed.nonce)?
        .try_into()
        .map_err(|_| VaultError::Corrupt("nonce size".into()))?;
    let ct = unhex(&sealed.ct)?;
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    Ok(cipher.decrypt(&XNonce::from(nonce), Payload { msg: &ct, aad }).ok().map(Zeroizing::new))
}

/// The vault while it is open: its data key and its secrets.
struct Open {
    key: Zeroizing<[u8; 32]>,
    secrets: Secrets,
}

impl Drop for Open {
    fn drop(&mut self) {
        // The key is wiped by Zeroizing; the values by hand.
        for ns in self.secrets.values_mut() {
            for v in ns.values_mut() {
                v.zeroize();
            }
        }
    }
}

/// Limen's vault.
pub struct Vault {
    path: PathBuf,
    envelope: Option<Envelope>,
    open: Option<Open>,
    keys: Box<dyn KeyStore>,
}

impl Vault {
    /// The vault at `path` (which may not exist yet), remembering keys in `keys`.
    pub fn load(path: &Path, keys: Box<dyn KeyStore>) -> R<Self> {
        let envelope = match std::fs::read_to_string(path) {
            Ok(text) => {
                let e: Envelope = serde_json::from_str(&text).map_err(|e| VaultError::Corrupt(e.to_string()))?;
                if e.format != FORMAT || e.version != VERSION {
                    return Err(VaultError::Corrupt(format!("unknown format {} v{}", e.format, e.version)));
                }
                Some(e)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(VaultError::Io(e.to_string())),
        };
        Ok(Self { path: path.to_path_buf(), envelope, open: None, keys })
    }

    /// Limen's vault, `<home>/vault.json`, remembering keys in the OS keychain.
    pub fn default_vault() -> R<Self> {
        Self::load(&crate::paths::vault_path(), Box::new(OsKeyStore))
    }

    pub fn exists(&self) -> bool {
        self.envelope.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Make a new, empty vault with this master password, and open it.
    pub fn create(&mut self, password: &str, kdf: Kdf) -> R<()> {
        // Over a file that is there — even one this vault could not read — never.
        if self.exists() || self.path.exists() {
            return Err(VaultError::Io("a vault already exists".into()));
        }
        let key = Zeroizing::new(random::<32>()?);
        self.open = Some(Open { key, secrets: Secrets::new() });
        self.write_with_password(password, kdf, hex(&random::<16>()?))
    }

    /// Open it with the master password.
    pub fn unlock(&mut self, password: &str) -> R<()> {
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let pk = derive(password, &unhex(&e.salt)?, e.kdf)?;
        let Some(key) = open(&pk, &e.key, &e.aad())? else {
            return Err(VaultError::WrongPassword);
        };
        let key: [u8; 32] = key.as_slice().try_into().map_err(|_| VaultError::Corrupt("key size".into()))?;
        // The password opened the key; if the key does not open the secrets,
        // the file was changed after it was written.
        self.open_with(Zeroizing::new(key)).map_err(|e| match e {
            VaultError::WrongPassword => VaultError::Corrupt("the secrets do not match their key".into()),
            e => e,
        })
    }

    /// Whether `password` is the master password — without opening, closing
    /// or changing anything. For asking again before showing a secret.
    pub fn check_password(&self, password: &str) -> R<bool> {
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let pk = derive(password, &unhex(&e.salt)?, e.kdf)?;
        Ok(open(&pk, &e.key, &e.aad())?.is_some())
    }

    /// Open it with the key this machine remembers, if it remembers one.
    /// `Ok(false)`: nothing remembered (or no longer right) — ask for the password.
    pub fn unlock_remembered(&mut self) -> R<bool> {
        let id = self.envelope.as_ref().ok_or(VaultError::Missing)?.id.clone();
        let Some(stored) = self.keys.get(&id) else { return Ok(false) };
        let bytes = Zeroizing::new(unhex(stored.trim()).unwrap_or_default());
        let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) else { return Ok(false) };
        match self.open_with(Zeroizing::new(key)) {
            Ok(()) => Ok(true),
            // A key from an older vault of the same id, say: forget it.
            Err(VaultError::WrongPassword) => {
                self.keys.delete(&id);
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    fn open_with(&mut self, key: Zeroizing<[u8; 32]>) -> R<()> {
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let Some(plain) = open(&key, &e.data, &e.aad())? else {
            return Err(VaultError::WrongPassword);
        };
        let secrets: Secrets = serde_json::from_slice(&plain).map_err(|e| VaultError::Corrupt(e.to_string()))?;
        self.open = Some(Open { key, secrets });
        Ok(())
    }

    /// Close it: the key and the secrets are wiped from memory.
    pub fn lock(&mut self) {
        self.open = None;
    }

    /// Whether this machine remembers the key.
    pub fn remembered(&self) -> bool {
        self.envelope.as_ref().is_some_and(|e| self.keys.get(&e.id).is_some())
    }

    /// Remember the key on this machine, or forget it. The vault must be open
    /// to remember.
    pub fn remember(&mut self, on: bool) -> R<()> {
        let id = self.envelope.as_ref().ok_or(VaultError::Missing)?.id.clone();
        if on {
            let o = self.open.as_ref().ok_or(VaultError::Locked)?;
            let key = Zeroizing::new(hex(o.key.as_ref()));
            self.keys.set(&id, &key)
        } else {
            self.keys.delete(&id);
            Ok(())
        }
    }

    /// A secret, from `ns`.
    pub fn get(&self, ns: &str, name: &str) -> R<Option<String>> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        Ok(o.secrets.get(ns).and_then(|m| m.get(name)).cloned())
    }

    /// Keep a secret in `ns`, written to disk at once.
    pub fn set(&mut self, ns: &str, name: &str, value: &str) -> R<()> {
        let o = self.open.as_mut().ok_or(VaultError::Locked)?;
        if let Some(mut old) = o.secrets.entry(ns.into()).or_default().insert(name.into(), value.into()) {
            old.zeroize();
        }
        self.write()
    }

    /// Remove a secret. Whether there was one.
    pub fn delete(&mut self, ns: &str, name: &str) -> R<bool> {
        let o = self.open.as_mut().ok_or(VaultError::Locked)?;
        let gone = o.secrets.get_mut(ns).and_then(|m| m.remove(name)).map(|mut v| v.zeroize()).is_some();
        if o.secrets.get(ns).is_some_and(|m| m.is_empty()) {
            o.secrets.remove(ns);
        }
        if gone {
            self.write()?;
        }
        Ok(gone)
    }

    /// The names (never the values) in `ns`.
    pub fn names(&self, ns: &str) -> R<Vec<String>> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        Ok(o.secrets.get(ns).map(|m| m.keys().cloned().collect()).unwrap_or_default())
    }

    /// Every compartment and the names in it — for the Vault page.
    pub fn contents(&self) -> R<Vec<(String, Vec<String>)>> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        Ok(o.secrets.iter().map(|(ns, m)| (ns.clone(), m.keys().cloned().collect())).collect())
    }

    /// Remove a whole compartment: a module that was uninstalled.
    pub fn remove_ns(&mut self, ns: &str) -> R<bool> {
        let o = self.open.as_mut().ok_or(VaultError::Locked)?;
        let gone = o.secrets.remove(ns).is_some();
        if gone {
            self.write()?;
        }
        Ok(gone)
    }

    /// A new master password. The data key stays, so a key remembered on
    /// this machine keeps working; a copy of the old file elsewhere still
    /// opens with the old password — that copy is not this file.
    pub fn change_password(&mut self, old: &str, new: &str) -> R<()> {
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let pk = derive(old, &unhex(&e.salt)?, e.kdf)?;
        if open(&pk, &e.key, &e.aad())?.is_none() {
            return Err(VaultError::WrongPassword);
        }
        let (kdf, id) = (e.kdf, e.id.clone());
        if self.open.is_none() {
            self.unlock(old)?;
        }
        self.write_with_password(new, kdf, id)
    }

    /// Re-encrypt the secrets under the data key, keeping the wrapped key.
    fn write(&mut self) -> R<()> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        let mut e = self.envelope.clone().ok_or(VaultError::Missing)?;
        let plain = Zeroizing::new(serde_json::to_vec(&o.secrets).map_err(|e| VaultError::Io(e.to_string()))?);
        e.data = seal(&o.key, &plain, &e.aad())?;
        self.save(e)
    }

    /// Wrap the data key under `password` with a fresh salt, then write all.
    fn write_with_password(&mut self, password: &str, kdf: Kdf, id: String) -> R<()> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        let salt = random::<16>()?;
        let mut e = Envelope {
            format: FORMAT.into(),
            version: VERSION,
            id,
            kdf,
            salt: hex(&salt),
            key: Sealed { nonce: String::new(), ct: String::new() },
            data: Sealed { nonce: String::new(), ct: String::new() },
        };
        let pk = derive(password, &salt, kdf)?;
        e.key = seal(&pk, o.key.as_ref(), &e.aad())?;
        let plain = Zeroizing::new(serde_json::to_vec(&o.secrets).map_err(|e| VaultError::Io(e.to_string()))?);
        e.data = seal(&o.key, &plain, &e.aad())?;
        self.save(e)
    }

    /// Write the file in one step — a new file renamed over the old — readable
    /// by its owner only.
    fn save(&mut self, e: Envelope) -> R<()> {
        let text = serde_json::to_string_pretty(&e).map_err(|e| VaultError::Io(e.to_string()))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| VaultError::Io(e.to_string()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp).map_err(|e| VaultError::Io(e.to_string()))?;
            f.write_all(text.as_bytes()).map_err(|e| VaultError::Io(e.to_string()))?;
            f.sync_all().map_err(|e| VaultError::Io(e.to_string()))?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| VaultError::Io(e.to_string()))?;
        self.envelope = Some(e);
        Ok(())
    }
}
