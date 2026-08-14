//! The Limen desktop shell (egui/eframe).
//!
//! The shell is deliberately thin: a sidebar of modules, and a central panel
//! that renders whatever UI the selected module describes for itself (via the
//! GUI core in [`crate::ui`]). There are no domain-specific built-in views —
//! each module draws its own window, and the core keeps the styling uniform.
//!
//! * **Overview** — the (minimal) list of installed modules.
//! * **<module>** — that module's self-described UI + a standardized result pane.
//! * **demo-ui** — the component/style gallery (debug builds only).
//!
//! All engine work is on the [`Worker`] thread, so the UI never blocks.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use limen_core::{ModuleSpec, Runtime};
use limen_registry::RemoteModule;

use crate::i18n;
use crate::ui;
use crate::worker::{Command, Event, RunTag, Worker};

// The shell is one module but not one file. Each of these is a piece of it;
// the re-exports keep every name reachable from where it was, so splitting
// the file moved code and nothing else.
mod brand;
mod dialogs;
// Nothing to re-export: it holds the `eframe::App` impl and no names of its
// own.
mod frame;
mod pages;
mod search;
mod tabs;

pub use brand::*;
pub(crate) use dialogs::*;
pub(crate) use pages::*;
pub use search::*;
pub use tabs::*;

/// A path a file dialog returned: `(widget id, chosen path)`.
pub(crate) type FilePick = (String, String);

/// Max lines kept in the debug console.
pub(crate) const LOG_CAP: usize = 2000;

pub struct LimenApp {
    pub(crate) worker: Worker,
    pub(crate) status: String,
    pub(crate) fatal: Option<String>,
    pub(crate) modules: Vec<ModuleSpec>,
    /// Names of installed modules that came from a git install (vs. manual).
    pub(crate) git_installed: HashSet<String>,
    /// name → (branch, short commit) for git-installed modules.
    pub(crate) git_meta: HashMap<String, (String, String)>,
    /// Installed git modules with a newer release available: name → latest version.
    pub(crate) available_updates: HashMap<String, String>,
    /// Modules that failed to start: name → error (shown in the module's tab).
    pub(crate) failed: HashMap<String, String>,
    /// Names of modules the user has granted their declared permissions
    /// (trusted at their current content digest).
    pub(crate) trusted: HashSet<String>,
    /// The consent dialog still on screen, which outlives `pending_action` by
    /// the length of its closing animation.
    pub(crate) consent_showing: Option<ui::Invoke>,
    /// A close was asked for and is waiting on an answer.
    pub(crate) quit_asking: bool,
    /// The answer was yes, so the next close request goes through.
    pub(crate) quit_confirmed: bool,
    /// The tab content area, as of the last frame that drew one. Pop-ups are
    /// scoped to it rather than to the window.
    pub(crate) content_rect: Option<egui::Rect>,
    /// What each module tab was showing when it was last active, keyed by module
    /// name. The active tab's own state lives in the fields above; this is where
    /// the others wait.
    pub(crate) module_pages: HashMap<String, ModulePage>,
    /// A module action waiting on the question its button carried, and the one
    /// still fading out — the same pair as the consent dialog, for the same
    /// reason: an answered question has to keep drawing while it leaves.
    pub(crate) pending_confirm: Option<ui::Invoke>,
    pub(crate) confirm_showing: Option<ui::Invoke>,
    /// An elevated action awaiting the user's consent (shown as a dialog).
    pub(crate) pending_action: Option<ui::Invoke>,
    /// A module the user asked to remove, held until they confirm. Removal
    /// deletes the module's directory — including anything it had fetched into
    /// `tools/` — so it is not something to do on a stray click.
    pub(crate) pending_remove: Option<String>,
    /// The name shown in the confirmation, kept while it animates closed.
    pub(crate) confirm_subject: Option<String>,

    /// Open detail tabs (from row actions), keyed by the id in `Tab::Detail`.
    pub(crate) detail_tabs: HashMap<u64, DetailTab>,
    /// Monotonic id for the next detail tab.
    pub(crate) next_detail_id: u64,

    /// Open tabs (in order) and the active index.
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,

    pub(crate) view: Option<ui::View>,
    pub(crate) view_error: Option<String>,
    /// Module pop-ups, innermost last. A view that arrives with `modal` set is
    /// pushed here instead of replacing the screen behind it.
    pub(crate) modal_stack: Vec<ui::View>,
    /// The pop-up that was just closed, kept only until it has finished
    /// animating away — otherwise it would vanish instead of leaving.
    pub(crate) modal_closing: Option<ui::View>,
    pub(crate) inputs: HashMap<String, String>,
    pub(crate) output: String,
    pub(crate) busy: bool,
    /// The action currently in flight (its button shows a spinner).
    pub(crate) busy_action: Option<ui::Action>,

