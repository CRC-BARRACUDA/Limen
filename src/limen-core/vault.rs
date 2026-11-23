//! Limen's vault: where Limen and its modules keep tokens and passwords.
//!
//! One file in Limen's home, `vault.json`, so it travels with a portable
//! Limen. Inside it:
//!
//! * the secrets — one compartment per owner (`limen`, `module:<name>`) —
//!   encrypted with a random 256-bit **data key** (XChaCha20-Poly1305);
//! * the data key, encrypted with a key derived from the **master password**
//!   (Argon2id), so the vault opens on any machine that knows the password —
//!   and, if the vault was set up with them, has its other factors:
//!
//! **Factors.** Besides the password — never instead of it — a vault can need
//! a **key file**, and **one of its security keys**: YubiKeys answering
//! HMAC-SHA1 challenge-response, a primary and a backup, say, either of which
//! will do. The key derived from the password is mixed (SHA-256) with the key
//! file's SHA-256 and with the vault's **security-key secret**, 32 random
//! bytes; that is what wraps the data key. Each enrolled security key keeps its
//! own copy of that secret, sealed under its answer to a challenge of its own —
//! so any one of them yields it, and adding a key, or changing the password or
//! the key file, leaves the others as they are. Removing a key draws a new
//! secret, and the keys kept are enrolled again: a lost key and an old copy of
//! the file must not open the vault as it is now. Which factors a vault needs —
//! each key's serial, slot, challenge and sealed secret — are part of the
//! header, so taking one out of the file makes nothing open.
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
/// 2 added factors. A version-1 file opens as it always did, and is written as
/// version 2 the next time its password or factors change.
const VERSION: u32 = 2;
/// The size of the challenge a security key is given: the most HMAC-SHA1
/// challenge-response takes.
const CHALLENGE_LEN: usize = 64;
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
    /// The password, the key file or the security key is wrong — said when
    /// the vault needs more than a password, since which one cannot be told.
    WrongKey,
    /// A factor the vault needs was not given: the key file, the security key.
    NeedsFactor(Factor),
    /// The security key plugged in is not one the vault enrolled.
    UnknownSecurityKey,
    /// The security key could not be asked: not plugged in, not answering,
    /// not touched in time.
    SecurityKey(String),
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
            VaultError::WrongKey => write!(f, "wrong master password, key file or security key"),
            VaultError::NeedsFactor(Factor::KeyFile) => write!(f, "this vault needs its key file"),
            VaultError::NeedsFactor(Factor::SecurityKey) => write!(f, "this vault needs its security key"),
            VaultError::SecurityKey(why) => write!(f, "the security key: {why}"),
            VaultError::UnknownSecurityKey => write!(f, "this security key is not one of the vault's"),
            VaultError::Corrupt(why) => write!(f, "the vault file is damaged: {why}"),
            VaultError::Io(why) => write!(f, "the vault file: {why}"),
            VaultError::Keychain(why) => write!(f, "the system keychain: {why}"),
        }
    }
}

impl std::error::Error for VaultError {}

type R<T> = Result<T, VaultError>;

/// A 256-bit key, wiped when dropped.
type Key32 = Zeroizing<[u8; 32]>;

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

/// One of the things, besides the password, a vault can need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Factor {
    KeyFile,
    SecurityKey,
}

/// A security key that answers HMAC-SHA1 challenge-response: a YubiKey's OTP
/// slot, or a stand-in in tests. Asking may wait for a touch.
pub trait ChallengeResponse {
    /// HMAC-SHA1 of `challenge` under the secret kept in `slot` (1 or 2).
    fn respond(&self, slot: u8, challenge: &[u8]) -> R<Zeroizing<[u8; 20]>>;
    /// Its serial number, when it tells: the vault then asks it only its own
    /// question — one touch, not one per enrolled key.
    fn serial(&self) -> Option<u32> {
        None
    }
    /// What it calls itself.
    fn name(&self) -> String {
        "security key".into()
    }
}

/// What opens the vault: the master password, and its other factors when it
/// has them. A bare `&str` is a password alone.
#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub password: &'a str,
    /// The key file's contents.
    pub keyfile: Option<&'a [u8]>,
    /// The security keys plugged in: any one the vault enrolled will do.
    pub security_keys: &'a [&'a dyn ChallengeResponse],
}

impl<'a> Credentials<'a> {
    pub fn password(password: &'a str) -> Self {
        Self { password, keyfile: None, security_keys: &[] }
    }
}

impl<'a> From<&'a str> for Credentials<'a> {
    fn from(password: &'a str) -> Self {
        Self::password(password)
    }
}

impl<'a> From<&'a String> for Credentials<'a> {
    fn from(password: &'a String) -> Self {
        Self::password(password)
    }
}

