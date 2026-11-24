//! The user's own OpenPGP key, through the GnuPG they have installed.
//!
//! Limen checks the team's signatures by itself (`limen-sign`); that needs
//! only public keys. Signing as the *user* needs their private key, which is
//! theirs to keep — in GnuPG's keyring, or on a smart card — and never
//! Limen's to hold. So signing is asked of `gpg`: it finds the key, and its
//! own pinentry asks for the passphrase or the card's PIN.
//!
//! Which key is the user's own is a setting (`gpg_key`, a fingerprint), chosen
//! on the OpenPGP page from the keys [`Gpg::secret_keys`] lists — a default,
//! since what signs can also be picked at the moment of signing. A key may
//! carry several user ids (a work address and a private one, say): which of
//! them a signature is made *as* is chosen too (`gpg_user_id`), and written
//! into the signature (`--sender`), so whoever checks it sees that one.
//!
//! GnuPG is looked for where it is installed. One that is not installed — a
//! portable copy on a stick, a build in a folder — is named by another setting
//! (`gpg_path`, the program itself), and then that one is used and no other.

use std::path::{Path, PathBuf};
use std::process::Command;

use limen_proto::NoConsole;

/// Why GnuPG could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpgError {
    /// There is no `gpg` on this machine.
    NotInstalled,
    /// The program named in the settings is not a working `gpg`: its path.
    NotGpg(String),
    /// No key has been chosen on the OpenPGP page.
    NoKeyChosen,
    /// The chosen key is not in the keyring any more (or its card is not in).
    KeyMissing(String),
    /// `gpg` said no: a cancelled passphrase, a wrong PIN, a file it cannot
    /// read. Its own words.
    Failed(String),
}

impl std::fmt::Display for GpgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpgError::NotInstalled => write!(f, "GnuPG is not installed"),
            GpgError::NotGpg(path) => write!(f, "{path} is not a working gpg program"),
            GpgError::NoKeyChosen => write!(f, "no OpenPGP key is chosen: pick yours on Limen's OpenPGP page"),
            GpgError::KeyMissing(k) => write!(f, "the chosen OpenPGP key ({k}) is not in the GnuPG keyring"),
            GpgError::Failed(why) => write!(f, "gpg: {why}"),
        }
    }
}

impl std::error::Error for GpgError {}

/// One of the user's keys: one whose private half GnuPG has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKey {
    /// The primary key's fingerprint, 40 hex digits.
    pub fingerprint: String,
    /// Its first user id: "Name <email>".
    pub user_id: String,
    /// All its user ids that are not revoked, the first first.
    pub user_ids: Vec<String>,
    /// It can sign now: it has a signing (sub)key, and is not expired, revoked
    /// or disabled.
    pub can_sign: bool,
    /// Its private half is on a smart card (a YubiKey), not on disk.
    pub on_card: bool,
}

/// A file signed: where the signature is, and whose it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    /// The detached, armored signature: the file's name with `.asc`.
    pub signature: PathBuf,
    pub fingerprint: String,
    pub user_id: String,
}

/// The installed GnuPG.
#[derive(Debug, Clone)]
pub struct Gpg {
    program: PathBuf,
    /// Another keyring than the user's own (tests).
    home: Option<PathBuf>,
}

/// Whether `program` runs and says it is GnuPG.
fn is_gpg(program: &Path) -> bool {
    Command::new(program)
        .arg("--version")
        .no_console()
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("GnuPG"))
}

impl Gpg {
    /// The `gpg` to use: the one at `custom` when a path is given — that one
    /// or none, never another found in its place — else the installed one.
    pub fn locate(custom: Option<&str>) -> Result<Self, GpgError> {
        match custom.map(str::trim).filter(|p| !p.is_empty()) {
            Some(path) => {
                let program = PathBuf::from(path);
                if program.is_file() && is_gpg(&program) {
                    Ok(Self { program, home: None })
                } else {
                    Err(GpgError::NotGpg(path.to_string()))
                }
            }
            None => Self::find(),
        }
    }