    // Modules page state
    pub(crate) search: String,
    pub(crate) filter: ModuleFilter,
    // Modules available in the GitHub org
    pub(crate) remote: Vec<RemoteModule>,
    pub(crate) remote_error: Option<String>,
    pub(crate) remote_loading: bool,
    pub(crate) remote_fetched: bool,
    /// The repo currently being installed, if any (disables Install + shows a spinner).
    pub(crate) installing: Option<String>,

    // Developer tab
    pub(crate) dev_tab: DevTab,
    pub(crate) logs: std::collections::VecDeque<String>,
    pub(crate) log_autoscroll: bool,

    /// Dev mode: source app/module updates from local dirs instead of GitHub.
    /// Session-only — never persisted, so it resets on restart.
    pub(crate) dev_mode_on: bool,
    pub(crate) dev_limen_path: String,
    pub(crate) dev_modules_path: String,

    /// Global UI scale as a percentage (persisted in settings).
    pub(crate) ui_scale: f32,

    /// Whether UI animations are enabled (persisted in settings).
    pub(crate) animations: bool,
    /// Whether passing notices are shown (persisted in settings).
    pub(crate) alerts: bool,

    /// The active UI language (persisted in settings; mirrors `i18n`'s global).
    pub(crate) language: i18n::Lang,

    /// When the About tab was last shown — drives its staggered content reveal.
    pub(crate) about_revealed_at: Option<f64>,
    /// Reveal timers for the Settings/Developer/License tab entrance animations.
    pub(crate) settings_revealed_at: Option<f64>,
    pub(crate) developer_revealed_at: Option<f64>,
    /// Whether the changelog pop-up is showing, and whether it is still on
    /// screen finishing its exit.
    pub(crate) changes_open: bool,
    pub(crate) changes_alive: bool,
    /// Whether the license pop-up is showing.
    /// A notice a module asked for, waiting for a frame to raise it in — a
    /// view arrives on the worker's message, which has no egui context.
    pub(crate) pending_notice: Option<(ui::toast::Level, String)>,
    pub(crate) license_open: bool,
    /// Whether it is still on screen — it has an exit animation to finish after
    /// it stops being open.
    pub(crate) license_alive: bool,
    /// Which module the current view entrance was armed for, and when it began.
    /// Switching to a *different* module rearms it, so each module plays its
    /// staggered entrance rather than snapping into place.
    pub(crate) module_reveal: Option<(String, f64)>,
    /// Results from native file dialogs. The dialog call blocks until the user
    /// answers, so it runs on a thread of its own and reports back here — the UI
    /// thread only ever drains this channel.
    pub(crate) file_pick: (mpsc::Sender<FilePick>, mpsc::Receiver<FilePick>),
    /// Developer sub-tab the reveal was started for; a change replays it.
    pub(crate) shown_dev_tab: DevTab,
    /// When the Modules tab was last shown — drives the staggered list reveal.
    pub(crate) modules_revealed_at: Option<f64>,
    /// The filter the current reveal was started for; a change replays the reveal.
    pub(crate) shown_filter: ModuleFilter,
    /// Arrival time per available module — the cards stream in from GitHub in
    /// parallel (out of order), so each animates in from when *it* arrived.
    pub(crate) remote_arrivals: HashMap<String, f64>,

    /// Modules mid-removal: name → animation start time (or a negative sentinel
    /// once the actual removal has been sent). Drives the exit animation.
    pub(crate) removing: HashMap<String, f64>,

    /// An available app update (from the background check), if any.
    pub(crate) update: Option<limen_core::UpdateInfo>,
    /// True while an update download/install is in flight.
    pub(crate) updating: bool,
    /// A portable interpreter currently being installed (e.g. "Python"), if any.
    pub(crate) installing_runtime: Option<String>,
    /// Startup splash: the frame time it first rendered (set lazily on the first
    /// frame), and whether the ~2s animated intro has finished. Once done, the
    /// normal UI takes over.
    pub(crate) splash_start: Option<f64>,
    pub(crate) splash_done: bool,
    /// Whether the window has been centered + revealed yet (it's created hidden
    /// to avoid a dark startup flash).
    pub(crate) window_shown: bool,
    /// How many startup frames have pre-warmed the font atlas so far (warmed for
    /// the first several, after zoom/DPI settles, then stops).
    pub(crate) fonts_warm_frames: u32,
    /// Tab strip horizontal scroll state (single row; overflow → arrows + wheel).
    /// `scroll` is the current offset; `content_w`/`view_w` (from last frame) drive
    /// arrow visibility/enablement; `scroll_to` is a pending offset from an arrow.
    pub(crate) tab_scroll: f32,
    pub(crate) tab_content_w: f32,
    pub(crate) tab_view_w: f32,
    pub(crate) tab_scroll_to: Option<f32>,
}

