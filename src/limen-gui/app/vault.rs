//! Limen's vault, as the GUI holds it: shared with the host (modules reach it
//! through `host.secret_*`), the window that creates or unlocks it, the lock
//! after 15 idle minutes, the Vault section of Settings and the padlock in the
//! title bar.
//!
//! The vault is behind one mutex. Everything slow — Argon2 (half a second on
//! purpose), the keychain — runs on a thread; the window shows a spinner.
//!
//! A module that finds the vault locked (or not made yet) does not fail at
//! once: the window opens and its call waits. Unlocked (or made), the call
//! goes on as if it had been open all along, so the action the user tapped is
//! done without tapping it again. A wrong password leaves the window up and
//! the call waiting; turning the window down — or [`WAIT_FOR`] passing —
//! fails it, as locked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use limen_core::security_key::{self, YubiKey};
use limen_core::vault::{
    self, ChallengeResponse, Credentials, Enrolled, Factor, Factors, Kdf, NewKey, Vault, VaultError, LIMEN,
};
use zeroize::Zeroizing;
use limen_host::{SecretError, SecretStore};
use limen_ui as ui;

use crate::i18n;

/// How long the window shows that it worked before it closes.
const SUCCESS_S: f64 = 0.75;
/// How long the padlock in it takes to swing open.
const SWING_S: f64 = 0.45;
/// How long a wrong password shakes the field.
const SHAKE_S: f64 = 0.4;
/// How long a revealed secret stays on screen.
const REVEAL_FOR: Duration = Duration::from_secs(30);
/// A master password's least length.
const MIN_PASSWORD: usize = 8;
/// How long a module's call waits for the vault to be opened before it fails.
const WAIT_FOR: Duration = Duration::from_secs(5 * 60);
/// How often a waiting call looks again, should a wake-up have been missed.
const RECHECK: Duration = Duration::from_millis(250);

/// What the GUI and the host share.
struct Shared {
    vault: Mutex<Vault>,
    /// The vault file is there but could not be read: why. Nothing is made or
    /// written over it; the Vault page says so.
    damaged: Option<String>,
    /// When it was last used — what the auto-lock counts from.
    last_used: Mutex<Instant>,
    /// A module found it locked (or missing): who, for the window to say.
    asked_by: Mutex<Option<String>>,
    /// How many times the user turned the window down. A call waiting for the
    /// vault gives up when this moves.
    declined: Mutex<u64>,
    /// Woken when the vault may have opened, or the window was turned down.
    woken: Condvar,
    /// How long a call waits: [`WAIT_FOR`], shorter in tests.
    wait_for: Duration,
    ctx: egui::Context,
}

impl Shared {
    fn vault(&self) -> std::sync::MutexGuard<'_, Vault> {
        self.vault.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn ask(&self, who: &str) {
        *self.asked_by.lock().unwrap_or_else(|e| e.into_inner()) = Some(who.to_string());
        self.ctx.request_repaint();
    }

    fn declines(&self) -> u64 {
        *self.declined.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The user turned the window down: every waiting call fails.
    fn decline(&self) {
        *self.declined.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.woken.notify_all();
    }

    /// The vault may have opened: waiting calls look again.
    fn wake(&self) {
        self.woken.notify_all();
    }

    /// Wait until the vault is open: true; or until the user turns the window
    /// down after `declined` was read, or `wait_for` passes: false. Never
    /// holds the vault while waiting — the unlock needs it.
    fn wait_open(&self, declined: u64) -> bool {
        let until = Instant::now() + self.wait_for;
        loop {
            if self.vault().is_open() {
                return true;
            }
            let seen = self.declined.lock().unwrap_or_else(|e| e.into_inner());
            if *seen != declined {
                return false;
            }
            let Some(left) = until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else {
                return false;
            };
            let _ = self.woken.wait_timeout(seen, left.min(RECHECK));
        }
    }
}

/// The vault as the host sees it: one compartment per module, opened quietly
/// with this machine's key when it can be, and asking the user when not.
struct VaultStore(Arc<Shared>);

impl VaultStore {
    /// The vault, open — or why not. Locked or not made yet, the user is asked
    /// and the call waits for their answer: opened, it goes on.
    fn with<T>(&self, module: &str, f: impl FnOnce(&mut Vault) -> Result<T, VaultError>) -> Result<T, SecretError> {
        if let Some(why) = &self.0.damaged {
            return Err(SecretError::Failed(format!("Limen's vault is damaged: {why}")));
        }
        let mut v = self.0.vault();
        // Locked after a while idle, but this machine remembers the key: open
        // again without asking — the lock wiped the key from memory, no more.
        if !v.is_open() && !(v.exists() && v.unlock_remembered().unwrap_or(false)) {
            let missing = !v.exists();
            drop(v);
            // Read before asking, so a "no" given at once is not missed.
            let declined = self.0.declines();
            self.0.ask(module);
            if !self.0.wait_open(declined) {
                return Err(if missing { SecretError::NoVault } else { SecretError::Locked });
            }
            v = self.0.vault();
            if !v.is_open() {
                // Locked again in the moment between: as good as a no.
                return Err(SecretError::Locked);
            }
        }
        self.0.touch();
        f(&mut v).map_err(|e| match e {
            VaultError::Locked => SecretError::Locked,
            VaultError::Missing => SecretError::NoVault,
            e => SecretError::Failed(e.to_string()),
        })
    }
}

impl SecretStore for VaultStore {
    fn get(&self, module: &str, name: &str) -> Result<Option<String>, SecretError> {
        self.with(module, |v| v.get(&vault::module_ns(module), name))
    }
    fn set(&self, module: &str, name: &str, value: &str) -> Result<(), SecretError> {
        self.with(module, |v| v.set(&vault::module_ns(module), name, value))
    }
    fn delete(&self, module: &str, name: &str) -> Result<bool, SecretError> {
        self.with(module, |v| v.delete(&vault::module_ns(module), name))
    }
    fn names(&self, module: &str) -> Result<Vec<String>, SecretError> {
        self.with(module, |v| v.names(&vault::module_ns(module)))
    }
}

/// What the window is for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Create,
    Unlock,
    ChangePassword,
    /// Ask the password again, then show this secret.
    Reveal { ns: String, name: String },
}

/// The most a key file may be: any file can be one, but a film is a mistake.
const KEYFILE_MAX: u64 = 16 * 1024 * 1024;

/// What the window's fields stand for, gathered off the GUI thread: the key
/// file read, the YubiKeys plugged in looked for.
struct Gathered {
    password: Zeroizing<String>,
    keyfile: Option<Zeroizing<Vec<u8>>>,
    keys: Vec<YubiKey>,
}

impl Gathered {
    /// `keyfile`: the file to read, if one is needed. `keys`: whether the
    /// YubiKeys plugged in are needed.
    fn get(password: String, keyfile: Option<&str>, keys: bool) -> Result<Self, String> {
        let keyfile = match keyfile {
            Some(path) => Some(read_keyfile(path)?),
            None => None,
        };
        let keys = if keys {
            match security_key::find_all() {
                Ok(k) if k.is_empty() => return Err(i18n::t("vault.plug_in")),
                Ok(k) => k,
                Err(e) => return Err(said(e)),
            }
        } else {
            Vec::new()
        };
        Ok(Self { password: Zeroizing::new(password), keyfile, keys })
    }

