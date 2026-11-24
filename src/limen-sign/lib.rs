//! Whether what Limen is about to install was signed by the team.
//!
//! A release — of Limen, of a module — carries a detached OpenPGP signature.
//! Limen has the team's public keys built in ([`TEAM_KEYS`]), each pinned by
//! its fingerprint, and checks the signature here, with no GnuPG on the
//! machine: [`verify`] says [`Verdict::Valid`] and by whom, or why not.
//!
//! What the answers mean to the one installing:
//!
//! * [`Verdict::Valid`] — signed by one of the team's keys: install.
//! * [`Verdict::Bad`] — a team key's signature that does not match what was
//!   downloaded, or something that is no signature at all: refuse.
//! * [`Verdict::Unsigned`], [`Verdict::UnknownSigner`] — nothing from the team:
//!   say so, and let the user decide.
//!
//! **Keys change.** A key that is no longer signed with stays in the list as
//! [`KeyStatus::Retired`], and what it signed still verifies — an old module
//! needs no new release. A key's expiry date is not looked at for the same
//! reason: Limen trusts the keys in this list for as long as they are in it,
//! and they leave it only by a new version of Limen. A key that leaked is
//! taken out; what it signed then counts as from an unknown signer, until the
//! team signs it again — a signature file may hold several signatures, and one
//! good one from the team is enough.

use std::sync::OnceLock;
use std::time::SystemTime;

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::packet::Signature;
use pgp::types::KeyDetails;

/// Whether the team still signs with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// Releases are signed with it.
    Active,
    /// No longer signed with; what it signed is still the team's.
    Retired,
}

/// One of the team's keys, as built into Limen.
#[derive(Debug, Clone, Copy)]
pub struct TrustedKey {
    /// Who it is, for the screen: "CRC-BARRACUDA".
    pub name: &'static str,
    /// The primary key's fingerprint: 40 hex digits, no spaces. The key is
    /// trusted by this — its armor is only where the key material comes from.
    pub fingerprint: &'static str,
    /// The public key, ASCII-armored.
    pub armored: &'static str,
    pub status: KeyStatus,
}

/// The team's keys. Changed only by a release of Limen.
pub const TEAM_KEYS: &[TrustedKey] = &[TrustedKey {
    name: "CRC-BARRACUDA",
    fingerprint: "8D151FD231907502BBFA528E140ABA21244518B8",
    armored: include_str!("../../resources/keys/crc-barracuda.asc"),
    status: KeyStatus::Active,
}];

/// Who signed, when a signature is the team's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    /// The key's name in the list.
    pub name: String,
    /// The primary key's fingerprint, 40 hex digits.
    pub fingerprint: String,
    /// When the signature says it was made.
    pub signed_at: Option<SystemTime>,
    /// The key is one the team no longer signs with.
    pub retired: bool,
}

/// What a signature check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Signed by one of the team's keys, and it matches.
    Valid(Signer),
    /// There is no signature.
    Unsigned,
    /// Signed, but by no key of the team's: `key` is the signer's id.
    UnknownSigner { key: String },
    /// A team key's signature that does not match, or not a signature: why.
    Bad(String),
}

impl Verdict {
    pub fn is_valid(&self) -> bool {
        matches!(self, Verdict::Valid(_))
    }

    /// Whether this must stop an install outright, whatever the user says.
    pub fn refuses(&self) -> bool {
        matches!(self, Verdict::Bad(_))
    }
}

/// One trusted key, as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub name: String,
    /// 40 hex digits.
    pub fingerprint: String,
    /// The key's own user ids: "CRC-BARRACUDA <security@…>".
    pub user_ids: Vec<String>,
    pub status: KeyStatus,
}

/// A key read and checked, ready to verify with.
struct Loaded {
    name: String,
    fingerprint: String,
    retired: bool,
    key: SignedPublicKey,
}

/// A set of trusted keys.
pub struct Keyring {
    keys: Vec<Loaded>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

impl Keyring {
    /// Read `keys`. Each has to be a public key whose fingerprint is the one
    /// pinned for it, and whose own signatures hold.
    pub fn from_keys(keys: &[TrustedKey]) -> Result<Self, String> {
        let mut loaded = Vec::new();
        for k in keys {
            let (key, _) = SignedPublicKey::from_string(k.armored)
                .map_err(|e| format!("{}: not a public key: {e}", k.name))?;
            let fingerprint = hex(key.fingerprint().as_bytes());
            let pinned: String = k.fingerprint.chars().filter(|c| !c.is_whitespace()).collect();
            if !fingerprint.eq_ignore_ascii_case(&pinned) {
                return Err(format!("{}: the key is {fingerprint}, not the pinned {pinned}", k.name));
            }
            key.verify_bindings().map_err(|e| format!("{}: the key's own signatures do not hold: {e}", k.name))?;
            loaded.push(Loaded {
                name: k.name.to_string(),
                fingerprint,
                retired: k.status == KeyStatus::Retired,
                key,
            });
        }
        Ok(Self { keys: loaded })
    }

    /// The keys in this ring, in the order they were given.
    pub fn keys(&self) -> Vec<KeyInfo> {
        self.keys
            .iter()
            .map(|k| KeyInfo {
                name: k.name.clone(),
                fingerprint: k.fingerprint.clone(),
                user_ids: k.key.details.users.iter().filter_map(|u| u.id.as_str().map(str::to_string)).collect(),
                status: if k.retired { KeyStatus::Retired } else { KeyStatus::Active },
            })
            .collect()
    }