impl LimenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Vec<std::path::PathBuf>) -> Self {
        ui::apply_theme(&cc.egui_ctx);
        let animations = limen_core::Config::load()
            .map(|c| c.animations)
            .unwrap_or(true);
        ui::set_animations(animations);
        let alerts = limen_core::Config::load().map(|c| c.alerts).unwrap_or(true);
        ui::toast::set_enabled(alerts);
        // `ui` holds no catalog, so it does not know what to call the four
        // levels. It asks this, and asks again every frame — so a notice that is
        // already up follows a change of language.
        ui::toast::set_namer(|level| {
            i18n::t(match level {
                ui::toast::Level::Info => "toast.info",
                ui::toast::Level::Ok => "toast.ok",
                ui::toast::Level::Warning => "toast.warning",
                ui::toast::Level::Error => "toast.error",
            })
        });
        // Resolve the UI language: saved choice → OS locale → English.
        let language = limen_core::Config::load()
            .ok()
            .and_then(|c| c.language)
            .and_then(|code| i18n::Lang::from_code(&code))
            .unwrap_or_else(i18n::detect);
        i18n::set_locale(language);
        // Share it with the module host + registry (host.locale callback, and the
        // localized-description lookup when listing modules).
        limen_proto::locale::set(language.code());
        // Apply the admin's GitHub token (if set) to the module registry before the
        // first request; absent = unauthenticated (the default). Point the registry
        // at ~/.limen for its conditional-request (ETag) cache.
        limen_registry::set_registry_cache_dir(limen_core::paths::home());
        limen_registry::set_github_token(
            limen_core::Config::load().ok().and_then(|c| c.github_token),
        );
        Self {
            worker: Worker::spawn(dirs),
            status: "starting modules…".to_string(),
            fatal: None,
            modules: Vec::new(),
            git_installed: HashSet::new(),
            git_meta: HashMap::new(),
            available_updates: HashMap::new(),
            failed: HashMap::new(),
            trusted: HashSet::new(),
            pending_action: None,
            pending_remove: None,
            confirm_subject: None,
            detail_tabs: HashMap::new(),
            next_detail_id: 0,
            tabs: vec![Tab::About, Tab::Modules],
            active: 0,
            view: None,
            view_error: None,
            consent_showing: None,
            quit_asking: false,
            quit_confirmed: false,
            content_rect: None,
            module_pages: HashMap::new(),
            pending_confirm: None,
            confirm_showing: None,
            modal_stack: Vec::new(),
            modal_closing: None,
            inputs: HashMap::new(),
            output: String::new(),
            busy: false,
            busy_action: None,
            search: String::new(),
            filter: ModuleFilter::All,
            remote: Vec::new(),
            remote_error: None,
            remote_loading: false,
            remote_fetched: false,
            installing: None,
            dev_tab: DevTab::DevMode,
            logs: std::collections::VecDeque::new(),
            log_autoscroll: true,
            dev_mode_on: false,
            dev_limen_path: String::new(),
            dev_modules_path: String::new(),
            ui_scale: {
                let pct = limen_core::Config::load()
                    .map(|c| c.ui_scale_percent)
                    .unwrap_or(0);
                if pct == 0 { 100.0 } else { pct as f32 }
            },
            animations,
            alerts,
            language,
            about_revealed_at: None,
            settings_revealed_at: None,
            developer_revealed_at: None,
            changes_open: false,
            changes_alive: false,
            pending_notice: None,
            license_open: false,
            license_alive: false,
            module_reveal: None,
            file_pick: mpsc::channel(),
            shown_dev_tab: DevTab::DevMode,
            modules_revealed_at: None,
            shown_filter: ModuleFilter::All,
            remote_arrivals: HashMap::new(),
            removing: HashMap::new(),
            update: None,
            updating: false,
            installing_runtime: None,
            splash_start: None,
            splash_done: false,
            window_shown: false,
            fonts_warm_frames: 0,
            tab_scroll: 0.0,
            tab_content_w: 0.0,
            tab_view_w: 0.0,
            tab_scroll_to: None,
        }
    }

    /// The active tab (cloned).
    pub(crate) fn active_tab(&self) -> Option<Tab> {
        self.tabs.get(self.active).cloned()
    }

    /// Lift the page currently in the fields out of them.
    pub(crate) fn take_page(&mut self) -> ModulePage {
        ModulePage {
            view: self.view.take(),
            view_error: self.view_error.take(),
            modal_stack: std::mem::take(&mut self.modal_stack),
            modal_closing: self.modal_closing.take(),
            inputs: std::mem::take(&mut self.inputs),
            output: std::mem::take(&mut self.output),
            busy: std::mem::take(&mut self.busy),
            busy_action: self.busy_action.take(),
        }
    }

    /// Put a page into the fields, which is what the frame draws from.
    pub(crate) fn put_page(&mut self, p: ModulePage) {
        self.view = p.view;
        self.view_error = p.view_error;
        self.modal_stack = p.modal_stack;
        self.modal_closing = p.modal_closing;
        self.inputs = p.inputs;
        self.output = p.output;
        self.busy = p.busy;
        self.busy_action = p.busy_action;
    }

    /// Put the active module tab's state away, ready for another to take the
    /// fields.
    pub(crate) fn stash_page(&mut self) {
        let from = match self.active_tab() {
            Some(Tab::Module(name)) => Some(name),
            _ => None,
        };
        let cur = self.take_page();
        let next = swap_page(cur, &mut self.module_pages, from.as_deref(), None);
        self.put_page(next);
    }

    /// Take back what a module tab was showing, or start it empty.
    pub(crate) fn restore_page(&mut self, name: &str) {
        let cur = self.take_page();
        let to = (!name.is_empty()).then_some(name);
        let next = swap_page(cur, &mut self.module_pages, None, to);
        self.put_page(next);
    }

    /// The one door every tab change goes through, so a module tab's state is
    /// always put away before another takes the fields.
    pub(crate) fn activate(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.stash_page();
        self.active = index;
        match self.active_tab() {
            Some(Tab::Module(name)) => {
                self.restore_page(&name);
                self.resume_page();
            }
            // Not a module tab: leave the fields empty rather than showing the
            // last module's view behind an About page.
            _ => self.restore_page(""),
        }
    }

    /// Open `tab` (focus it if already open, else append), and activate it.
    pub(crate) fn open_tab(&mut self, tab: Tab) {
        match self.tabs.iter().position(|t| *t == tab) {
            Some(i) => self.activate(i),
            None => {
                self.stash_page();
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
                match self.active_tab() {
                    Some(Tab::Module(name)) => {
                        self.restore_page(&name);
                        self.resume_page();
                    }
                    _ => self.restore_page(""),
                }
            }
        }
    }

    /// Close the tab at `index`.
    pub(crate) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        // Free a tab's stored view when it closes.
        match &self.tabs[index] {
            Tab::Detail { id } => {
                self.detail_tabs.remove(id);
            }
            Tab::Module(name) => {
                self.module_pages.remove(name);
                // Closing the tab you are looking at also clears the fields, or
                // its view would linger under the next one.
                if index == self.active {
                    self.view = None;
                    self.view_error = None;
                    self.modal_stack.clear();
                    self.modal_closing = None;
                    self.inputs.clear();
                    self.output.clear();
                    self.busy = false;
                    self.busy_action = None;
                }
            }
            _ => {}
        }
        self.tabs.remove(index);
        let was = self.active;
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if self.active > index {
            self.active -= 1;
        }
        // Closing a tab lands you on another one, which has to be given its own
        // state back just as switching to it would.
        if was != self.active || was == index {
            match self.active_tab() {
                Some(Tab::Module(name)) => {
                    self.restore_page(&name);
                    self.resume_page();
                }
                _ => self.restore_page(""),
            }
        }
    }

    /// Pick up a chain the tab was in the middle of.
    ///
    /// A view that carries an `auto` was polling something — a scan, an install.
    /// Restoring it puts the picture back but not the loop, so the loop is
    /// started again; otherwise a scan left running in one tab would be frozen
    /// mid-progress on return.
    pub(crate) fn resume_page(&mut self) {
        if let Some(a) = self.view.as_ref().and_then(|v| v.auto.clone()) {
            self.dispatch(a.into_invoke());
        }
    }

    /// Persist the current UI scale to settings.json (without clobbering others).
    pub(crate) fn save_ui_scale(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.ui_scale_percent = self.ui_scale.round() as u32;
            let _ = cfg.save();
        }
    }

    /// Persist the animations toggle to settings.json (without clobbering others).
    pub(crate) fn save_animations(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.animations = self.animations;
            let _ = cfg.save();
        }
    }

    pub(crate) fn save_alerts(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.alerts = self.alerts;
            let _ = cfg.save();
        }
    }

    /// Apply the chosen language globally and persist it to settings.json.
    ///
    /// The host's own chrome re-renders from the catalogue every frame, so it
    /// follows immediately. A module's screen does not: it is a stored view of
    /// strings the module already translated, and it only changes language when
    /// the module is asked again. So every cached view is dropped and the module
    /// on screen is re-asked — otherwise the app sits in two languages at once
    /// until something happens to invoke it.
    pub(crate) fn save_language(&mut self) {
        i18n::set_locale(self.language);
        limen_proto::locale::set(self.language.code());
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.language = Some(self.language.code().to_string());
            let _ = cfg.save();
        }

        // Tabs that are not on screen: drop what they were showing, so each is
        // rebuilt in the new language when it is next opened.
        self.module_pages.clear();

        // The tab on screen: ask its module for its view again. `ui` is a
        // module's landing screen, so this returns the user to it — a language
        // change is rare and deliberate, and a screen half in the old language
        // is worse than starting a module over.
        if let Some(Tab::Module(name)) = self.active_tab() {
            self.view = None;
            self.view_error = None;
            self.modal_stack.clear();
            self.modal_closing = None;
            self.output.clear();
            self.busy = false;
            self.busy_action = None;
            self.select_module(name);
        }
    }

    /// Apply any path the user chose in a file dialog to its widget's input.
    pub(crate) fn drain_file_picks(&mut self) {
        while let Ok((id, path)) = self.file_pick.1.try_recv() {
            self.inputs.insert(id, path);
        }
    }

    /// Run a pending Browse request from a [`ui::Widget::File`].
    ///
    /// `rfd`'s dialog blocks until the user answers it, which would freeze the
    /// render loop — so it goes on its own thread and the result comes back
    /// through `file_pick`. A cancelled dialog simply sends nothing.
    pub(crate) fn serve_browse_request(&mut self, ctx: &egui::Context) {
        let req: Option<(String, bool)> = ctx.data_mut(|d| {
            let id = ui::browse_request_id();
            let v = d.get_temp::<(String, bool)>(id);
            if v.is_some() {
                d.remove::<(String, bool)>(id);
            }
            v
        });
        let Some((id, directory)) = req else { return };
        let tx = self.file_pick.0.clone();
        let start = self.inputs.get(&id).cloned().unwrap_or_default();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut dlg = rfd::FileDialog::new();
            // Reopen where the last pick left off, when that path still exists.
            let at = std::path::Path::new(&start);
            if let Some(dir) = at.parent().filter(|p| p.is_dir()) {
                dlg = dlg.set_directory(dir);
            }
            let picked = if directory {
                dlg.pick_folder()
            } else {
                dlg.pick_file()
            };
            if let Some(p) = picked {
                let _ = tx.send((id, p.display().to_string()));
                // The dialog stole focus; wake the app so the new path paints.
                ctx.request_repaint();
            }
        });
    }

    /// Draw the removal confirmation and report the name once the user agrees.
    ///
    /// Called every frame, not only while something is pending: the dialog
    /// animates itself out, and it can only do that if it is still being drawn
    /// after the answer. `confirm_subject` holds the name for those last frames,
    /// or the box would blink empty as it leaves.
    pub(crate) fn confirmed_removal(&mut self, ctx: &egui::Context) -> Option<String> {
        // Show what the user recognises — the module's display name, localized —
        // rather than the identifier they never chose.
        if let Some(name) = &self.pending_remove {
            let shown = self
                .modules
                .iter()
                .find(|m| &m.name == name)
                .map(|m| localized_name(ctx, m))
                .unwrap_or_else(|| name.clone());
            self.confirm_subject = Some(shown);
        }
        let answer = ui::confirm_dialog(
            ctx,
            "removal",
            self.pending_remove.is_some(),
            &i18n::t("confirm.remove_title"),
            self.confirm_subject.as_deref(),
            &i18n::t("confirm.remove_yes"),
            &i18n::t("confirm.cancel"),
            self.content_rect,
        );
        match answer {
            Some(true) => self.pending_remove.take(),
            Some(false) => {
                self.pending_remove = None;
                None
            }
            None => None,
        }
    }

    /// Recompute which sensitive modules the user has granted (trusted at their
    /// current digest). Cheap enough to run on module load / after a grant.
    /// The module that provides `capability`, if any.
    pub(crate) fn module_of(&self, capability: &str) -> Option<&ModuleSpec> {
        self.modules
            .iter()
            .find(|m| m.capabilities.iter().any(|c| c == capability))
    }

    /// Does this action invoke an elevated method on a module the user hasn't
    /// granted yet? If so, it needs a consent prompt first.
    pub(crate) fn action_needs_consent(&self, a: &ui::Action) -> bool {
        match self.module_of(&a.capability) {
            Some(m) => {
                m.permissions.method_needs_consent(&a.method) && !self.trusted.contains(&m.name)
            }
            None => false,
        }
    }

    /// Grant a module its declared permissions: pin trust to its current digest,
    /// persist, and update the cached trusted set. Returns success.
    pub(crate) fn grant_trust(&mut self, name: &str) -> bool {
        let Some(spec) = self.modules.iter().find(|m| m.name == name) else {
            return false;
        };
        let Ok(digest) = limen_registry::digest_dir(&spec.cwd) else {
            return false;
        };
        let mut trust =
            limen_registry::TrustStore::load(&limen_core::paths::home()).unwrap_or_default();
        trust.approve(name, &digest);
        if trust.save(&limen_core::paths::home()).is_ok() {
            self.trusted.insert(name.to_string());
            true
        } else {
            false
        }
    }

    pub(crate) fn push_log(&mut self, line: String) {
        self.logs.push_back(line);
        while self.logs.len() > LOG_CAP {
            self.logs.pop_front();
        }
    }

    pub(crate) fn drain_events(&mut self, now: f64) {
        while let Ok(evt) = self.worker.rx.try_recv() {
            match evt {
                Event::Ready(snap) | Event::Modules(snap) => {
                    self.modules = snap.specs;
                    self.git_installed = snap.git_installed.into_iter().collect();
                    self.git_meta = snap.git_meta;
                    self.failed = snap.failed;
                    // Trust was digested on the worker thread — just take the result.
                    self.trusted = snap.trusted.into_iter().collect();
                    // A reload follows a completed install/remove — clear the spinner.
                    self.installing = None;
                    self.status = format!("{} module(s) loaded", self.modules.len());
                }
                Event::ModuleUpdates(map) => {
                    self.available_updates = map;
                }
                Event::RuntimeInstalling(rt) => {
                    self.installing_runtime = rt;
                }
                Event::RemoteFound(m) => {
                    // Streamed in as fetched (parallel, out of order). Append,
                    // keep alphabetical, and stamp arrival for the entrance anim.
                    if !self.remote.iter().any(|r| r.name == m.name) {
                        self.remote_arrivals.insert(m.name.clone(), now);
                        self.remote.push(m);
                        self.remote.sort_by(|a, b| a.name.cmp(&b.name));
                    }
                    self.remote_error = None;
                }
                Event::RemoteDone(result) => {
                    self.remote_loading = false;
                    if let Err(e) = result {
                        self.remote_error = Some(e);
                    }
                }
                Event::RunDone { tag, result } => match tag {
                    RunTag::Ui { module } => {
                        // A result for a tab the user has since left still has to
                        // land: its tab keeps its own state now, and dropping it
                        // would leave that tab waiting on a view that already
                        // arrived — and, because it would still look busy, never
                        // ask for another.
                        if self.active_tab() != Some(Tab::Module(module.clone())) {
                            let page = self.module_pages.entry(module).or_default();
                            page.busy = false;
                            match result {
                                Ok(v) => match serde_json::from_value::<ui::View>(v) {
                                    Ok(view) => {
                                        // Stored, not chained: a background tab's
                                        // `auto` loop resumes when it is next
                                        // shown (see `resume_page`).
                                        page.view = Some(view);
                                        page.view_error = None;
                                    }
                                    Err(e) => {
                                        page.view_error = Some(format!("invalid UI spec: {e}"))
                                    }
                                },
                                Err(e) => page.view_error = Some(e),
                            }
                            return;
                        }
                        if self.active_tab() == Some(Tab::Module(module)) {
                            self.busy = false;
                            match result {
                                Ok(v) => match serde_json::from_value::<ui::View>(v) {
                                    Ok(view) => {
                                        let auto = view.auto.clone();
                                        self.accept_view(view);
                                        // Chain the next step, if the view asked for one.
                                        if let Some(a) = auto {
                                            self.dispatch(a.into_invoke());
                                        }
                                    }
                                    Err(e) => {
                                        self.view_error = Some(format!("invalid UI spec: {e}"));
                                    }
                                },
                                Err(e) => {
                                    self.view_error = Some(if e.contains("unknown method") {
                                        "This module does not provide a UI.".to_string()
                                    } else {
                                        e
                                    });
                                }
                            }
                        }
                    }
                    RunTag::Action => {
                        self.busy = false;
                        self.busy_action = None;
                        match result {
                            // A method may return a *view* (object with "widgets")
                            // to re-render the module UI in place (e.g. Refresh /
                            // Search). Otherwise the result is shown as output.
                            Ok(v) if v.get("widgets").is_some() => {
                                match serde_json::from_value::<ui::View>(v) {
                                    Ok(view) => {
                                        let auto = view.auto.clone();
                                        self.accept_view(view);
                                        self.output.clear();
                                        // Chain the next step, if the view asked for one.
                                        if let Some(a) = auto {
                                            self.dispatch(a.into_invoke());
                                        }
                                    }
                                    Err(e) => self.output = format!("invalid view: {e}"),
                                }
                            }
                            // A null result is a fire-and-forget acknowledgement
                            // (e.g. "open path") — nothing to show in the Result pane.
                            Ok(v) if v.is_null() => self.output.clear(),
                            Ok(v) => {
                                self.output = serde_json::to_string_pretty(&v)
                                    .unwrap_or_else(|e| e.to_string())
                            }
                            Err(e) => self.output = format!("error: {e}"),
                        }
                        self.status = "done".to_string();
                    }
                    RunTag::Detail { id } => {
                        // Fill the detail tab, if it's still open.
                        if let Some(tab) = self.detail_tabs.get_mut(&id) {
                            tab.busy = false;
                            match result {
                                Ok(v) => match serde_json::from_value::<ui::View>(v) {
                                    Ok(view) => {
                                        if !view.title.is_empty() {
                                            tab.title = view.title.clone();
                                        }
                                        tab.view = Some(view);
                                        tab.error = None;
                                    }
                                    Err(e) => tab.error = Some(format!("invalid view: {e}")),
                                },
                                Err(e) => tab.error = Some(format!("error: {e}")),
                            }
                        }
                    }
                },
                Event::Status(msg) => {
                    self.busy = false;
                    // A failed update leaves the Update button spinning — stop it.
                    if msg.starts_with("update failed") {
                        self.updating = false;
                    }
                    self.push_log(format!("[status] {msg}"));
                    self.status = msg;
                }
                Event::Log(line) => self.push_log(line),
                Event::UpdateAvailable(info) => {
                    self.push_log(format!("[update] v{} available", info.latest));
                    self.update = Some(info);
                }
                Event::Fatal(e) => {
                    self.push_log(format!("[fatal] {e}"));
                    self.fatal = Some(e);
                    self.status = "failed to start".to_string();
                }
            }
        }
    }

    pub(crate) fn select_module(&mut self, name: String) {
        // Opening the tab restores whatever it was showing, pop-up included.
        self.open_tab(Tab::Module(name.clone()));
        // Coming back to a tab that already has a screen: leave it alone. Asking
        // the module for a fresh `ui` would throw away where the user was — a
        // half-filled form, an open settings pop-up, a finished scan's results.
        if self.view.is_some() || self.view_error.is_some() || self.busy {
            return;
        }
        // A module that failed to start has no live connection — show why, here,
        // instead of trying to call it (or blocking the whole app).
        if let Some(err) = self.failed.get(&name) {
            self.view_error = Some(format!("{}\n\n{err}", i18n::t("module.failed_start")));
            return;
        }
        match self.first_capability(&name) {
            Some(cap) => {
                self.busy = true;
                self.worker.send(Command::Run {
                    tag: RunTag::Ui { module: name },
                    capability: cap,
                    method: "ui".to_string(),
                    params: serde_json::json!({}),
                });
            }
            None => self.view_error = Some("This module provides no capability.".to_string()),
        }
    }

    /// Take a view a module returned for the module page.
    ///
    /// A `modal` view opens over what is already there; anything else replaces
    /// the screen and closes whatever pop-ups were open — a module that returns
    /// a fresh screen has moved on, so leaving a pop-up floating over it would
    /// strand the user on a form belonging to the previous one.
    pub(crate) fn accept_view(&mut self, view: ui::View) {
        // Whatever the module wants said about this arriving. Taken here rather
        // than at draw time: a view redraws many times, and each redraw is not
        // a new event.
        if let Some(n) = &view.notice {
            self.pending_notice = Some((n.level(), n.text.clone()));
        }
        if let Some(id) = view.modal.clone() {
            // Already open: redraw that pop-up where it stands, and close
            // anything that was opened over it — going back to a form means
            // leaving what it led to.
            if let Some(at) = self
                .modal_stack
                .iter()
                .position(|v| v.modal.as_deref() == Some(id.as_str()))
            {
                for v in self.modal_stack.split_off(at) {
                    self.forget_inputs(&v);
                }
            }
            // A module in a loop could otherwise stack pop-ups without end.
            const MAX_DEPTH: usize = 8;
            if self.modal_stack.len() < MAX_DEPTH {
                self.modal_stack.push(view);
            }
        } else {
            for v in std::mem::take(&mut self.modal_stack) {
                self.forget_inputs(&v);
            }
            self.view = Some(view);
        }
        self.view_error = None;
    }

    /// Close the pop-up entirely, however many steps deep it went.
    ///
    /// The cross means "I am done here" — three steps into a settings form that
    /// should not mean "take me back two". The back arrow is for that.
    pub(crate) fn close_modals(&mut self) {
        let top = self.modal_stack.last().cloned();
        for v in std::mem::take(&mut self.modal_stack) {
            self.forget_inputs(&v);
        }
        self.modal_closing = top;
    }

    /// Drop the field values belonging to a pop-up that has gone.
    ///
    /// A widget's default only seeds its entry the first time it is drawn, so
    /// leaving these behind would make Cancel a no-op: the pop-up would reopen
    /// showing exactly the edits it was meant to throw away.
    pub(crate) fn forget_inputs(&mut self, view: &ui::View) {
        for id in ui::widget_ids(view) {
            self.inputs.remove(&id);
        }
    }

    /// Close the top pop-up, keeping it just long enough to animate away.
    pub(crate) fn dismiss_modal(&mut self) {
        // Only the last one out animates: closing one pop-up to reveal another
        // behind it is a change of contents, not the layer going away.
        if let Some(v) = self.modal_stack.pop() {
            self.forget_inputs(&v);
            if self.modal_stack.is_empty() {
                self.modal_closing = Some(v);
            }
        }
    }

    pub(crate) fn dispatch(&mut self, invoke: ui::Invoke) {
        // A button that carries a question is not run until it is answered. The
        // module is never told about the click, so it cannot skip asking.
        if invoke.confirm.is_some() {
            self.pending_confirm = Some(invoke);
            return;
        }
        // Base params come from the active view's inputs (a module tab's search
        // box etc.); a detail tab has no shared inputs. Row/menu args (the row
        // `id`, `via`, …) are merged on top.
        let mut params: serde_json::Map<String, serde_json::Value> = match self.active_tab() {
            Some(Tab::Module(_)) => {
                // The screen behind, then each pop-up over it: a settings pop-up
                // has to send what was typed into it, and where both define a
                // field the one in front is the one the user just edited.
                let mut m = serde_json::Map::new();
                for v in self.view.iter().chain(self.modal_stack.iter()) {
                    if let serde_json::Value::Object(o) = ui::collect_params(v, &self.inputs) {
                        m.extend(o);
                    }
                }
                m
            }
            // A detail tab has its own view + inputs (e.g. a config form).
            Some(Tab::Detail { id }) => match self
                .detail_tabs
                .get(&id)
                .and_then(|t| t.view.as_ref().map(|v| ui::collect_params(v, &t.inputs)))
            {
                Some(serde_json::Value::Object(m)) => m,
                _ => serde_json::Map::new(),
            },
            _ => serde_json::Map::new(),
        };
        for (k, v) in &invoke.args {
            params.insert(k.clone(), v.clone());
        }
        let params = serde_json::Value::Object(params);
        let ui::Action { capability, method } = invoke.action.clone();

        if invoke.open_in_tab {
            // Open (or focus) a fresh detail tab and load it in the background.
            let id = self.next_detail_id;
            self.next_detail_id += 1;
            self.detail_tabs.insert(
                id,
                DetailTab {
                    title: method.clone(),
                    busy: true,
                    ..Default::default()
                },
            );
            self.open_tab(Tab::Detail { id });
            self.status = format!("{capability}.{method}");
            self.worker.send(Command::Run {
                tag: RunTag::Detail { id },
                capability,
                method,
                params,
            });
            return;
        }

        self.busy = true;
        self.busy_action = Some(invoke.action.clone());
        self.output.clear(); // the button spinner shows progress, not the Result pane
        self.status = format!("{capability}.{method}");
        self.worker.send(Command::Run {
            tag: RunTag::Action,
            capability,
            method,
            params,
        });
    }

    pub(crate) fn first_capability(&self, name: &str) -> Option<String> {
        self.modules
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| m.capabilities.first().cloned())
    }
}