    /// Run `f` with these as credentials to open the vault with.
    fn opening<T>(&self, f: impl FnOnce(Credentials) -> T) -> T {
        let keys: Vec<&dyn ChallengeResponse> = self.keys.iter().map(|k| k as &dyn ChallengeResponse).collect();
        f(Credentials { password: &self.password, keyfile: self.keyfile.as_ref().map(|k| k.as_slice()), security_keys: &keys })
    }

    /// These as a new key: keeping the vault's YubiKeys or not, adding `add`.
    fn new_key<'a>(&'a self, keep: bool, add: &'a [Enrolled]) -> NewKey<'a> {
        NewKey {
            password: &self.password,
            keyfile: self.keyfile.as_ref().map(|k| k.as_slice()),
            keep_security_keys: keep,
            add,
        }
    }
}

fn read_keyfile(path: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let unreadable = |why: String| i18n::t("vault.keyfile_unreadable").replace("{why}", &why);
    let path = path.trim();
    if path.is_empty() {
        return Err(i18n::t("vault.no_keyfile"));
    }
    let size = std::fs::metadata(path).map_err(|e| unreadable(e.to_string()))?.len();
    if size > KEYFILE_MAX {
        return Err(unreadable(i18n::t("vault.keyfile_too_big")));
    }
    std::fs::read(path).map(Zeroizing::new).map_err(|e| unreadable(e.to_string()))
}

/// A vault error in the window's words.
fn said(e: VaultError) -> String {
    match e {
        VaultError::WrongPassword => i18n::t("vault.wrong"),
        VaultError::WrongKey => i18n::t("vault.wrong_key"),
        VaultError::NeedsFactor(Factor::KeyFile) => i18n::t("vault.no_keyfile"),
        VaultError::NeedsFactor(Factor::SecurityKey) => i18n::t("vault.plug_in"),
        VaultError::SecurityKey(why) => i18n::t("vault.key_failed").replace("{why}", &why),
        VaultError::UnknownSecurityKey => i18n::t("vault.unknown_key"),
        e => e.to_string(),
    }
}

/// Keep where the key file is for next time — or forget it, with none.
fn remember_keyfile(path: Option<&str>) {
    if let Ok(mut cfg) = limen_core::Config::load() {
        cfg.vault_keyfile = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        let _ = cfg.save();
    }
}

/// What "unlocks with" says: the password, and each factor.
fn unlocks_with(f: &Factors) -> String {
    let mut parts = vec![i18n::t("vault.f_password")];
    if f.keyfile {
        parts.push(i18n::t("vault.f_keyfile"));
    }
    match f.security_keys.len() {
        0 => {}
        1 => parts.push(i18n::t("vault.f_key")),
        n => parts.push(i18n::t("vault.f_keys").replace("{n}", &n.to_string())),
    }
    i18n::t("vault.unlocks_with").replace("{what}", &parts.join(" · "))
}

/// Which key-file field a file dialog fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyField {
    /// The one the vault needs now (unlock, reveal), or will (create, change).
    New,
    /// The one it needs until the change (change).
    Current,
}

/// A file dialog's answer: the path picked, or why a new key file was not made.
type Picked = (KeyField, Result<String, String>);

/// Ask for a key file on a thread of its own — the dialog blocks until it is
/// answered. `make`: choose where a new one goes, and write it there; never
/// over a file that is there, which may be another vault's key.
fn pick_keyfile(field: KeyField, make: bool, start: String, tx: mpsc::Sender<Picked>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new();
        if let Some(dir) = std::path::Path::new(&start).parent().filter(|p| p.is_dir()) {
            dlg = dlg.set_directory(dir);
        }
        let answer = if make {
            let Some(path) = dlg.set_file_name("limen.key").save_file() else { return };
            make_keyfile(&path).map(|()| path.display().to_string())
        } else {
            let Some(path) = dlg.pick_file() else { return };
            Ok(path.display().to_string())
        };
        let _ = tx.send((field, answer));
        ctx.request_repaint();
    });
}

fn make_keyfile(path: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    let bytes = vault::new_keyfile().map_err(|e| e.to_string())?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => i18n::t("vault.keyfile_exists"),
        _ => e.to_string(),
    })?;
    f.write_all(&bytes).and_then(|()| f.sync_all()).map_err(|e| e.to_string())
}

/// A finished piece of slow work.
enum Done {
    /// Opened (made, unlocked, password changed): close the window.
    Opened,
    /// The password was right: show this.
    Revealed { ns: String, name: String, value: String },
    Failed(String),
    /// Whether this machine remembers the key now.
    Remembered(bool),
    /// A YubiKey added in the window — or why not.
    Enrolled(Result<Enrolled, String>),
}

/// The vault's GUI.
pub(crate) struct VaultUi {
    shared: Arc<Shared>,
    mode: Option<Mode>,
    /// The window's fields.
    password: String,
    new_password: String,
    confirm: String,
    remember: bool,
    error: Option<String>,
    /// Slow work under way.
    working: Arc<AtomicBool>,
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    /// What was asked for, for the window's message.
    asked_by: Option<String>,
    /// A secret on show, until when.
    revealed: Option<(String, String, String, Instant)>,
    /// What "remember on this machine" is, as last read.
    remembered: bool,
    /// Lock after this long unused; None = never.
    lock_after: Option<Duration>,
    /// A secret waiting for "delete?" to be answered: (ns, name).
    deleting: Option<(String, String)>,
    deleting_shown: Option<(String, String)>,
    /// (exists, open) as last seen — what the GUI shows while slow work holds
    /// the vault, rather than waiting on it.
    known: (bool, bool),
    /// The key file the vault needs (unlock, reveal), or will (create, change).
    keyfile: String,
    /// The key file it needs until a change goes through.
    old_keyfile: String,
    /// Create and change: require a key file, a YubiKey, and the YubiKey's slot.
    use_keyfile: bool,
    /// Change: keep the YubiKeys the vault has.
    keep_keys: bool,
    /// YubiKeys added in this window, each asked its own challenge; and the
    /// slot the next one is added on.
    enrolled: Vec<Enrolled>,
    key_slot: u8,
    /// A YubiKey is being added.
    enrolling: bool,
    /// What the vault needs besides the password, as when the window opened.
    factors: Factors,
    /// File dialogs' answers.
    picks: (mpsc::Sender<Picked>, mpsc::Receiver<Picked>),
    /// It worked, at this time: the window shows so, then closes.
    succeeded_at: Option<f64>,
    /// A wrong password, at this time: the field shakes.
    shake_at: Option<f64>,
    /// The window is fading out. What it shows stays as it was until it has
    /// gone: cleared the moment it starts to close, it would fade out as the
    /// empty unlock form instead.
    closing: bool,
}