    /// The team's keys, as built in.
    pub fn team() -> &'static Keyring {
        static TEAM: OnceLock<Keyring> = OnceLock::new();
        // The built-in list is checked by a test: it reads.
        TEAM.get_or_init(|| Keyring::from_keys(TEAM_KEYS).expect("the built-in team keys read"))
    }

    /// Check `content` against a detached signature — armored or binary, one
    /// signature or several. `None`: there was no signature to fetch.
    pub fn verify(&self, content: &[u8], signature: Option<&[u8]>) -> Verdict {
        let Some(signature) = signature else {
            return Verdict::Unsigned;
        };
        let signatures = match read_signatures(signature) {
            Ok(s) if s.is_empty() => return Verdict::Bad("the signature file holds no signature".into()),
            Ok(s) => s,
            Err(why) => return Verdict::Bad(format!("the signature file cannot be read: {why}")),
        };
        let mut bad = None;
        let mut unknown = None;
        for sig in &signatures {
            match self.check(content, sig) {
                Checked::Valid(signer) => return Verdict::Valid(signer),
                Checked::Bad(why) => bad = bad.or(Some(why)),
                Checked::Unknown(key) => unknown = unknown.or(Some(key)),
            }
        }
        match (bad, unknown) {
            (Some(why), _) => Verdict::Bad(why),
            (None, Some(key)) => Verdict::UnknownSigner { key },
            (None, None) => Verdict::Bad("the signature file holds no signature".into()),
        }
    }

    fn check(&self, content: &[u8], sig: &Signature) -> Checked {
        let issuers: Vec<String> = sig.issuer_fingerprint().iter().map(|f| hex(f.as_bytes())).collect();
        let key_ids: Vec<String> = sig.issuer_key_id().iter().map(|k| hex(k.as_ref())).collect();
        // Says who it is by: whether a key (or subkey) of ours is that one.
        // A signature that names nobody is tried against every key.
        let named = !issuers.is_empty() || !key_ids.is_empty();
        let is_by = |fingerprint: &str| {
            !named || issuers.iter().any(|i| i == fingerprint) || key_ids.iter().any(|k| fingerprint.ends_with(k.as_str()))
        };
        let mut by_ours = None;
        for k in &self.keys {
            let detached = DetachedSignature::new(sig.clone());
            let mut verified = None;
            if is_by(&k.fingerprint) {
                verified = Some(detached.verify(&k.key, content));
            }
            for sub in &k.key.public_subkeys {
                if verified.as_ref().is_some_and(|v| v.is_ok()) {
                    break;
                }
                if is_by(&hex(sub.fingerprint().as_bytes())) && sub.verify_bindings(&k.key.primary_key).is_ok() {
                    verified = Some(detached.verify(sub, content));
                }
            }
            match verified {
                Some(Ok(())) => {
                    if let Some(weak) = weak_hash(sig) {
                        return Checked::Bad(format!("{}'s signature uses {weak}, which is not trusted", k.name));
                    }
                    return Checked::Valid(Signer {
                        name: k.name.clone(),
                        fingerprint: k.fingerprint.clone(),
                        signed_at: sig.created().map(SystemTime::from),
                        retired: k.retired,
                    });
                }
                Some(Err(_)) if named => by_ours = Some(k.name.clone()),
                _ => {}
            }
        }
        match by_ours {
            Some(name) => Checked::Bad(format!("{name}'s signature does not match what was downloaded")),
            None => Checked::Unknown(issuers.into_iter().next().or(key_ids.into_iter().next()).unwrap_or_else(|| "unnamed".into())),
        }
    }
}

enum Checked {
    Valid(Signer),
    Bad(String),
    Unknown(String),
}

/// A hash no signature should rest on any more.
fn weak_hash(sig: &Signature) -> Option<&'static str> {
    match sig.hash_alg() {
        Some(HashAlgorithm::Md5) => Some("MD5"),
        Some(HashAlgorithm::Sha1) => Some("SHA-1"),
        Some(HashAlgorithm::Ripemd160) => Some("RIPEMD-160"),
        _ => None,
    }
}

const ARMOR_BEGIN: &str = "-----BEGIN PGP SIGNATURE-----";

/// Every signature in a signature file: armored (one block or several, one
/// after the other) or binary.
fn read_signatures(bytes: &[u8]) -> Result<Vec<Signature>, String> {
    let mut out = Vec::new();
    let armored = std::str::from_utf8(bytes).ok().filter(|t| t.trim_start().starts_with(ARMOR_BEGIN));
    if let Some(text) = armored {
        for block in text.split(ARMOR_BEGIN).skip(1) {
            let block = format!("{ARMOR_BEGIN}{block}");
            let (many, _) = DetachedSignature::from_string_many(&block).map_err(|e| e.to_string())?;
            for sig in many {
                out.push(sig.map_err(|e| e.to_string())?.signature);
            }
        }
    } else {
        for sig in DetachedSignature::from_bytes_many(bytes).map_err(|e| e.to_string())? {
            out.push(sig.map_err(|e| e.to_string())?.signature);
        }
    }
    Ok(out)
}

/// Check `content` against `signature` with the team's keys.
pub fn verify(content: &[u8], signature: Option<&[u8]>) -> Verdict {
    Keyring::team().verify(content, signature)
}
