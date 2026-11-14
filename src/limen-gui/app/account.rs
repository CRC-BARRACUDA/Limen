//! The signed-in account, as the GUI holds it.
//!
//! Everything slow — waiting for the browser, the keychain (a D-Bus call on
//! Linux), the platform — runs on a thread of its own and reports back over a
//! channel, drained once a frame by [`AccountUi::poll`]. The access token lives
//! here, in memory, and nowhere else; the refresh token only in the keychain.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use eframe::egui;
use limen_core::account::{self, Account, Platform, SignIn, Tokens};
use limen_core::avatar::{self as av, AvatarService};
use limen_ui::avatar::{AvatarLook, Decoded, EffectKind, EffectSpec, Picture};
use limen_ui::user_card::{self, CardInfo};

use crate::i18n;

/// How long the browser has to come back before the sign-in gives up.
const SIGN_IN_FOR: Duration = Duration::from_secs(5 * 60);

/// Where the account stands, as the Settings page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountState {
    /// No client id is configured: there is nothing to sign in as.
    Off,
    SignedOut,
    /// A remembered session is being checked at start; `who` is who it was.
    Restoring { who: Option<Account> },
    /// The browser is out; `url` is the sign-in page, to open again.
    Waiting { url: String },
    SignedIn {
        who: Account,
        /// The keychain kept the session: it survives a restart.
        remembered: bool,
        /// Said under the account: the platform could not be reached, say.
        note: Option<String>,
    },
    /// The last attempt failed, in these words.
    Failed(String),
}

/// What the Settings page asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountAction {
    SignIn,
    Cancel,
    SignOut,
}

enum Done {
    SignedIn(Tokens, Account, bool),
    Restored(Option<(Tokens, Account)>),
    Failed(String),
    /// The platform could not be reached at start: keep showing who it was.
    Unreachable(String),
    /// How a user looks, fetched and decoded: their decoration's effect, their
    /// picture and their frame.
    Look { user_id: String, effect: Option<EffectSpec>, picture: Option<Decoded>, frame: Option<Decoded> },
}

pub(crate) struct AccountUi {
    pub(crate) state: AccountState,
    platform: Option<Platform>,
    /// The session's tokens, once signed in. In memory only.
    tokens: Option<Tokens>,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    ctx: egui::Context,
    avatars: AvatarService,
    /// How the signed-in user looks: initials at once, the picture and the
    /// decoration once they have arrived.
    look: Option<AvatarLook>,
    /// Whose look was last asked for.
    look_for: Option<String>,
    /// "Sign out?" is on screen, waiting for an answer.
    asking_sign_out: bool,
    /// Who it asks about, kept while the window closes so it does not blink empty.
    sign_out_subject: String,
}