fn lock_after_from(minutes: Option<u32>) -> Option<Duration> {
    match minutes.unwrap_or(15) {
        0 => None,
        m => Some(Duration::from_secs(m as u64 * 60)),
    }
}

impl VaultUi {
    /// Load the vault, plug it into the host, and open it with this machine's
    /// key if it remembers one.
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let cfg = limen_core::Config::load().unwrap_or_default();
        // A damaged file is left where it is, untouched: Limen runs on with no
        // vault it can use, and says so. (A vault at a path where no file is
        // stands in; it can never be created, since `create` refuses wherever
        // a file is — and `damaged` keeps the GUI from offering it.)
        let (vault, damaged) = match Vault::default_vault() {
            Ok(v) => (v, None),
            Err(e) => {
                let stand_in = limen_core::paths::vault_path().with_extension("damaged-stand-in");
                (Vault::load(&stand_in, Box::new(vault::OsKeyStore)).expect("no file there to read"), Some(e.to_string()))
            }
        };
        let shared = Arc::new(Shared {
            vault: Mutex::new(vault),
            damaged,
            last_used: Mutex::new(Instant::now()),
            asked_by: Mutex::new(None),
            declined: Mutex::new(0),
            woken: Condvar::new(),
            wait_for: WAIT_FOR,
            ctx: ctx.clone(),
        });
        limen_host::set_secret_store(Some(Arc::new(VaultStore(shared.clone()))));
        let (tx, rx) = mpsc::channel();
        let me = Self {
            shared,
            mode: None,
            password: String::new(),
            new_password: String::new(),
            confirm: String::new(),
            remember: false,
            error: None,
            working: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            asked_by: None,
            revealed: None,
            remembered: false,
            lock_after: lock_after_from(cfg.vault_lock_minutes),
            deleting: None,
            deleting_shown: None,
            known: (false, false),
            keyfile: String::new(),
            old_keyfile: String::new(),
            use_keyfile: false,
            keep_keys: false,
            enrolled: Vec::new(),
            key_slot: 2,
            enrolling: false,
            factors: Factors::default(),
            picks: mpsc::channel(),
            succeeded_at: None,
            shake_at: None,
            closing: false,
        };
        // The keychain is a D-Bus call on Linux: off the GUI thread.
        me.spawn(|v| {
            if v.exists() {
                let _ = v.unlock_remembered();
            }
            Done::Remembered(v.remembered())
        });
        me
    }

    fn spawn(&self, work: impl FnOnce(&mut Vault) -> Done + Send + 'static) {
        let shared = self.shared.clone();
        let tx = self.tx.clone();
        let working = self.working.clone();
        working.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let done = work(&mut shared.vault());
            working.store(false, Ordering::Relaxed);
            let _ = tx.send(done);
            // Made or unlocked: the calls waiting for it go on now.
            shared.wake();
            shared.ctx.request_repaint();
        });
    }

    pub(crate) fn exists(&self) -> bool {
        self.shared.vault().exists()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.shared.vault().is_open()
    }

    fn busy(&self) -> bool {
        self.working.load(Ordering::Relaxed)
    }

    /// (exists, open), never waiting: while Argon2 or the keychain holds the
    /// vault, what it was last.
    fn peek(&mut self) -> (bool, bool) {
        if let Ok(v) = self.shared.vault.try_lock() {
            self.known = (v.exists(), v.is_open());
        }
        self.known
    }

    fn open_window(&mut self, mode: Mode) {
        self.password.clear();
        self.new_password.clear();
        self.confirm.clear();
        self.error = None;
        self.remember = self.remembered;
        self.succeeded_at = None;
        self.shake_at = None;
        self.closing = false;
        // What it needs now, and where its key file was last.
        self.factors = self.shared.vault().factors().cloned().unwrap_or_default();
        let last = limen_core::Config::load().ok().and_then(|c| c.vault_keyfile).unwrap_or_default();
        self.keyfile = last.clone();
        self.old_keyfile = last;
        self.use_keyfile = self.factors.keyfile;
        self.keep_keys = !self.factors.security_keys.is_empty();
        self.enrolled.clear();
        self.enrolling = false;
        self.key_slot = self.factors.security_keys.first().map_or(2, |k| k.slot);
        self.mode = Some(mode);
    }

    /// Ask for the password: to make the vault, or to open it.
    pub(crate) fn ask_to_open(&mut self) {
        if self.shared.damaged.is_some() {
            return;
        }
        let mode = if self.exists() { Mode::Unlock } else { Mode::Create };
        self.open_window(mode);
    }

    /// Lock it now.
    pub(crate) fn lock(&mut self) {
        self.shared.vault().lock();
        self.revealed = None;
    }

    /// A module was removed: its compartment goes with it — if the vault is
    /// open. A locked vault keeps it, and the Vault page shows it as an
    /// orphan to remove by hand.
    pub(crate) fn forget_module(&mut self, module: &str) {
        let mut v = self.shared.vault();
        if v.is_open() {
            let _ = v.remove_ns(&vault::module_ns(module));
        }
    }

    /// Limen's own secret, when the vault is open: the GitHub token, say.
    pub(crate) fn limen_secret(&self, name: &str) -> Option<String> {
        self.shared.vault().get(LIMEN, name).ok().flatten()
    }

    /// Keep one of Limen's own secrets. `false` if the vault is not open.
    pub(crate) fn set_limen_secret(&self, name: &str, value: Option<&str>) -> bool {
        let mut v = self.shared.vault();
        match value {
            Some(val) => v.set(LIMEN, name, val).is_ok(),
            None => v.delete(LIMEN, name).is_ok(),
        }
    }

    /// Once a frame: take in finished work, lock when idle, and answer a
    /// module that found the vault shut.
    pub(crate) fn frame(&mut self, ctx: &egui::Context, bounds: Option<egui::Rect>) -> bool {
        let mut just_opened = false;
        let now = ctx.input(|i| i.time);
        while let Ok((which, answer)) = self.picks.1.try_recv() {
            match answer {
                Ok(path) => match which {
                    KeyField::New => self.keyfile = path,
                    KeyField::Current => self.old_keyfile = path,
                },
                Err(why) => self.error = Some(why),
            }
        }
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::Opened => {
                    // Shown for a moment — the padlock swings open — then closed.
                    self.succeeded_at = Some(now);
                    self.shared.touch();
                    just_opened = true;
                    self.zero_fields();
                }
                Done::Revealed { ns, name, value } => {
                    self.mode = None;
                    self.zero_fields();
                    self.revealed = Some((ns, name, value, Instant::now()));
                }
                Done::Failed(why) => {
                    if why == i18n::t("vault.wrong") || why == i18n::t("vault.wrong_key") {
                        self.shake_at = Some(now);
                    }
                    self.error = Some(why);
                }
                Done::Remembered(on) => {
                    self.remembered = on;
                    just_opened |= self.is_open();
                }
                Done::Enrolled(answer) => {
                    self.enrolling = false;
                    match answer {
                        Ok(key) => self.enrolled.push(key),
                        Err(why) => self.error = Some(why),
                    }
                }
            }
        }
        if let Some(at) = self.succeeded_at
            && !self.closing
        {
            if now - at >= SUCCESS_S {
                // Fade out still showing that it worked; let go once gone.
                self.closing = true;
            } else {
                ctx.request_repaint();
            }
        }
        // Lock once unused for long enough; wake to do it even if nothing else
        // moves on screen. Never waiting for the vault: busy, it is not idle.
        if let Some(after) = self.lock_after {
            let idle = self.shared.last_used.lock().map(|t| t.elapsed()).unwrap_or_default();
            if let Ok(mut v) = self.shared.vault.try_lock()
                && v.is_open()
                && !self.busy()
            {
                if idle >= after {
                    v.lock();
                    self.revealed = None;
                } else {
                    ctx.request_repaint_after(after - idle);
                }
            }
        }
        if let Some((_, _, _, at)) = &self.revealed {
            if at.elapsed() >= REVEAL_FOR {
                self.revealed = None;
            } else {
                ctx.request_repaint_after(REVEAL_FOR - at.elapsed());
            }
        }
        let asked = self.shared.asked_by.lock().ok().and_then(|mut a| a.take());
        // A window already up is answer enough.
        if let Some(who) = asked
            && (self.mode.is_none() || self.closing)
        {
            self.asked_by = Some(who);
            self.ask_to_open();
        }
        self.window(ctx, bounds);
        self.delete_dialog(ctx, bounds);
        just_opened
    }

    fn zero_fields(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        self.new_password.zeroize();
        self.confirm.zeroize();
        // The keys added here hold their answers: wiped as they go.
        self.enrolled.clear();
    }

    /// What the vault needs now: its key file if it has one, and whether its
    /// YubiKeys.
    fn needed(&self) -> (Option<String>, bool) {
        (self.factors.keyfile.then(|| self.keyfile.clone()), !self.factors.security_keys.is_empty())
    }

    /// The key file a new key will need, if any.
    fn chosen_keyfile(&self) -> Result<Option<String>, String> {
        if self.use_keyfile && self.keyfile.trim().is_empty() {
            return Err(i18n::t("vault.no_keyfile"));
        }
        Ok(self.use_keyfile.then(|| self.keyfile.clone()))
    }

    /// Add the YubiKey plugged in: ask it a new challenge, on its own thread
    /// — it may wait for a touch. One already added (or kept) is passed over,
    /// so the backup is added by swapping it in and asking again.
    fn add_key(&mut self) {
        let taken: Vec<u32> = self
            .enrolled
            .iter()
            .filter_map(|e| e.serial)
            .chain(self.keep_keys.then(|| self.factors.security_keys.iter().filter_map(|k| k.serial)).into_iter().flatten())
            .collect();
        let slot = self.key_slot;
        let tx = self.tx.clone();
        let ctx = self.shared.ctx.clone();
        self.enrolling = true;
        self.error = None;
        std::thread::spawn(move || {
            let answer = match security_key::find_all() {
                Err(e) => Err(said(e)),
                Ok(keys) if keys.is_empty() => Err(i18n::t("vault.plug_in")),
                Ok(keys) => match keys.into_iter().find(|k| k.serial.is_none_or(|s| !taken.contains(&s))) {
                    None => Err(i18n::t("vault.key_added_already")),
                    Some(key) => vault::enroll(&key, slot).map_err(said),
                },
            };
            let _ = tx.send(Done::Enrolled(answer));
            ctx.request_repaint();
        });
    }

    fn submit(&mut self) {
        let Some(mode) = self.mode.clone() else { return };
        let pw = std::mem::take(&mut self.password);
        let remember = self.remember;
        let new_password = |me: &mut Self, keep: Option<&str>| -> Option<String> {
            let new = std::mem::take(&mut me.new_password);
            // Change: an empty one keeps the current password.
            if let Some(current) = keep
                && new.is_empty()
                && me.confirm.is_empty()
            {
                return Some(current.to_string());
            }
            if new.chars().count() < MIN_PASSWORD {
                me.error = Some(i18n::t("vault.too_short").replace("{n}", &MIN_PASSWORD.to_string()));
                return None;
            }
            if new != me.confirm {
                me.error = Some(i18n::t("vault.mismatch"));
                return None;
            }
            Some(new)
        };
        match mode {
            Mode::Create => {
                self.new_password = pw;
                let Some(pw) = new_password(self, None) else { return };
                let keyfile = match self.chosen_keyfile() {
                    Ok(k) => k,
                    Err(why) => return self.error = Some(why),
                };
                let add = self.enrolled.clone();
                self.spawn(move |v| {
                    let g = match Gathered::get(pw, keyfile.as_deref(), false) {
                        Ok(g) => g,
                        Err(why) => return Done::Failed(why),
                    };
                    match v.create(g.new_key(false, &add), Kdf::DEFAULT).and_then(|_| v.remember(remember)) {
                        Ok(()) => {
                            remember_keyfile(keyfile.as_deref());
                            Done::Opened
                        }
                        Err(e) => Done::Failed(said(e)),
                    }
                });
            }
            Mode::Unlock => {
                let (keyfile, keys) = self.needed();
                self.spawn(move |v| {
                    let g = match Gathered::get(pw, keyfile.as_deref(), keys) {
                        Ok(g) => g,
                        Err(why) => return Done::Failed(why),
                    };
                    match g.opening(|c| v.unlock(c)) {
                        Ok(()) => {
                            let _ = v.remember(remember);
                            if keyfile.is_some() {
                                remember_keyfile(keyfile.as_deref());
                            }
                            Done::Opened
                        }
                        Err(e) => Done::Failed(said(e)),
                    }
                });
            }
            Mode::ChangePassword => {
                let Some(new) = new_password(self, Some(&pw)) else { return };
                let new_keyfile = match self.chosen_keyfile() {
                    Ok(k) => k,
                    Err(why) => return self.error = Some(why),
                };
                let old_keyfile = self.factors.keyfile.then(|| self.old_keyfile.clone());
                let old_keys = !self.factors.security_keys.is_empty();
                let (keep, add) = (self.keep_keys, self.enrolled.clone());
                self.spawn(move |v| {
                    let old = match Gathered::get(pw, old_keyfile.as_deref(), old_keys) {
                        Ok(g) => g,
                        Err(why) => return Done::Failed(why),
                    };
                    let new = match Gathered::get(new, new_keyfile.as_deref(), false) {
                        Ok(g) => g,
                        Err(why) => return Done::Failed(why),
                    };
                    match old.opening(|c| v.change_key(&c, &new.new_key(keep, &add))) {
                        Ok(()) => {
                            remember_keyfile(new_keyfile.as_deref());
                            Done::Opened
                        }
                        Err(e) => Done::Failed(said(e)),
                    }
                });
            }
            Mode::Reveal { ns, name } => {
                let (keyfile, keys) = self.needed();
                let wrong = if self.factors.is_empty() { VaultError::WrongPassword } else { VaultError::WrongKey };
                self.spawn(move |v| {
                    let g = match Gathered::get(pw, keyfile.as_deref(), keys) {
                        Ok(g) => g,
                        Err(why) => return Done::Failed(why),
                    };
                    match g.opening(|c| v.check_password(c)) {
                        Ok(true) => match v.get(&ns, &name) {
                            Ok(Some(value)) => Done::Revealed { ns, name, value },
                            Ok(None) => Done::Failed(i18n::t("vault.gone")),
                            Err(e) => Done::Failed(said(e)),
                        },
                        Ok(false) => Done::Failed(said(wrong)),
                        Err(e) => Done::Failed(said(e)),
                    }
                });
            }
        }
    }

    /// The window: create, unlock, change the password, or confirm a reveal.
    fn window(&mut self, ctx: &egui::Context, bounds: Option<egui::Rect>) {
        let open = self.mode.is_some() && !self.closing;
        let mode = self.mode.clone().unwrap_or(Mode::Unlock);
        let (title, lead) = match &mode {
            Mode::Create => (i18n::t("vault.create_title"), i18n::t("vault.create_lead")),
            Mode::Unlock => (i18n::t("vault.unlock_title"), String::new()),
            Mode::ChangePassword => (i18n::t("vault.change_title"), String::new()),
            Mode::Reveal { name, .. } => (i18n::t("vault.reveal_title").replace("{name}", name), String::new()),
        };
        let busy = self.busy();
        let mut submit = false;
        let mut cancel = false;
        // A file dialog asked for: which field, and whether to make a new file.
        let mut want_pick: Option<(KeyField, bool)> = None;
        // "Add the plugged-in YubiKey" was pressed.
        let mut want_key = false;
        let now = ctx.input(|i| i.time);
        let succeeded = self.succeeded_at.map(|at| now - at);
        let shake = self.shake_at.map(|at| now - at).filter(|d| *d < SHAKE_S);
        if shake.is_some() {
            ctx.request_repaint();
        }
        let opts = ui::OverlayOpts { width: 440.0, bounds, ..Default::default() };
        let out = ui::overlay(ctx, egui::Id::new("limen_vault_window"), open, &opts, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(&title).size(17.0).strong());
            });
            if let Some(d) = succeeded {
                // It worked: a padlock swings open, and says what happened.
                let t = if ui::animations_enabled() { (d / SWING_S).min(1.0) as f32 } else { 1.0 };
                let e = ui::ease_in_out_back(t);
                ui.add_space(18.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
                let ink = ui::lerp_color(ui::color::TEXT_MUTED, ui::color::ACCENT_BRIGHT, t);
                paint_padlock(ui.painter(), rect.center(), 3.2, e, ink);
                ui.add_space(14.0);
                let said = match &mode {
                    Mode::Create => i18n::t("vault.created"),
                    Mode::ChangePassword => i18n::t("vault.changed"),
                    _ => i18n::t("vault.unlocked"),
                };
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(said).color(ui::color::ACCENT_BRIGHT));
                });
                ui.add_space(10.0);
                return;
            }
            if let Some(who) = &self.asked_by {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(i18n::t("vault.asked_by").replace("{module}", who)).color(ui::color::TEXT_MUTED));
            }
            if !lead.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&lead).small().color(ui::color::TEXT_MUTED));
            }
            ui.add_space(12.0);
            // A wrong password shakes the field it was typed in: a quick,
            // fading side to side.
            let jolt = shake.map(|d| {
                let u = (d / SHAKE_S) as f32;
                (1.0 - u) * 7.0 * (u * std::f32::consts::PI * 6.0).sin()
            });
            let field = |ui: &mut egui::Ui, label: &str, value: &mut String, shaken: bool| {
                ui.label(egui::RichText::new(label).small());
                let r = ui
                    .horizontal(|ui| {
                        let dx = if shaken { jolt.unwrap_or(0.0) } else { 0.0 };
                        ui.add_space(6.0 + dx);
                        ui::text_field(ui, value, "", ui.available_width() - 12.0, true)
                    })
                    .inner;
                ui.add_space(8.0);
                r
            };
            let first_label = match &mode {
                Mode::Create => i18n::t("vault.new_password"),
                Mode::ChangePassword => i18n::t("vault.current_password"),
                _ => i18n::t("vault.password"),
            };
            let first = field(ui, &first_label, &mut self.password, true);
            if !first.has_focus() && self.password.is_empty() && !busy {
                first.request_focus();
            }
            let mut enter = first.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // A key file's path, with a dialog to find it — or make one.
            let keyfile_row = |ui: &mut egui::Ui, label: &str, value: &mut String, which: KeyField, can_make: bool| {
                ui.label(egui::RichText::new(label).small());
                let mut asked = None;
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let buttons = if can_make { 196.0 } else { 96.0 };
                    ui::text_field(ui, value, "", (ui.available_width() - buttons - 12.0).max(80.0), false);
                    if ui::outline_button(ui, &i18n::t("vault.browse"), egui::Vec2::ZERO).clicked() {
                        asked = Some((which, false));
                    }
                    if can_make && ui::outline_button(ui, &i18n::t("vault.make_keyfile"), egui::Vec2::ZERO).clicked() {
                        asked = Some((which, true));
                    }
                });
                ui.add_space(8.0);
                asked
            };
            let note = |ui: &mut egui::Ui, text: String| {
                ui.label(egui::RichText::new(text).small().color(ui::color::TEXT_MUTED));
            };
            // What the vault needs now, besides the password.
            if self.factors.keyfile && matches!(mode, Mode::Unlock | Mode::Reveal { .. }) {
                want_pick = want_pick.or(keyfile_row(ui, &i18n::t("vault.keyfile"), &mut self.keyfile, KeyField::New, false));
            }
            if self.factors.keyfile && mode == Mode::ChangePassword {
                want_pick =
                    want_pick.or(keyfile_row(ui, &i18n::t("vault.current_keyfile"), &mut self.old_keyfile, KeyField::Current, false));
            }
            if mode == Mode::ChangePassword {
                let r = field(ui, &i18n::t("vault.keep_password"), &mut self.new_password, false);
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            if matches!(mode, Mode::Create | Mode::ChangePassword) {
                let r = field(ui, &i18n::t("vault.confirm_password"), &mut self.confirm, false);
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                // The factors it will need: as it has them, to add, swap or drop.
                ui::toggle(ui, &mut self.use_keyfile, &i18n::t("vault.require_keyfile"));
                if self.use_keyfile {
                    ui.add_space(4.0);
                    want_pick = want_pick.or(keyfile_row(ui, &i18n::t("vault.keyfile"), &mut self.keyfile, KeyField::New, true));
                    note(ui, i18n::t("vault.keyfile_hint"));
                }
                ui.add_space(6.0);
                // YubiKeys: any one of them will open it.
                ui.label(egui::RichText::new(i18n::t("vault.keys")).small());
                if mode == Mode::ChangePassword && !self.factors.security_keys.is_empty() {
                    let n = self.factors.security_keys.len();
                    ui::toggle(ui, &mut self.keep_keys, &i18n::t("vault.keep_keys").replace("{n}", &n.to_string()));
                    if !self.keep_keys {
                        note(ui, i18n::t("vault.keys_removed"));
                    }
                }
                let mut drop_at = None;
                for (i, key) in self.enrolled.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        ui.label(format!("{} · {} {}", key.name, i18n::t("vault.slot"), key.slot));
                        if ui::outline_button(ui, "✕", egui::Vec2::ZERO).clicked() {
                            drop_at = Some(i);
                        }
                    });
                }
                if let Some(i) = drop_at {
                    self.enrolled.remove(i);
                }
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(i18n::t("vault.slot")).small());
                    for slot in [1u8, 2] {
                        if ui::chip(ui, &slot.to_string(), self.key_slot == slot).clicked() {
                            self.key_slot = slot;
                        }
                    }
                    let add = ui
                        .add_enabled_ui(!self.enrolling && !busy, |ui| ui::outline_button(ui, &i18n::t("vault.add_key"), egui::Vec2::ZERO))
                        .inner;
                    if add.clicked() {
                        want_key = true;
                    }
                    if self.enrolling {
                        ui.spinner();
                    }
                });
                if self.enrolling {
                    ui.label(egui::RichText::new(i18n::t("vault.touch")).color(ui::color::ACCENT_BRIGHT));
                }
                if !self.enrolled.is_empty() || (self.keep_keys && !self.factors.security_keys.is_empty()) {
                    note(ui, i18n::t("vault.key_hint"));
                }
                ui.add_space(6.0);
            }
            // A YubiKey is being asked: it may be waiting for a touch.
            // Only opening asks a key now: those being added were asked already.
            let asking_key = !matches!(mode, Mode::Create) && !self.factors.security_keys.is_empty();
            if busy && asking_key {
                ui.label(egui::RichText::new(i18n::t("vault.touch")).color(ui::color::ACCENT_BRIGHT));
                ui.add_space(4.0);
            }
            if matches!(mode, Mode::Create | Mode::Unlock) {
                ui::toggle(ui, &mut self.remember, &i18n::t("vault.remember"));
                ui.label(egui::RichText::new(i18n::t("vault.remember_hint")).small().color(ui::color::TEXT_MUTED));
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(e).color(ui::color::ERROR));
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let go = match &mode {
                    Mode::Create => i18n::t("vault.create"),
                    Mode::Unlock => i18n::t("vault.unlock"),
                    Mode::ChangePassword => i18n::t("vault.change"),
                    Mode::Reveal { .. } => i18n::t("vault.reveal"),
                };
                let clicked = ui
                    .add_enabled_ui(!busy, |ui| ui::primary_button(ui, &go, egui::vec2(140.0, 0.0)))
                    .inner
                    .clicked();
                submit = (clicked || enter) && !busy;
                if ui::outline_button(ui, &i18n::t("vault.cancel"), egui::vec2(120.0, 0.0)).clicked() {
                    cancel = true;
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        if want_key {
            self.add_key();
        }
        if let Some((which, make)) = want_pick {
            let start = match which {
                KeyField::New => self.keyfile.clone(),
                KeyField::Current => self.old_keyfile.clone(),
            };
            pick_keyfile(which, make, start, self.picks.0.clone(), ctx.clone());
        }
        if submit {
            self.error = None;
            self.submit();
        }
        if (cancel || out.back || out.close) && !busy && self.succeeded_at.is_none() && open {
            // Faded out as it stands, then let go.
            self.closing = true;
            self.zero_fields();
            // Not opened: the modules' calls waiting for it fail now.
            if matches!(self.mode, Some(Mode::Unlock | Mode::Create)) {
                self.shared.decline();
            }
        }
        if out.closed && self.closing {
            self.closing = false;
            self.mode = None;
            self.succeeded_at = None;
            self.asked_by = None;
            self.error = None;
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context, bounds: Option<egui::Rect>) {
        let open = self.deleting.is_some();
        if open {
            self.deleting_shown = self.deleting.clone();
        }
        let (ns, name) = self.deleting_shown.clone().unwrap_or_default();
        let subject = format!("{} · {name}", owner_label(&ns));
        match ui::confirm_dialog(
            ctx,
            "vault_delete",
            open,
            &i18n::t("vault.delete_title"),
            Some(&subject),
            &i18n::t("vault.delete"),
            &i18n::t("vault.cancel"),
            bounds,
        ) {
            Some(true) => {
                self.deleting = None;
                let _ = self.shared.vault().delete(&ns, &name);
                if self.revealed.as_ref().is_some_and(|(n, k, _, _)| *n == ns && *k == name) {
                    self.revealed = None;
                }
            }
            Some(false) => self.deleting = None,
            None => {}
        }
    }

    /// The Vault section of Settings. `installed`: the modules there are, to
    /// tell an orphaned compartment apart.
    pub(crate) fn section(&mut self, ui: &mut egui::Ui, installed: &[String]) {
        let muted = |ui: &mut egui::Ui, t: String| {
            ui.label(egui::RichText::new(t).small().color(ui::color::TEXT_MUTED));
        };
        if let Some(why) = &self.shared.damaged {
            ui.label(egui::RichText::new(i18n::t("vault.damaged").replace("{why}", why)).color(ui::color::ERROR));
            return;
        }
        let (exists, open) = self.peek();
        if !exists {
            muted(ui, i18n::t("vault.none"));
            ui.add_space(6.0);
            if ui::primary_button(ui, &i18n::t("vault.create"), egui::Vec2::ZERO).clicked() {
                self.open_window(Mode::Create);
            }
            return;
        }
        if let Some(f) = self.shared.vault.try_lock().ok().and_then(|v| v.factors().cloned()) {
            muted(ui, unlocks_with(&f));
        }
        if !open {
            muted(ui, i18n::t("vault.locked"));
            ui.add_space(6.0);
            if ui::primary_button(ui, &i18n::t("vault.unlock"), egui::Vec2::ZERO).clicked() {
                self.open_window(Mode::Unlock);
            }
            return;
        }
        // Looking at it is using it.
        self.shared.touch();
        muted(ui, i18n::t("vault.open"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui::outline_button(ui, &i18n::t("vault.lock_now"), egui::Vec2::ZERO).clicked() {
                self.lock();
            }
            if ui::outline_button(ui, &i18n::t("vault.change"), egui::Vec2::ZERO).clicked() {
                self.open_window(Mode::ChangePassword);
            }
        });
        ui.add_space(8.0);
        let mut remember = self.remembered;
        if ui::toggle(ui, &mut remember, &i18n::t("vault.remember")).changed() && !self.busy() {
            self.spawn(move |v| {
                let _ = v.remember(remember);
                Done::Remembered(v.remembered())
            });
        }
        muted(ui, i18n::t("vault.remember_hint"));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(i18n::t("vault.auto_lock")).small());
        ui.horizontal(|ui| {
            for minutes in [5u32, 15, 30, 60, 0] {
                let label = if minutes == 0 { i18n::t("vault.never") } else { format!("{minutes} min") };
                let on = lock_after_from(Some(minutes)) == self.lock_after;
                if ui::chip(ui, &label, on).clicked() {
                    self.lock_after = lock_after_from(Some(minutes));
                    if let Ok(mut cfg) = limen_core::Config::load() {
                        cfg.vault_lock_minutes = Some(minutes);
                        let _ = cfg.save();
                    }
                }
            }
        });
        ui.add_space(12.0);
        // Busy (Argon2, the keychain): say so rather than wait on it.
        let Ok(contents) = self.shared.vault.try_lock().map(|v| v.contents().unwrap_or_default()) else {
            ui.horizontal(|ui| {
                ui.spinner();
                muted(ui, i18n::t("vault.working"));
            });
            return;
        };
        if contents.is_empty() {
            muted(ui, i18n::t("vault.empty"));
        }
        for (ns, names) in contents {
            let orphan = ns
                .strip_prefix("module:")
                .is_some_and(|m| !installed.iter().any(|i| i == m));
            let mut head = owner_label(&ns);
            if orphan {
                head = format!("{head} · {}", i18n::t("vault.not_installed"));
            }
            ui.label(egui::RichText::new(head).strong());
            for name in names {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(&name).monospace());
                    let shown = self.revealed.as_ref().filter(|(n, k, _, _)| *n == ns && *k == name).map(|r| r.2.clone());
                    match shown {
                        Some(value) => {
                            ui.label(egui::RichText::new(&value).monospace().color(ui::color::ACCENT_BRIGHT));
                            if ui::outline_button(ui, &i18n::t("vault.copy"), egui::Vec2::ZERO).clicked() {
                                ui.ctx().copy_text(value);
                            }
                            if ui::outline_button(ui, &i18n::t("vault.hide"), egui::Vec2::ZERO).clicked() {
                                self.revealed = None;
                            }
                        }
                        None => {
                            if ui::outline_button(ui, &i18n::t("vault.reveal"), egui::Vec2::ZERO).clicked() {
                                self.open_window(Mode::Reveal { ns: ns.clone(), name: name.clone() });
                            }
                        }
                    }
                    if ui::outline_button(ui, &i18n::t("vault.delete"), egui::Vec2::ZERO).clicked() {
                        self.deleting = Some((ns.clone(), name.clone()));
                    }
                });
            }
            ui.add_space(6.0);
        }
    }

    /// The padlock in the title bar: shut or open; a click opens or locks it.
    /// Nothing until there is a vault.
    pub(crate) fn padlock(&mut self, ui: &mut egui::Ui) {
        let (exists, open) = self.peek();
        if !exists {
            return;
        }
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        let hover = ui::anim_bool(ui, resp.id.with("hover"), resp.hovered(), 0.15);
        // How open it is, 0 shut .. 1 open, eased with a little overshoot: the
        // shackle lifts and swings out, or drops back and snaps home.
        let t = if ui::animations_enabled() {
            ui.ctx().animate_bool_with_time(resp.id.with("open"), open, 0.32)
        } else {
            open as u8 as f32
        };
        let e = ui::ease_in_out_back(t);
        let base = ui::lerp_color(ui::color::TEXT_MUTED, ui::color::ACCENT_BRIGHT, t.clamp(0.0, 1.0));
        let ink = ui::lerp_color(base, egui::Color32::WHITE, hover * 0.6);
        paint_padlock(ui.painter(), rect.center(), 1.0, e, ink);
        if t > 0.0 && t < 1.0 {
            ui.ctx().request_repaint();
        }
        let tip = if open { i18n::t("vault.tip_open") } else { i18n::t("vault.tip_locked") };
        let resp = resp.on_hover_text(tip);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            if open {
                self.lock();
            } else {
                self.open_window(Mode::Unlock);
            }
        }
    }
}

