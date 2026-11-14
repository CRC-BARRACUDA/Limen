//! Limen's vault, as the GUI holds it: shared with the host (modules reach it
//! through `host.secret_*`), the window that creates or unlocks it, the lock
//! after 15 idle minutes, the Vault section of Settings and the padlock in the
//! title bar.
//!
//! The vault is behind one mutex. Everything slow — Argon2 (half a second on
//! purpose), the keychain — runs on a thread; the window shows a spinner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use limen_core::vault::{self, Kdf, Vault, VaultError, LIMEN};
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
}

/// The vault as the host sees it: one compartment per module, opened quietly
/// with this machine's key when it can be, and asking the user when not.
struct VaultStore(Arc<Shared>);

impl VaultStore {
    /// The vault, open — or why not, with the user asked.
    fn with<T>(&self, module: &str, f: impl FnOnce(&mut Vault) -> Result<T, VaultError>) -> Result<T, SecretError> {
        if let Some(why) = &self.0.damaged {
            return Err(SecretError::Failed(format!("Limen's vault is damaged: {why}")));
        }
        let mut v = self.0.vault();
        if !v.exists() {
            drop(v);
            self.0.ask(module);
            return Err(SecretError::NoVault);
        }
        // Locked after a while idle, but this machine remembers the key: open
        // again without asking — the lock wiped the key from memory, no more.
        if !v.is_open() && !v.unlock_remembered().unwrap_or(false) {
            drop(v);
            self.0.ask(module);
            return Err(SecretError::Locked);
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

/// A finished piece of slow work.
enum Done {
    /// Opened (made, unlocked, password changed): close the window.
    Opened,
    /// The password was right: show this.
    Revealed { ns: String, name: String, value: String },
    Failed(String),
    /// Whether this machine remembers the key now.
    Remembered(bool),
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
                    if why == i18n::t("vault.wrong") {
                        self.shake_at = Some(now);
                    }
                    self.error = Some(why);
                }
                Done::Remembered(on) => {
                    self.remembered = on;
                    just_opened |= self.is_open();
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
    }

    fn submit(&mut self) {
        let Some(mode) = self.mode.clone() else { return };
        let pw = std::mem::take(&mut self.password);
        let remember = self.remember;
        match mode {
            Mode::Create => {
                if pw.chars().count() < MIN_PASSWORD {
                    self.error = Some(i18n::t("vault.too_short").replace("{n}", &MIN_PASSWORD.to_string()));
                    return;
                }
                if pw != self.confirm {
                    self.error = Some(i18n::t("vault.mismatch"));
                    return;
                }
                self.spawn(move |v| match v.create(&pw, Kdf::DEFAULT).and_then(|_| v.remember(remember)) {
                    Ok(()) => Done::Opened,
                    Err(e) => Done::Failed(e.to_string()),
                });
            }
            Mode::Unlock => self.spawn(move |v| match v.unlock(&pw) {
                Ok(()) => {
                    let _ = v.remember(remember);
                    Done::Opened
                }
                Err(VaultError::WrongPassword) => Done::Failed(i18n::t("vault.wrong")),
                Err(e) => Done::Failed(e.to_string()),
            }),
            Mode::ChangePassword => {
                let new = std::mem::take(&mut self.new_password);
                if new.chars().count() < MIN_PASSWORD {
                    self.error = Some(i18n::t("vault.too_short").replace("{n}", &MIN_PASSWORD.to_string()));
                    return;
                }
                if new != self.confirm {
                    self.error = Some(i18n::t("vault.mismatch"));
                    return;
                }
                self.spawn(move |v| match v.change_password(&pw, &new) {
                    Ok(()) => Done::Opened,
                    Err(VaultError::WrongPassword) => Done::Failed(i18n::t("vault.wrong")),
                    Err(e) => Done::Failed(e.to_string()),
                });
            }
            Mode::Reveal { ns, name } => self.spawn(move |v| match v.check_password(&pw) {
                Ok(true) => match v.get(&ns, &name) {
                    Ok(Some(value)) => Done::Revealed { ns, name, value },
                    Ok(None) => Done::Failed(i18n::t("vault.gone")),
                    Err(e) => Done::Failed(e.to_string()),
                },
                Ok(false) => Done::Failed(i18n::t("vault.wrong")),
                Err(e) => Done::Failed(e.to_string()),
            }),
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
            if mode == Mode::ChangePassword {
                let r = field(ui, &i18n::t("vault.new_password"), &mut self.new_password, false);
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            if matches!(mode, Mode::Create | Mode::ChangePassword) {
                let r = field(ui, &i18n::t("vault.confirm_password"), &mut self.confirm, false);
                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
        if submit {
            self.error = None;
            self.submit();
        }
        if (cancel || out.back || out.close) && !busy && self.succeeded_at.is_none() && open {
            // Faded out as it stands, then let go.
            self.closing = true;
            self.zero_fields();
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
        let dir = std::env::temp_dir().join(format!("limen-gui-vault-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let v = Vault::load(&dir.join("vault.json"), Box::new(keys.clone())).unwrap();
        let shared = Arc::new(Shared {
            vault: Mutex::new(v),
            damaged: None,
            last_used: Mutex::new(Instant::now()),
            asked_by: Mutex::new(None),
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