impl AccountUi {
    /// From the settings, and — when an account was signed in — start checking
    /// that its session still holds.
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let cfg = limen_core::Config::load().unwrap_or_default();
        let platform = Platform::from_config(&cfg);
        let (tx, rx) = mpsc::channel();
        let mut me = Self {
            state: AccountState::Off,
            platform,
            tokens: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            ctx: ctx.clone(),
            avatars: AvatarService::from_config(&cfg),
            look: None,
            look_for: None,
            asking_sign_out: false,
            sign_out_subject: String::new(),
        };
        if let Some(p) = me.platform.clone() {
            me.state = AccountState::Restoring { who: cfg.account.clone() };
            // Who it was is known already: show them while the session is checked.
            if let Some(who) = &cfg.account {
                me.load_look(who);
            }
            me.spawn(move || match account::restore(&p) {
                Ok(found) => Done::Restored(found),
                // Refused outright: the session is over. A server error or no
                // network is not that — keep showing who it was.
                Err(e)
                    if e
                        .downcast_ref::<account::PlatformSaid>()
                        .is_some_and(|s| (400..500).contains(&s.status)) =>
                {
                    Done::Restored(None)
                }
                Err(e) => Done::Unreachable(e.to_string()),
            });
        }
        me
    }

    fn spawn(&self, work: impl FnOnce() -> Done + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }

    /// Do what the Settings page asked.
    pub(crate) fn act(&mut self, action: AccountAction) {
        match action {
            AccountAction::SignIn => self.sign_in(),
            AccountAction::Cancel => self.cancel.store(true, Ordering::Relaxed),
            AccountAction::SignOut => {
                // Asked first: the session goes from this machine, and coming
                // back means the browser again.
                if let Some(who) = self.who() {
                    self.sign_out_subject = format!("{} (@{})", who.display_name(), who.username);
                    self.asking_sign_out = true;
                }
            }
        }
    }

    /// The "Sign out?" window — Limen's own confirmation, as removing a module
    /// has. Every frame: it fades in from how shut it was the frame before.
    pub(crate) fn sign_out_dialog(&mut self, ctx: &egui::Context, bounds: Option<egui::Rect>) {
        let answer = limen_ui::confirm_dialog(
            ctx,
            "sign_out",
            self.asking_sign_out,
            &i18n::t("account.sign_out_title"),
            Some(&self.sign_out_subject),
            &i18n::t("account.sign_out"),
            &i18n::t("account.stay"),
            bounds,
        );
        match answer {
            Some(true) => {
                self.asking_sign_out = false;
                self.sign_out();
            }
            Some(false) => self.asking_sign_out = false,
            None => {}
        }
    }

    fn sign_in(&mut self) {
        let Some(p) = self.platform.clone() else { return };
        if matches!(self.state, AccountState::Waiting { .. }) {
            return;
        }
        let flow = match SignIn::start(&p) {
            Ok(f) => f,
            Err(e) => {
                self.state = AccountState::Failed(e.to_string());
                return;
            }
        };
        let url = flow.url().to_string();
        self.ctx.open_url(egui::OpenUrl::new_tab(&url));
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.state = AccountState::Waiting { url };
        self.spawn(move || match flow.finish(&cancel, SIGN_IN_FOR) {
            Ok((t, who)) => {
                let kept = t.refresh_token.as_deref().is_some_and(|r| account::remember(&p, r));
                Done::SignedIn(t, who, kept)
            }
            Err(e) => Done::Failed(e.to_string()),
        });
    }

    /// Start fetching how `who` looks, once per user.
    fn load_look(&mut self, who: &Account) {
        if self.look_for.as_deref() == Some(who.id.as_str()) {
            return;
        }
        self.look_for = Some(who.id.clone());
        self.look = Some(AvatarLook { user_id: who.id.clone(), name: who.display_name(), ..Default::default() });
        let svc = self.avatars.clone();
        let user_id = who.id.clone();
        self.spawn(move || {
            let look = av::look_up(&svc, &user_id);
            let get = |p: Option<&av::Picture>| {
                let bytes = av::fetch(p?).ok()?;
                limen_ui::avatar::decode(&bytes, limen_ui::avatar::MAX_SIDE).ok()
            };
            let effect = look.spec.as_ref().and_then(|s| s.effect.as_ref()).and_then(|e| {
                Some(EffectSpec {
                    kind: EffectKind::parse(e.name.as_str())?,
                    colors: e.color.clone().unwrap_or_default(),
                    intensity: e.intensity.unwrap_or(0.5) as f32,
                })
            });
            Done::Look { picture: get(look.icon.as_ref()), frame: get(look.frame.as_ref()), effect, user_id }
        });
    }

    /// How the signed-in user looks, when someone is.
    pub(crate) fn look(&self) -> Option<&AvatarLook> {
        self.look.as_ref()
    }

    /// Who is signed in (or being restored), and their settings page.
    pub(crate) fn who(&self) -> Option<&Account> {
        match &self.state {
            AccountState::SignedIn { who, .. } => Some(who),
            AccountState::Restoring { who } => who.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn settings_url(&self) -> Option<String> {
        self.platform.as_ref().map(Platform::account_settings_url)
    }

    fn sign_out(&mut self) {
        self.look = None;
        self.look_for = None;
        self.tokens = None;
        self.state = AccountState::SignedOut;
        save_account(None);
        if let Some(p) = self.platform.clone() {
            self.spawn(move || {
                account::forget(&p);
                Done::Restored(None)
            });
        }
    }

    /// Take in whatever the threads have finished. Once a frame.
    pub(crate) fn poll(&mut self) {
        while let Ok(done) = self.rx.try_recv() {
            match done {
                Done::SignedIn(t, who, remembered) => {
                    save_account(Some(&who));
                    self.load_look(&who);
                    self.tokens = Some(t);
                    self.state = AccountState::SignedIn { who, remembered, note: None };
                }
                Done::Restored(Some((t, who))) => {
                    save_account(Some(&who));
                    self.load_look(&who);
                    self.tokens = Some(t);
                    self.state = AccountState::SignedIn { who, remembered: true, note: None };
                }
                Done::Restored(None) => {
                    // Nothing remembered, or the platform no longer takes it.
                    // Not over a sign-in started meanwhile.
                    if matches!(self.state, AccountState::Restoring { .. }) {
                        save_account(None);
                        self.look = None;
                        self.look_for = None;
                        self.state = AccountState::SignedOut;
                    }
                }
                Done::Unreachable(why) => {
                    if let AccountState::Restoring { who } = &self.state {
                        self.state = match who.clone() {
                            Some(who) => AccountState::SignedIn { who, remembered: true, note: Some(why) },
                            None => AccountState::Failed(why),
                        };
                    }
                }
                Done::Failed(why) => {
                    if matches!(self.state, AccountState::Waiting { .. }) {
                        self.state = AccountState::Failed(why);
                    }
                }
                Done::Look { user_id, effect, picture, frame } => {
                    // Only for whoever is still the one shown.
                    if self.look_for.as_deref() == Some(user_id.as_str())
                        && let Some(look) = &mut self.look
                    {
                        look.effect = effect;
                        look.picture = picture.map(|d| Picture::upload(&self.ctx, &format!("avatar:{user_id}"), d));
                        look.frame = frame.map(|d| Picture::upload(&self.ctx, &format!("frame:{user_id}"), d));
                    }
                }
            }
        }
    }

    /// A valid access token for calling the platform, refreshed when it is
    /// about to expire. Blocking: call it off the GUI thread.
    #[allow(dead_code)] // the first caller is the platform capability for modules
    pub(crate) fn access_token(&mut self) -> Option<String> {
        let p = self.platform.clone()?;
        let t = self.tokens.as_ref()?;
        if !t.expiring() {
            return Some(t.access_token.clone());
        }
        let refresh = t.refresh_token.clone()?;
        let (fresh, _) = account::renew(&p, &refresh).ok()?;
        let token = fresh.access_token.clone();
        self.tokens = Some(fresh);
        Some(token)
    }
}

/// The account's corner of the title bar, beside the gear, as the website's
/// header has it: "Sign in" when no one is; the avatar — decorated, with its
/// card on hover — and a sign-out button when someone is. Nothing when
/// signing in is not set up, or while a session nobody is known for is checked.
/// Drawn in a right-to-left row: what comes first here sits further right.
pub(crate) fn account_chip(ui: &mut egui::Ui, acc: &AccountUi, action: &mut Option<AccountAction>) {
    match &acc.state {
        AccountState::Off | AccountState::Restoring { who: None } => {}
        AccountState::SignedOut | AccountState::Failed(_) => {
            let resp = sign_in_button(ui);
            let resp = match &acc.state {
                AccountState::Failed(why) => resp.on_hover_text(why),
                _ => resp,
            };
            if resp.clicked() {
                *action = Some(AccountAction::SignIn);
            }
        }
        AccountState::Waiting { .. } => {
            // The browser is out: a click calls it off.
            let resp = ui
                .add(egui::Spinner::new().size(18.0))
                .interact(egui::Sense::click())
                .on_hover_text(format!("{}\n{}", i18n::t("account.waiting"), i18n::t("account.cancel_hint")));
            if resp.clicked() {
                *action = Some(AccountAction::Cancel);
            }
        }
        AccountState::SignedIn { .. } | AccountState::Restoring { who: Some(_) } => {
            let (Some(who), Some(look)) = (acc.who(), acc.look()) else { return };
            let id = egui::Id::new("limen_account_chip");
            // As tall as the bar's other buttons, so the bar keeps its height;
            // the effect still runs, spilling onto a layer above the panels.
            let resp = limen_ui::avatar::avatar_from(ui, id, look, HEADER_SIZE, true, Some(egui::Order::PanelResizeLine), HEADER_SIZE);
            let url = acc.settings_url();
            // The email, and what the Settings page used to say under the account.
            let mut details: Vec<String> = Some(who.email.clone()).filter(|e| !e.is_empty()).into_iter().collect();
            if let AccountState::SignedIn { remembered, note, .. } = &acc.state {
                if !remembered {
                    details.push(i18n::t("account.not_remembered"));
                }
                if let Some(why) = note {
                    details.push(i18n::t("account.unreachable").replace("{why}", why));
                }
            }
            let link_text = i18n::t("account.settings_link");
            let sign_out = i18n::t("account.sign_out");
            let signed_out = user_card::hover_card(
                ui,
                id.with("card"),
                &resp,
                &CardInfo {
                    look,
                    name: &who.display_name(),
                    username: &who.username,
                    details: &details,
                    tag: Some(&i18n::t("account.you")),
                    link: url.as_deref().map(|u| (link_text.as_str(), u)),
                    sign_out: Some(&sign_out),
                },
            );
            if signed_out {
                *action = Some(AccountAction::SignOut);
            }
            // The avatar is a link to the account, as on the website.
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked()
                && let Some(u) = url
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(u));
            }
        }
    }
}