/// A padlock centred on `at`, `scale` times the title bar's, `open` from 0
/// (shut) to 1 (open — overshoot welcome). The shackle is a half-ring over two
/// legs; the left one is the hinge and stays in the body, while the ring with
/// its right leg lifts and turns about its top, so the right leg comes out.
fn paint_padlock(p: &egui::Painter, at: egui::Pos2, scale: f32, open: f32, ink: egui::Color32) {
    let k = scale;
    let s = egui::Stroke::new(1.6_f32 * k, ink);
    let body = egui::Rect::from_center_size(at + egui::vec2(0.0, 3.0 * k), egui::vec2(12.0 * k, 9.0 * k));
    p.rect_stroke(body, 1.5 * k, s);
    let (l, r) = (body.left() + 2.5 * k, body.right() - 2.5 * k);
    let lift = 3.0 * k * open;
    let hinge = egui::pos2(l, body.top() - 1.5 * k);
    let (sin, cos) = (-0.45 * open).sin_cos();
    let turn = |q: egui::Pos2| {
        let d = q - hinge;
        hinge + egui::vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
    };
    let top = body.top() - 6.0 * k - lift;
    let radius = (r - l) / 2.0;
    let mut shackle: Vec<egui::Pos2> = vec![egui::pos2(l, body.top() - lift)];
    shackle.extend((0..=12).map(|i| {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
        egui::pos2((l + r) / 2.0 + a.cos() * radius, top + radius + a.sin() * radius)
    }));
    shackle.push(egui::pos2(r, body.top() - lift));
    let shackle: Vec<egui::Pos2> = shackle.into_iter().map(turn).collect();
    p.add(egui::Shape::line(shackle.clone(), s));
    // The hinge leg reaches back down into the body however far it lifted.
    p.line_segment([shackle[0], egui::pos2(l, body.top())], s);
}