/// A security key asked its own new challenge, ready to be added to a vault.
/// Holds its answer — wiped when dropped — until the key is written.
#[derive(Clone)]
pub struct Enrolled {
    pub serial: Option<u32>,
    pub name: String,
    pub slot: u8,
    challenge: [u8; CHALLENGE_LEN],
    answer: Zeroizing<[u8; 20]>,
}

/// Ask `device` a new challenge on `slot`, to add it to a vault — one key at
/// a time, so a primary and a backup are enrolled one after the other.
pub fn enroll(device: &dyn ChallengeResponse, slot: u8) -> R<Enrolled> {
    let challenge = random::<CHALLENGE_LEN>()?;
    let answer = device.respond(slot, &challenge)?;
    Ok(Enrolled { serial: device.serial(), name: device.name(), slot, challenge, answer })
}

/// What a new key is to be: the password, the key file, and the security keys.
#[derive(Clone, Copy)]
pub struct NewKey<'a> {
    pub password: &'a str,
    pub keyfile: Option<&'a [u8]>,
    /// Changing: keep the security keys the vault has. Otherwise they go, and
    /// the secret with them.
    pub keep_security_keys: bool,
    /// Security keys to add.
    pub add: &'a [Enrolled],
}

impl<'a> NewKey<'a> {
    pub fn password(password: &'a str) -> Self {
        Self { password, keyfile: None, keep_security_keys: false, add: &[] }
    }
}

impl<'a> From<&'a str> for NewKey<'a> {
    fn from(password: &'a str) -> Self {
        Self::password(password)
    }
}

impl<'a> From<&'a String> for NewKey<'a> {
    fn from(password: &'a String) -> Self {
        Self::password(password)
    }
}

/// The factors a vault needs besides its password: public, kept in the
/// header, and bound to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Factors {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keyfile: bool,
    /// Any one of these opens it (with the password, and the key file).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_keys: Vec<SecurityKeyFactor>,
}

/// One security key a vault enrolled: which key, which slot, its challenge,
/// and the vault's security-key secret sealed under its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityKeyFactor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<u32>,
    #[serde(default)]
    pub name: String,
    pub slot: u8,
    /// Hex. Not a secret: what makes it worth anything is the key's answer.
    pub challenge: String,
    secret: Sealed,
}

impl SecurityKeyFactor {
    /// Bound to this vault, and to which key and question it is.
    fn aad(&self, vault_id: &str) -> Vec<u8> {
        let serial = self.serial.map_or("-".to_string(), |s| s.to_string());
        format!("limen-vault/security-key|{vault_id}|{serial}|{}|{}", self.slot, self.challenge).into_bytes()
    }

    fn sealing_key(answer: &[u8; 20]) -> Zeroizing<[u8; 32]> {
        use sha2::{Digest, Sha256};
        Zeroizing::new(Sha256::new().chain_update(b"limen-vault/security-key/1").chain_update(answer).finalize().into())
    }
}

impl Factors {
    pub fn is_empty(&self) -> bool {
        !self.keyfile && self.security_keys.is_empty()
    }