/// The title bar's controls: the brand mark's size.
const HEADER_SIZE: f32 = 24.0;

#[derive(Clone, Copy)]
enum Glyph {
    SignIn,
}

/// The website's sign-in mark: a door bracket and an arrow into it.
fn paint_glyph(p: &egui::Painter, rect: egui::Rect, glyph: Glyph, color: egui::Color32) {
    let s = egui::Stroke::new(1.6_f32, color);
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let w = rect.width();
    let mid = rect.center().y;
    let at = |x: f32, y: f32| egui::pos2(x, y);
    match glyph {
        Glyph::SignIn => {
            // The door on the right; the arrow comes in from the left.
            p.line_segment([at(r - w * 0.42, t), at(r, t)], s);
            p.line_segment([at(r, t), at(r, b)], s);
            p.line_segment([at(r, b), at(r - w * 0.42, b)], s);
            p.line_segment([at(l, mid), at(r - w * 0.3, mid)], s);
            p.line_segment([at(r - w * 0.3, mid), at(r - w * 0.52, mid - w * 0.22)], s);
            p.line_segment([at(r - w * 0.3, mid), at(r - w * 0.52, mid + w * 0.22)], s);
        }
    }
}

/// "SIGN IN" in the website's outlined amber, with its mark; brighter on hover.
fn sign_in_button(ui: &mut egui::Ui) -> egui::Response {
    let text = i18n::t("account.sign_in").to_uppercase();
    let font = egui::FontId::monospace(12.0);
    let galley = ui.painter().layout_no_wrap(text, font, egui::Color32::WHITE);
    let icon = 12.0;
    let size = egui::vec2(galley.size().x + icon + 8.0 + 24.0, HEADER_SIZE);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = limen_ui::anim_bool(ui, resp.id.with("hover"), resp.hovered(), 0.15);
    let ink = limen_ui::lerp_color(egui::Color32::from_rgb(0xf0, 0xa8, 0x50), egui::Color32::from_rgb(0xfb, 0xc8, 0x80), hover);
    let p = ui.painter();
    p.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(200, 120, 48, (15.0 + 15.0 * hover) as u8));
    p.rect_stroke(rect, 0.0, egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(200, 120, 48, 90)));
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0 + icon / 2.0, rect.center().y), egui::vec2(icon, icon));
    paint_glyph(p, icon_rect, Glyph::SignIn, ink);
    p.galley(egui::pos2(icon_rect.right() + 8.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn save_account(who: Option<&Account>) {
    if let Ok(mut cfg) = limen_core::Config::load()
        && cfg.account.as_ref() != who
    {
        cfg.account = who.cloned();
        let _ = cfg.save();
    }
}