/// How an owner shows on the Vault page: "Limen", or the module's name.
fn owner_label(ns: &str) -> String {
    if ns == LIMEN {
        "Limen".into()
    } else {
        ns.strip_prefix("module:").unwrap_or(ns).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Clone, Default)]
    struct MemKeys(Arc<Mutex<HashMap<String, String>>>);

    impl vault::KeyStore for MemKeys {
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

    fn store(name: &str, keys: &MemKeys) -> (VaultStore, Arc<Shared>) {
        waiting_store(name, keys, Duration::from_millis(50))
    }

    /// A store whose calls wait `wait_for` for the vault to be opened.
    fn waiting_store(name: &str, keys: &MemKeys, wait_for: Duration) -> (VaultStore, Arc<Shared>) {
        let dir = std::env::temp_dir().join(format!("limen-gui-vault-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = Vault::load(&dir.join("vault.json"), Box::new(keys.clone())).unwrap();
        let shared = Arc::new(Shared {
            vault: Mutex::new(v),
            damaged: None,
            last_used: Mutex::new(Instant::now()),
            asked_by: Mutex::new(None),
            declined: Mutex::new(0),
            woken: Condvar::new(),
            wait_for,
            ctx: egui::Context::default(),
        });
        (VaultStore(shared.clone()), shared)
    }

    fn asked(s: &Shared) -> Option<String> {
        s.asked_by.lock().unwrap().take()
    }

    #[test]
    fn a_module_meeting_no_vault_or_a_shut_one_asks_the_user() {
        let keys = MemKeys::default();
        let (st, shared) = store("ask", &keys);
        assert_eq!(st.get("crowdstrike", "client_secret"), Err(SecretError::NoVault));
        assert_eq!(asked(&shared).as_deref(), Some("crowdstrike"), "no vault: the user is asked to make one");

        shared.vault().create("correct horse", Kdf::TEST).unwrap();
        st.set("crowdstrike", "client_secret", "s").unwrap();
        shared.vault().lock();
        assert_eq!(st.get("crowdstrike", "client_secret"), Err(SecretError::Locked));
        assert_eq!(asked(&shared).as_deref(), Some("crowdstrike"), "locked, nothing remembered: asked");
    }

    /// What the window does, from another thread, once a module has asked:
    /// waits until the ask arrives, then answers with `answer`.
    fn the_user(shared: &Arc<Shared>, answer: impl FnOnce(&Shared) + Send + 'static) -> std::thread::JoinHandle<()> {
        let shared = shared.clone();
        std::thread::spawn(move || {
            while asked(&shared).is_none() {
                std::thread::sleep(Duration::from_millis(5));
            }
            answer(&shared);
        })
    }

    /// A vault with one secret in it, locked: a call to it has to wait.
    fn locked(name: &str, wait_for: Duration) -> (VaultStore, Arc<Shared>) {
        let keys = MemKeys::default();
        let (st, shared) = waiting_store(name, &keys, wait_for);
        shared.vault().create("correct horse", Kdf::TEST).unwrap();
        st.set("crowdstrike", "client_secret", "s").unwrap();
        shared.vault().lock();
        (st, shared)
    }

    #[test]
    fn a_call_to_a_locked_vault_goes_on_once_it_is_unlocked() {
        let (st, shared) = locked("goes-on", Duration::from_secs(30));
        let user = the_user(&shared, |s| {
            // A wrong password first: the window stays up, the call waits on.
            assert!(s.vault().unlock("wrong horse").is_err());
            s.wake();
            std::thread::sleep(Duration::from_millis(100));
            s.vault().unlock("correct horse").unwrap();
            s.wake();
        });
        assert_eq!(st.get("crowdstrike", "client_secret").unwrap().as_deref(), Some("s"), "answered, not refused");
        user.join().unwrap();
        // Writing waits the same way.
        shared.vault().lock();
        let user = the_user(&shared, |s| {
            s.vault().unlock("correct horse").unwrap();
            s.wake();
        });
        st.set("crowdstrike", "client_id", "id").unwrap();
        user.join().unwrap();
        assert_eq!(st.get("crowdstrike", "client_id").unwrap().as_deref(), Some("id"));
    }

    #[test]
    fn turning_the_window_down_fails_the_waiting_call_at_once() {
        let (st, shared) = locked("declined", Duration::from_secs(30));
        let user = the_user(&shared, |s| s.decline());
        let started = Instant::now();
        assert_eq!(st.get("crowdstrike", "client_secret"), Err(SecretError::Locked));
        assert!(started.elapsed() < Duration::from_secs(5), "not left to time out: {:?}", started.elapsed());
        user.join().unwrap();
    }

    #[test]
    fn a_call_gives_up_when_nobody_answers() {
        let (st, _shared) = locked("timeout", Duration::from_millis(100));
        let started = Instant::now();
        assert_eq!(st.get("crowdstrike", "client_secret"), Err(SecretError::Locked));
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[test]
    fn a_call_meeting_no_vault_goes_on_once_one_is_made() {
        let keys = MemKeys::default();
        let (st, shared) = waiting_store("made", &keys, Duration::from_secs(30));
        let user = the_user(&shared, |s| {
            s.vault().create("correct horse", Kdf::TEST).unwrap();
            s.wake();
        });
        st.set("crowdstrike", "client_secret", "new").unwrap();
        user.join().unwrap();
        assert_eq!(st.get("crowdstrike", "client_secret").unwrap().as_deref(), Some("new"));

        // And made not: no vault, as before.
        let (st, shared) = waiting_store("not-made", &MemKeys::default(), Duration::from_secs(30));
        let user = the_user(&shared, |s| s.decline());
        assert_eq!(st.get("crowdstrike", "client_secret"), Err(SecretError::NoVault));
        user.join().unwrap();
    }

    #[test]
    fn a_new_key_file_is_never_written_over_another() {
        let dir = std::env::temp_dir().join(format!("limen-gui-keyfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("limen.key");
        make_keyfile(&path).unwrap();
        let first = std::fs::read(&path).unwrap();
        assert_eq!(first.len(), 65);
        assert_eq!(make_keyfile(&path), Err(i18n::t("vault.keyfile_exists")), "there already");
        assert_eq!(std::fs::read(&path).unwrap(), first, "and left as it was");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "its owner's only");
        }
        assert_eq!(read_keyfile(path.to_str().unwrap()).unwrap().as_slice(), first.as_slice());
        assert_eq!(read_keyfile("  "), Err(i18n::t("vault.no_keyfile")));
        assert!(read_keyfile(dir.join("not-there").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_remembered_key_reopens_after_the_idle_lock_without_asking() {
        let keys = MemKeys::default();
        let (st, shared) = store("remember", &keys);
        shared.vault().create("correct horse", Kdf::TEST).unwrap();
        shared.vault().remember(true).unwrap();
        st.set("crowdstrike", "client_secret", "s").unwrap();
        shared.vault().lock();
        assert_eq!(st.get("crowdstrike", "client_secret").unwrap().as_deref(), Some("s"));
        assert_eq!(asked(&shared), None, "opened with this machine's key; nobody asked");
    }

    #[test]
    fn each_module_reaches_only_its_own_compartment() {
        let keys = MemKeys::default();
        let (st, shared) = store("compartments", &keys);
        shared.vault().create("correct horse", Kdf::TEST).unwrap();
        st.set("a", "token", "A").unwrap();
        st.set("b", "token", "B").unwrap();
        shared.vault().set(LIMEN, "github_token", "G").unwrap();
        assert_eq!(st.get("a", "token").unwrap().as_deref(), Some("A"));
        assert_eq!(st.get("b", "token").unwrap().as_deref(), Some("B"));
        assert_eq!(st.get("a", "github_token").unwrap(), None, "Limen's own is not a module's");
        assert_eq!(st.names("a").unwrap(), ["token"]);
        assert!(st.delete("a", "token").unwrap());
        assert_eq!(st.get("b", "token").unwrap().as_deref(), Some("B"), "b's is untouched");
    }

    #[test]
    fn using_it_resets_the_idle_clock() {
        let keys = MemKeys::default();
        let (st, shared) = store("idle", &keys);
        shared.vault().create("correct horse", Kdf::TEST).unwrap();
        *shared.last_used.lock().unwrap() = Instant::now() - Duration::from_secs(600);
        st.get("a", "x").unwrap();
        assert!(shared.last_used.lock().unwrap().elapsed() < Duration::from_secs(5));
    }
}