    /// As the header binds them: changing any of it changes the seal.
    fn bound(&self) -> String {
        let keys = if self.security_keys.is_empty() {
            "-".to_string()
        } else {
            self.security_keys
                .iter()
                .map(|k| {
                    let serial = k.serial.map_or("-".to_string(), |s| s.to_string());
                    format!("{serial}:{}:{}:{}:{}", k.slot, k.challenge, k.secret.nonce, k.secret.ct)
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        format!("keyfile={}|security_keys={keys}", self.keyfile as u8)
    }
}

/// A new key file's contents: 32 random bytes, as hex. Any file can be a key
/// file — it is its SHA-256 that counts — but one made here is plainly random.
pub fn new_keyfile() -> R<Zeroizing<Vec<u8>>> {
    let bytes = Zeroizing::new(random::<32>()?);
    Ok(Zeroizing::new(format!("{}\n", hex(bytes.as_ref())).into_bytes()))
}

/// A sealed box: a nonce and what was encrypted with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// What, besides the password, opens it (version 2).
    #[serde(default, skip_serializing_if = "Factors::is_empty")]
    factors: Factors,
    /// The data key, under the key the password — and the factors — make.
    key: Sealed,
    /// The secrets, under the data key.
    data: Sealed,
}

impl Envelope {
    /// The associated data of both encryptions: everything that says how to
    /// open the rest.
    fn aad(&self) -> Vec<u8> {
        let mut aad = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.format, self.version, self.id, self.kdf.m_kib, self.kdf.t, self.kdf.p, self.salt
        );
        // Version 1 sealed exactly the above; from 2 on, the factors too.
        if self.version >= 2 {
            aad.push('|');
            aad.push_str(&self.factors.bound());
        }
        aad.into_bytes()
    }

    /// The key that wraps the data key: the password's, mixed with the key
    /// file's and the security-key secret when the vault has them.
    fn wrapping_key(&self, password: &str, keyfile: Option<&[u8]>, secret: Option<&[u8; 32]>) -> R<Zeroizing<[u8; 32]>> {
        use sha2::{Digest, Sha256};
        let pk = derive(password, &unhex(&self.salt)?, self.kdf)?;
        if self.factors.is_empty() {
            return Ok(pk);
        }
        let mut h = Sha256::new();
        h.update(b"limen-vault/factors/2");
        h.update(pk.as_ref());
        if self.factors.keyfile {
            let file = keyfile.ok_or(VaultError::NeedsFactor(Factor::KeyFile))?;
            h.update(b"|keyfile|");
            h.update(Sha256::digest(file));
        }
        if !self.factors.security_keys.is_empty() {
            let secret = secret.ok_or(VaultError::NeedsFactor(Factor::SecurityKey))?;
            h.update(b"|security_keys|");
            h.update(secret);
        }
        Ok(Zeroizing::new(h.finalize().into()))
    }

    /// The security-key secret, from whichever enrolled key is plugged in. A
    /// key that tells its serial is asked only its own question.
    fn security_secret(&self, devices: &[&dyn ChallengeResponse]) -> R<Zeroizing<[u8; 32]>> {
        if devices.is_empty() {
            return Err(VaultError::NeedsFactor(Factor::SecurityKey));
        }
        let mut failed = None;
        for device in devices {
            let serial = device.serial();
            let known = |k: &&SecurityKeyFactor| match (serial, k.serial) {
                (Some(a), Some(b)) => a == b,
                // A key that does not tell, or an entry that does not say:
                // worth asking.
                _ => true,
            };
            for k in self.factors.security_keys.iter().filter(known) {
                let answer = match device.respond(k.slot, &unhex(&k.challenge)?) {
                    Ok(a) => a,
                    Err(e) => {
                        failed = Some(e);
                        continue;
                    }
                };
                if let Some(plain) = open(&SecurityKeyFactor::sealing_key(&answer), &k.secret, &k.aad(&self.id))? {
                    let secret: [u8; 32] =
                        plain.as_slice().try_into().map_err(|_| VaultError::Corrupt("secret size".into()))?;
                    return Ok(Zeroizing::new(secret));
                }
            }
        }
        Err(failed.unwrap_or(VaultError::UnknownSecurityKey))
    }

    /// The wrapping key these credentials make: the key file checked first,
    /// then a security key asked, then the password's Argon2.
    fn key_for(&self, c: &Credentials) -> R<(Key32, Option<Key32>)> {
        if self.factors.keyfile && c.keyfile.is_none() {
            return Err(VaultError::NeedsFactor(Factor::KeyFile));
        }
        let secret = if self.factors.security_keys.is_empty() {
            None
        } else {
            Some(self.security_secret(c.security_keys)?)
        };
        let wk = self.wrapping_key(c.password, c.keyfile, secret.as_deref())?;
        Ok((wk, secret))
    }

    /// What a wrong set of credentials is called: a wrong password, or — when
    /// there is more than a password — a wrong key.
    fn wrong(&self) -> VaultError {
        if self.factors.is_empty() {
            VaultError::WrongPassword
        } else {
            VaultError::WrongKey
        }
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
                if e.format != FORMAT || !(1..=VERSION).contains(&e.version) {
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

    /// What, besides the password, this vault needs to open — for the GUI
    /// to ask for it. None when there is no vault.
    pub fn factors(&self) -> Option<&Factors> {
        self.envelope.as_ref().map(|e| &e.factors)
    }

    /// Make a new, empty vault with this key — the password, and whichever
    /// factors are given — and open it.
    pub fn create<'a>(&mut self, key: impl Into<NewKey<'a>>, kdf: Kdf) -> R<()> {
        // Over a file that is there — even one this vault could not read — never.
        if self.exists() || self.path.exists() {
            return Err(VaultError::Io("a vault already exists".into()));
        }
        let data_key = Zeroizing::new(random::<32>()?);
        self.open = Some(Open { key: data_key, secrets: Secrets::new() });
        let written = self.write_with_key(&key.into(), kdf, hex(&random::<16>()?), None);
        if written.is_err() {
            self.open = None;
        }
        written
    }

    /// Open it with the master password, and its factors if it has them.
    pub fn unlock<'a>(&mut self, credentials: impl Into<Credentials<'a>>) -> R<()> {
        let data_key = self.data_key(&credentials.into())?.0;
        // The credentials opened the key; if the key does not open the
        // secrets, the file was changed after it was written.
        self.open_with(data_key).map_err(|e| match e {
            VaultError::WrongPassword => VaultError::Corrupt("the secrets do not match their key".into()),
            e => e,
        })
    }

    /// The data key these credentials unwrap — and the security-key secret
    /// they yielded, if the vault has one.
    fn data_key(&self, c: &Credentials) -> R<(Key32, Option<Key32>)> {
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let (wk, secret) = e.key_for(c)?;
        let Some(key) = open(&wk, &e.key, &e.aad())? else {
            return Err(e.wrong());
        };
        let key: [u8; 32] = key.as_slice().try_into().map_err(|_| VaultError::Corrupt("key size".into()))?;
        Ok((Zeroizing::new(key), secret))
    }

    /// Whether these credentials open it — without opening, closing or
    /// changing anything. For asking again before showing a secret.
    pub fn check_password<'a>(&self, credentials: impl Into<Credentials<'a>>) -> R<bool> {
        match self.data_key(&credentials.into()) {
            Ok(_) => Ok(true),
            Err(VaultError::WrongPassword | VaultError::WrongKey | VaultError::UnknownSecurityKey) => Ok(false),
            Err(e) => Err(e),
        }
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

    /// A new master password, keeping no other factors. See [`Vault::change_key`].
    pub fn change_password(&mut self, old: &str, new: &str) -> R<()> {
        self.change_key(&Credentials::password(old), &NewKey::password(new))
    }

    /// A new key: the password, key file and security keys `new` gives, in
    /// place of those `old` opens it with — asked for even while it is open.
    /// Kept security keys keep working as they are; any other change to them
    /// draws a new secret. The data key stays, so a key remembered on this
    /// machine keeps working; a copy of the old file elsewhere still opens the
    /// old way — that copy is not this file.
    pub fn change_key(&mut self, old: &Credentials, new: &NewKey) -> R<()> {
        let (data_key, secret) = self.data_key(old)?;
        let e = self.envelope.as_ref().ok_or(VaultError::Missing)?;
        let (kdf, id) = (e.kdf, e.id.clone());
        let kept = match secret {
            Some(s) if new.keep_security_keys => Some((s, e.factors.security_keys.clone())),
            _ => None,
        };
        if self.open.is_none() {
            self.open_with(data_key)?;
        }
        self.write_with_key(new, kdf, id, kept)
    }

    /// Re-encrypt the secrets under the data key, keeping the wrapped key.
    fn write(&mut self) -> R<()> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        let mut e = self.envelope.clone().ok_or(VaultError::Missing)?;
        let plain = Zeroizing::new(serde_json::to_vec(&o.secrets).map_err(|e| VaultError::Io(e.to_string()))?);
        e.data = seal(&o.key, &plain, &e.aad())?;
        self.save(e)
    }