    /// The `gpg` the settings say to use (`gpg_path`), or the installed one.
    pub fn configured() -> Result<Self, GpgError> {
        let custom = crate::Config::load().ok().and_then(|c| c.gpg_path);
        Self::locate(custom.as_deref())
    }

    /// The program this is.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The `gpg` installed on this machine, if there is one.
    pub fn find() -> Result<Self, GpgError> {
        let mut candidates = vec![PathBuf::from("gpg"), PathBuf::from("gpg2")];
        if cfg!(windows) {
            // Gpg4win's own place, when it is not on the PATH.
            for base in ["ProgramFiles(x86)", "ProgramFiles"] {
                if let Ok(dir) = std::env::var(base) {
                    candidates.push(Path::new(&dir).join("GnuPG").join("bin").join("gpg.exe"));
                }
            }
        }
        for program in candidates {
            if is_gpg(&program) {
                return Ok(Self { program, home: None });
            }
        }
        Err(GpgError::NotInstalled)
    }

    /// Use the keyring in `home` rather than the user's own.
    #[doc(hidden)]
    pub fn with_home(mut self, home: &Path) -> Self {
        self.home = Some(home.to_path_buf());
        self
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        // No questions on a terminal that is not there: what gpg has to ask,
        // its pinentry asks in a window of its own.
        cmd.args(["--batch", "--no-tty"]);
        if let Some(home) = &self.home {
            cmd.arg("--homedir").arg(home);
        }
        cmd.no_console();
        cmd
    }

    /// The keys whose private half is here: the user's own.
    pub fn secret_keys(&self) -> Result<Vec<UserKey>, GpgError> {
        let out = self
            .command()
            .args(["--with-colons", "--fixed-list-mode", "--list-secret-keys"])
            .output()
            .map_err(|e| GpgError::Failed(e.to_string()))?;
        if !out.status.success() {
            return Err(GpgError::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        Ok(parse_secret_keys(&String::from_utf8_lossy(&out.stdout)))
    }

    /// The key `fingerprint` among the user's own: whose it is. What a
    /// signature by it will say, before one is made.
    pub fn key(&self, fingerprint: &str) -> Result<UserKey, GpgError> {
        let fingerprint: String = fingerprint.chars().filter(|c| !c.is_whitespace()).collect();
        if fingerprint.is_empty() {
            return Err(GpgError::NoKeyChosen);
        }
        self.secret_keys()?
            .into_iter()
            .find(|k| k.fingerprint.eq_ignore_ascii_case(&fingerprint))
            .ok_or(GpgError::KeyMissing(fingerprint))
    }

    /// Sign `file` with the key `fingerprint`: a detached, armored signature
    /// written beside it as `<file>.asc`, replacing one already there. GnuPG's
    /// pinentry asks for the passphrase or PIN if it needs one.
    ///
    /// `user_id`: which of the key's user ids to sign as — it has to be one of
    /// them. None: its first.
    pub fn sign_file(&self, fingerprint: &str, user_id: Option<&str>, file: &Path) -> Result<Signed, GpgError> {
        let fingerprint: String = fingerprint.chars().filter(|c| !c.is_whitespace()).collect();
        if fingerprint.is_empty() {
            return Err(GpgError::NoKeyChosen);
        }
        if !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GpgError::KeyMissing(fingerprint));
        }
        let key = self
            .secret_keys()?
            .into_iter()
            .find(|k| k.fingerprint.eq_ignore_ascii_case(&fingerprint))
            .ok_or_else(|| GpgError::KeyMissing(fingerprint.clone()))?;
        // Only a user id the key really has: it goes into the signature, and
        // onto gpg's command line.
        let user_id = match user_id.map(str::trim).filter(|u| !u.is_empty()) {
            Some(asked) => key
                .user_ids
                .iter()
                .find(|u| u.as_str() == asked)
                .cloned()
                .ok_or_else(|| GpgError::Failed(format!("the key has no user ID \"{asked}\"")))?,
            None => key.user_id.clone(),
        };
        if !file.is_file() {
            return Err(GpgError::Failed(format!("{} is not a file", file.display())));
        }
        let mut signature = file.as_os_str().to_owned();
        signature.push(".asc");
        let signature = PathBuf::from(signature);
        let mut cmd = self.command();
        // The key by its fingerprint — exactly that key, not one found by name.
        cmd.arg("--yes").arg("--local-user").arg(format!("{}!", primary_or_signing(&key)));
        // And which user id it signs as, when that is an address gpg can
        // record (it takes none without one).
        if user_id.contains('@') {
            cmd.arg("--sender").arg(&user_id);
        }
        let out = cmd
            .args(["--armor", "--detach-sign", "--output"])
            .arg(&signature)
            .arg(file)
            .output()
            .map_err(|e| GpgError::Failed(e.to_string()))?;
        if !out.status.success() {
            let said = String::from_utf8_lossy(&out.stderr);
            // gpg's last line is the one that says what went wrong.
            let why = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("signing failed");
            return Err(GpgError::Failed(why.trim().trim_start_matches("gpg: ").to_string()));
        }
        Ok(Signed { signature, fingerprint: key.fingerprint, user_id })
    }
}