    /// Wrap the data key under `key` with a fresh salt, then write all. `kept`:
    /// the security-key secret and the keys holding it, to keep as they are.
    fn write_with_key(
        &mut self,
        key: &NewKey,
        kdf: Kdf,
        id: String,
        kept: Option<(Zeroizing<[u8; 32]>, Vec<SecurityKeyFactor>)>,
    ) -> R<()> {
        let o = self.open.as_ref().ok_or(VaultError::Locked)?;
        let salt = random::<16>()?;
        // The secret the security keys hold: kept, or new for keys added
        // afresh, or none without keys.
        let (secret, mut security_keys) = match kept {
            Some((s, keys)) => (Some(s), keys),
            None if key.add.is_empty() => (None, Vec::new()),
            None => (Some(Zeroizing::new(random::<32>()?)), Vec::new()),
        };
        if let Some(secret) = &secret {
            for added in key.add {
                if added.serial.is_some() && security_keys.iter().any(|k| k.serial == added.serial) {
                    return Err(VaultError::SecurityKey("that key is enrolled already".into()));
                }
                let mut entry = SecurityKeyFactor {
                    serial: added.serial,
                    name: added.name.clone(),
                    slot: added.slot,
                    challenge: hex(&added.challenge),
                    secret: Sealed { nonce: String::new(), ct: String::new() },
                };
                entry.secret = seal(&SecurityKeyFactor::sealing_key(&added.answer), secret.as_ref(), &entry.aad(&id))?;
                security_keys.push(entry);
            }
        }
        let mut e = Envelope {
            format: FORMAT.into(),
            version: VERSION,
            id,
            kdf,
            salt: hex(&salt),
            factors: Factors { keyfile: key.keyfile.is_some(), security_keys },
            key: Sealed { nonce: String::new(), ct: String::new() },
            data: Sealed { nonce: String::new(), ct: String::new() },
        };
        let wk = e.wrapping_key(key.password, key.keyfile, secret.as_deref())?;
        e.key = seal(&wk, o.key.as_ref(), &e.aad())?;
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