/// What `--local-user` is given. The primary key's fingerprint: gpg picks the
/// signing subkey that belongs to it.
fn primary_or_signing(key: &UserKey) -> &str {
    &key.fingerprint
}

/// A field of a colon listing, with its `\x3a`-style escapes undone.
fn unescape(field: &str) -> String {
    let mut out = Vec::new();
    let bytes = field.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Read as bytes: a name may hold letters that are more than one.
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1] == b'x'
            && let Some(b) = std::str::from_utf8(&bytes[i + 2..i + 4]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read `gpg --with-colons --list-secret-keys`.
#[doc(hidden)]
pub fn parse_secret_keys(listing: &str) -> Vec<UserKey> {
    let mut keys: Vec<UserKey> = Vec::new();
    // Whether the lines being read belong to a primary key that still wants
    // its fingerprint (the first `fpr` after `sec`), and its first user id.
    let (mut want_fpr, mut want_uid) = (false, false);
    for line in listing.lines() {
        let f: Vec<&str> = line.split(':').collect();
        let field = |n: usize| f.get(n).copied().unwrap_or("");
        match field(0) {
            "sec" => {
                let validity = field(1);
                let caps = field(11);
                let usable = !validity.chars().any(|c| "erid".contains(c)) && !caps.contains('D');
                keys.push(UserKey {
                    fingerprint: String::new(),
                    user_id: String::new(),
                    user_ids: Vec::new(),
                    // Upper case: what the key can do with any of its subkeys.
                    can_sign: usable && caps.contains('S'),
                    // A serial number, or `#` for a stub: the secret is elsewhere.
                    on_card: !field(14).is_empty() && field(14) != "+",
                });
                (want_fpr, want_uid) = (true, true);
            }
            "ssb" => (want_fpr, want_uid) = (false, false),
            "fpr" if want_fpr => {
                if let Some(k) = keys.last_mut() {
                    k.fingerprint = field(9).to_uppercase();
                }
                want_fpr = false;
            }
            // Every user id of the key, but for revoked ones.
            "uid" if want_uid => {
                if let Some(k) = keys.last_mut()
                    && !field(1).contains('r')
                {
                    let id = unescape(field(9));
                    if k.user_ids.is_empty() {
                        k.user_id = id.clone();
                    }
                    k.user_ids.push(id);
                }
            }
            _ => {}
        }
    }
    keys.retain(|k| !k.fingerprint.is_empty());
    keys
}
