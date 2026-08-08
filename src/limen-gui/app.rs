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

/// An open tab. All tabs are closable.
#[derive(Clone, PartialEq, Debug)]
enum Tab {
    About,
    License,
    Modules,
    Module(String),
    Settings,
    Developer,
    Update,
    /// A device/detail view opened from a row action (keyed into `detail_tabs`).
    Detail {
        id: u64,
    },
}

impl Tab {
    fn title(&self) -> String {
        match self {
            Tab::About => i18n::t("tab.about"),
            Tab::License => i18n::t("tab.license"),
            Tab::Modules => i18n::t("tab.modules"),
            Tab::Module(n) => n.clone(),
            Tab::Settings => i18n::t("tab.settings"),
            Tab::Developer => i18n::t("tab.developer"),
            Tab::Update => i18n::t("tab.update"),
            // The real label comes from the stored view's title (looked up in the
            // tab bar); this is only a fallback.
            Tab::Detail { .. } => i18n::t("tab.details"),
        }
    }
}

/// Move the page on screen aside and bring another forward.
///
/// The whole rule of per-tab state in one place: what a tab was showing is kept
/// under its own name, and what the next tab was showing is handed back. A `from`
/// of `None` means the page being left belongs to no module tab and is simply
/// dropped; a `to` of `None` means the tab being opened is not a module tab and
/// starts empty — otherwise the last module's view would show behind an About
/// page.
fn swap_page(
    current: ModulePage,
    stored: &mut HashMap<String, ModulePage>,
    from: Option<&str>,
    to: Option<&str>,
) -> ModulePage {
    if let Some(name) = from {
        stored.insert(name.to_string(), current);
    }
    to.and_then(|name| stored.remove(name)).unwrap_or_default()
}

/// Everything a module tab holds while it is not the one on screen.
///
/// The module page's state used to be the app's, so switching tabs threw it away
/// and rebuilt it — which meant a pop-up open in one tab could not survive a
/// glance at another. Each module tab keeps its own, and the app swaps the
/// active one in and out.
#[derive(Default)]
struct ModulePage {
    view: Option<ui::View>,
    view_error: Option<String>,
    modal_stack: Vec<ui::View>,
    modal_closing: Option<ui::View>,
    inputs: HashMap<String, String>,
    output: String,
    /// The scan (or whatever) this tab is waiting on, so its spinner survives a
    /// look at another tab.
    busy: bool,
    busy_action: Option<ui::Action>,
}

/// A detail tab's content: a module-returned [`ui::View`] opened from a row
/// action (e.g. "About device"), with its own inputs and load state.
#[derive(Default)]
struct DetailTab {
    title: String,
    view: Option<ui::View>,
    error: Option<String>,
    inputs: HashMap<String, String>,
    busy: bool,
}

/// The full license text, embedded so it's always in sync with the repo.
const LICENSE_TEXT: &str = include_str!("../../LICENSE");

/// A path a file dialog returned: `(widget id, chosen path)`.
type FilePick = (String, String);

/// The Modules-page installed/available filter.
#[derive(Clone, Copy, PartialEq)]
enum ModuleFilter {
    All,
    Installed,
    Available,
}

/// Tabs inside the Developer window.
#[derive(Clone, Copy, PartialEq)]
enum DevTab {
    DevMode,
    UiKit,
    Console,
}

/// Max lines kept in the debug console.
const LOG_CAP: usize = 2000;

pub struct LimenApp {
    worker: Worker,
    status: String,
    fatal: Option<String>,
    modules: Vec<ModuleSpec>,
    /// Names of installed modules that came from a git install (vs. manual).
    git_installed: HashSet<String>,
    /// name → (branch, short commit) for git-installed modules.
    git_meta: HashMap<String, (String, String)>,
    /// Installed git modules with a newer release available: name → latest version.
    available_updates: HashMap<String, String>,
    /// Modules that failed to start: name → error (shown in the module's tab).
    failed: HashMap<String, String>,
    /// Names of modules the user has granted their declared permissions
    /// (trusted at their current content digest).
    trusted: HashSet<String>,
    /// The consent dialog still on screen, which outlives `pending_action` by
    /// the length of its closing animation.
    consent_showing: Option<ui::Invoke>,
    /// A close was asked for and is waiting on an answer.
    quit_asking: bool,
    /// The answer was yes, so the next close request goes through.
    quit_confirmed: bool,
    /// The tab content area, as of the last frame that drew one. Pop-ups are
    /// scoped to it rather than to the window.
    content_rect: Option<egui::Rect>,
    /// What each module tab was showing when it was last active, keyed by module
    /// name. The active tab's own state lives in the fields above; this is where
    /// the others wait.
    module_pages: HashMap<String, ModulePage>,
    /// A module action waiting on the question its button carried, and the one
    /// still fading out — the same pair as the consent dialog, for the same
    /// reason: an answered question has to keep drawing while it leaves.
    pending_confirm: Option<ui::Invoke>,
    confirm_showing: Option<ui::Invoke>,
    /// An elevated action awaiting the user's consent (shown as a dialog).
    pending_action: Option<ui::Invoke>,
    /// A module the user asked to remove, held until they confirm. Removal
    /// deletes the module's directory — including anything it had fetched into
    /// `tools/` — so it is not something to do on a stray click.
    pending_remove: Option<String>,
    /// The name shown in the confirmation, kept while it animates closed.
    confirm_subject: Option<String>,

    /// Open detail tabs (from row actions), keyed by the id in `Tab::Detail`.
    detail_tabs: HashMap<u64, DetailTab>,
    /// Monotonic id for the next detail tab.
    next_detail_id: u64,

    /// Open tabs (in order) and the active index.
    tabs: Vec<Tab>,
    active: usize,

    view: Option<ui::View>,
    view_error: Option<String>,
    /// Module pop-ups, innermost last. A view that arrives with `modal` set is
    /// pushed here instead of replacing the screen behind it.
    modal_stack: Vec<ui::View>,
    /// The pop-up that was just closed, kept only until it has finished
    /// animating away — otherwise it would vanish instead of leaving.
    modal_closing: Option<ui::View>,
    inputs: HashMap<String, String>,
    output: String,
    busy: bool,
    /// The action currently in flight (its button shows a spinner).
    busy_action: Option<ui::Action>,

    // Modules page state
    search: String,
    filter: ModuleFilter,
    // Modules available in the GitHub org
    remote: Vec<RemoteModule>,
    remote_error: Option<String>,
    remote_loading: bool,
    remote_fetched: bool,
    /// The repo currently being installed, if any (disables Install + shows a spinner).
    installing: Option<String>,

    // Developer tab
    dev_tab: DevTab,
    logs: std::collections::VecDeque<String>,
    log_autoscroll: bool,

    /// Dev mode: source app/module updates from local dirs instead of GitHub.
    /// Session-only — never persisted, so it resets on restart.
    dev_mode_on: bool,
    dev_limen_path: String,
    dev_modules_path: String,

    /// Global UI scale as a percentage (persisted in settings).
    ui_scale: f32,

    /// Whether UI animations are enabled (persisted in settings).
    animations: bool,

    /// The active UI language (persisted in settings; mirrors `i18n`'s global).
    language: i18n::Lang,

    /// When the About tab was last shown — drives its staggered content reveal.
    about_revealed_at: Option<f64>,
    /// Reveal timers for the Settings/Developer/License tab entrance animations.
    settings_revealed_at: Option<f64>,
    developer_revealed_at: Option<f64>,
    license_revealed_at: Option<f64>,
    /// Which module the current view entrance was armed for, and when it began.
    /// Switching to a *different* module rearms it, so each module plays its
    /// staggered entrance rather than snapping into place.
    module_reveal: Option<(String, f64)>,
    /// Results from native file dialogs. The dialog call blocks until the user
    /// answers, so it runs on a thread of its own and reports back here — the UI
    /// thread only ever drains this channel.
    file_pick: (mpsc::Sender<FilePick>, mpsc::Receiver<FilePick>),
    /// Developer sub-tab the reveal was started for; a change replays it.
    shown_dev_tab: DevTab,
    /// When the Modules tab was last shown — drives the staggered list reveal.
    modules_revealed_at: Option<f64>,
    /// The filter the current reveal was started for; a change replays the reveal.
    shown_filter: ModuleFilter,
    /// Arrival time per available module — the cards stream in from GitHub in
    /// parallel (out of order), so each animates in from when *it* arrived.
    remote_arrivals: HashMap<String, f64>,

    /// Modules mid-removal: name → animation start time (or a negative sentinel
    /// once the actual removal has been sent). Drives the exit animation.
    removing: HashMap<String, f64>,

    /// An available app update (from the background check), if any.
    update: Option<limen_core::UpdateInfo>,
    /// True while an update download/install is in flight.
    updating: bool,
    /// A portable interpreter currently being installed (e.g. "Python"), if any.
    installing_runtime: Option<String>,
    /// Startup splash: the frame time it first rendered (set lazily on the first
    /// frame), and whether the ~2s animated intro has finished. Once done, the
    /// normal UI takes over.
    splash_start: Option<f64>,
    splash_done: bool,
    /// Whether the window has been centered + revealed yet (it's created hidden
    /// to avoid a dark startup flash).
    window_shown: bool,
    /// How many startup frames have pre-warmed the font atlas so far (warmed for
    /// the first several, after zoom/DPI settles, then stops).
    fonts_warm_frames: u32,
    /// Tab strip horizontal scroll state (single row; overflow → arrows + wheel).
    /// `scroll` is the current offset; `content_w`/`view_w` (from last frame) drive
    /// arrow visibility/enablement; `scroll_to` is a pending offset from an arrow.
    tab_scroll: f32,
    tab_content_w: f32,
    tab_view_w: f32,
    tab_scroll_to: Option<f32>,
}

impl LimenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Vec<std::path::PathBuf>) -> Self {
        ui::apply_theme(&cc.egui_ctx);
        let animations = limen_core::Config::load()
            .map(|c| c.animations)
            .unwrap_or(true);
        ui::set_animations(animations);
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
            language,
            about_revealed_at: None,
            settings_revealed_at: None,
            developer_revealed_at: None,
            license_revealed_at: None,
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
    fn active_tab(&self) -> Option<Tab> {
        self.tabs.get(self.active).cloned()
    }

    /// Lift the page currently in the fields out of them.
    fn take_page(&mut self) -> ModulePage {
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
    fn put_page(&mut self, p: ModulePage) {
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
    fn stash_page(&mut self) {
        let from = match self.active_tab() {
            Some(Tab::Module(name)) => Some(name),
            _ => None,
        };
        let cur = self.take_page();
        let next = swap_page(cur, &mut self.module_pages, from.as_deref(), None);
        self.put_page(next);
    }

    /// Take back what a module tab was showing, or start it empty.
    fn restore_page(&mut self, name: &str) {
        let cur = self.take_page();
        let to = (!name.is_empty()).then_some(name);
        let next = swap_page(cur, &mut self.module_pages, None, to);
        self.put_page(next);
    }

    /// The one door every tab change goes through, so a module tab's state is
    /// always put away before another takes the fields.
    fn activate(&mut self, index: usize) {
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
    fn open_tab(&mut self, tab: Tab) {
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
    fn close_tab(&mut self, index: usize) {
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
    fn resume_page(&mut self) {
        if let Some(a) = self.view.as_ref().and_then(|v| v.auto.clone()) {
            self.dispatch(a.into_invoke());
        }
    }

    /// Persist the current UI scale to settings.json (without clobbering others).
    fn save_ui_scale(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.ui_scale_percent = self.ui_scale.round() as u32;
            let _ = cfg.save();
        }
    }

    /// Persist the animations toggle to settings.json (without clobbering others).
    fn save_animations(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.animations = self.animations;
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
    fn save_language(&mut self) {
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
    fn drain_file_picks(&mut self) {
        while let Ok((id, path)) = self.file_pick.1.try_recv() {
            self.inputs.insert(id, path);
        }
    }

    /// Run a pending Browse request from a [`ui::Widget::File`].
    ///
    /// `rfd`'s dialog blocks until the user answers it, which would freeze the
    /// render loop — so it goes on its own thread and the result comes back
    /// through `file_pick`. A cancelled dialog simply sends nothing.
    fn serve_browse_request(&mut self, ctx: &egui::Context) {
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
    fn confirmed_removal(&mut self, ctx: &egui::Context) -> Option<String> {
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
    fn module_of(&self, capability: &str) -> Option<&ModuleSpec> {
        self.modules
            .iter()
            .find(|m| m.capabilities.iter().any(|c| c == capability))
    }

    /// Does this action invoke an elevated method on a module the user hasn't
    /// granted yet? If so, it needs a consent prompt first.
    fn action_needs_consent(&self, a: &ui::Action) -> bool {
        match self.module_of(&a.capability) {
            Some(m) => {
                m.permissions.method_needs_consent(&a.method) && !self.trusted.contains(&m.name)
            }
            None => false,
        }
    }

    /// Grant a module its declared permissions: pin trust to its current digest,
    /// persist, and update the cached trusted set. Returns success.
    fn grant_trust(&mut self, name: &str) -> bool {
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

    fn push_log(&mut self, line: String) {
        self.logs.push_back(line);
        while self.logs.len() > LOG_CAP {
            self.logs.pop_front();
        }
    }

    fn drain_events(&mut self, now: f64) {
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

    fn select_module(&mut self, name: String) {
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
    fn accept_view(&mut self, view: ui::View) {
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
    fn close_modals(&mut self) {
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
    fn forget_inputs(&mut self, view: &ui::View) {
        for id in ui::widget_ids(view) {
            self.inputs.remove(&id);
        }
    }

    /// Close the top pop-up, keeping it just long enough to animate away.
    fn dismiss_modal(&mut self) {
        // Only the last one out animates: closing one pop-up to reveal another
        // behind it is a change of contents, not the layer going away.
        if let Some(v) = self.modal_stack.pop() {
            self.forget_inputs(&v);
            if self.modal_stack.is_empty() {
                self.modal_closing = Some(v);
            }
        }
    }

    fn dispatch(&mut self, invoke: ui::Invoke) {
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

    fn first_capability(&self, name: &str) -> Option<String> {
        self.modules
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| m.capabilities.first().cloned())
    }
}

impl eframe::App for LimenApp {
    /// Clear transparent during the startup splash so only the floating icons
    /// show over the desktop; opaque warm-black once the app takes over.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.splash_done {
            let c = ui::color::BG;
            [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
                1.0,
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now_t = ctx.input(|i| i.time);
        self.drain_events(now_t);
        self.drain_file_picks();
        // Keep the screen→UI mapping current, so a file drag (during which the
        // window system stops reporting the pointer) can still be located.
        crate::cursor::calibrate(ctx);
        // A file drag produces exactly one event — `HoveredFile` — and then
        // silence: no cursor motion is reported for its duration. egui would go
        // idle on the very next frame, freezing the cursor sample taken as the
        // drag entered, so the drop zone would follow nothing. Keep asking for
        // frames while one is in flight, and only then.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            ctx.request_repaint();
        }

        // Apply the global UI scale (set_zoom_factor no-ops if unchanged).
        ctx.set_zoom_factor(self.ui_scale / 100.0);

        // The window is created hidden; on the first frame centre it on the
        // monitor and reveal it, so the splash appears cleanly mid-screen instead
        // of flashing a dark, unfocused window at the default position.
        if !self.window_shown {
            let size = egui::vec2(980.0, 640.0);
            if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = egui::pos2(
                    ((mon.x - size.x) * 0.5).max(0.0),
                    ((mon.y - size.y) * 0.5).max(0.0),
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.window_shown = true;
        }

        // Warm the font atlas over the first few frames while the window is still
        // a blank transparent canvas — *before* the splash starts. The glyph
        // rasterization hitches, but nothing is drawn yet (the window is
        // transparent, showing the desktop) and the splash clock hasn't started,
        // so it's invisible. Warming several frames lets the pixels-per-point /
        // zoom settle first, and returning early keeps the hitch off the splash
        // animation, which then plays smoothly against a settled atlas.
        const WARM_FRAMES: u32 = 6;
        if self.fonts_warm_frames < WARM_FRAMES {
            self.fonts_warm_frames += 1;
            prewarm_fonts(ctx);
            ctx.request_repaint();
            return;
        }

        // Startup splash in the centred window: the marks fade transparent→opaque,
        // hold opaque ~1s, then a 2s exit animation, after which the window
        // maximizes and the app takes over. Worker events keep draining above.
        // Skipped when animations are off.
        if !self.splash_done {
            let start = *self.splash_start.get_or_insert(now_t);
            let elapsed = (now_t - start) as f32;
            // With animations off, keep the transparent lead (so the clumsy opaque
            // startup frames stay hidden), then show the marks statically — no
            // fade/scanline motion.
            let dur = if self.animations {
                SPLASH_SECS
            } else {
                SPLASH_LEAD + SPLASH_HOLD
            };
            if elapsed < dur {
                ctx.request_repaint();
                splash_screen(ctx, elapsed, self.animations);
                return;
            }
            self.splash_done = true;
            // Grow into the app: fill the screen once the intro finishes.
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }

        // Reload is requested from the Modules page (set during the central panel).
        let mut reload = false;

        // Intents collected while rendering, applied after.
        let mut open_tab: Option<Tab> = None;
        let mut switch_to: Option<usize> = None;
        let mut close_idx: Option<usize> = None;
        let mut scale_changed = false;
        let mut anim_changed = false;
        let mut lang_changed = false;
        let mut dev_applied = false;

        // Custom-decoration resize grips along the window edges/corners.
        ui::window_resize_grips(ctx);

        // Title bar: brand + quick-open buttons + status.
        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::none()
                    .fill(ui::color::BG_ELEVATED)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        ui::with_alpha(ui::color::ACCENT, 0.18),
                    ))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                // Whole-bar drag to move the window + double-click to maximize.
                // Done before the row below so its buttons sit on top and keep
                // their own clicks. Constrain the drag zone to the row height —
                // `max_rect` here spans the whole window until content is measured,
                // so an unconstrained rect would hijack clicks on the pages below.
                let full = ui.max_rect();
                let bar_rect =
                    egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, full.min.y + 28.0));
                let bar = ui.interact(
                    bar_rect,
                    egui::Id::new("titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if bar.double_clicked() {
                    let max = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                if bar.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                    // App icon (the ◈ brand mark) in place of the wordmark.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    draw_brand(ui.painter(), rect, 1.0, false);
                    ui.add_space(12.0);
                    let active = self.active_tab();
                    if ui::chip(ui, &i18n::t("nav.about"), active == Some(Tab::About)).clicked() {
                        open_tab = Some(Tab::About);
                    }
                    if ui::chip(ui, &i18n::t("nav.modules"), active == Some(Tab::Modules)).clicked()
                    {
                        open_tab = Some(Tab::Modules);
                    }
                    // "Update available" pill, next to Modules.
                    if self.update.is_some() {
                        ui.add_space(6.0);
                        if ui::pill(ui, &i18n::t("nav.update_available"), ui::color::ORANGE)
                            .on_hover_text(i18n::t("nav.update_available_hint"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Update);
                        }
                    }
                    // Portable-interpreter install indicator (spinner + label),
                    // shown while a runtime like Python is being bundled.
                    if let Some(rt) = &self.installing_runtime {
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(format!("{} {rt}…", i18n::t("nav.installing")))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Window controls (rightmost). GNOME's default title bar
                        // shows only a close button, so on Linux that's all we
                        // draw (double-click still maximizes); Windows/macOS get
                        // the full close · maximize · minimize set.
                        if ui::window_button(ui, ui::WinBtn::Close)
                            .on_hover_text(i18n::t("win.close"))
                            .clicked()
                        {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if !cfg!(target_os = "linux") {
                            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                            let (mbtn, tip) = if maximized {
                                (ui::WinBtn::Restore, i18n::t("win.restore"))
                            } else {
                                (ui::WinBtn::Maximize, i18n::t("win.maximize"))
                            };
                            if ui::window_button(ui, mbtn).on_hover_text(tip).clicked() {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(
                                    !maximized,
                                ));
                            }
                            if ui::window_button(ui, ui::WinBtn::Minimize)
                                .on_hover_text(i18n::t("win.minimize"))
                                .clicked()
                            {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                        if ui::chip(ui, "🛠", active == Some(Tab::Developer))
                            .on_hover_text(i18n::t("nav.developer"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Developer);
                        }
                        if ui::chip(ui, "⚙", active == Some(Tab::Settings))
                            .on_hover_text(i18n::t("nav.settings"))
                            .clicked()
                        {
                            open_tab = Some(Tab::Settings);
                        }
                    });
                });
            });

        // Tab strip: open tabs, each with a close button. A tab *is* the
        // module's session — closing it ends the session.
        egui::TopBottomPanel::top("tabstrip")
            .frame(
                egui::Frame::none()
                    .fill(ui::color::BG)
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // A single-row, horizontally-scrollable tab strip. When the
                    // tabs overflow, nav arrows (« ‹ › ») appear and the mouse
                    // wheel scrolls it.
                    let arrow = |ui: &mut egui::Ui, glyph: &str, enabled: bool| -> bool {
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(20.0, 26.0), egui::Sense::click());
                        let col = if !enabled {
                            ui::with_alpha(ui::color::TEXT_MUTED, 0.3)
                        } else if resp.hovered() {
                            ui::color::ACCENT
                        } else {
                            ui::color::TEXT_MUTED
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            glyph,
                            egui::FontId::proportional(15.0),
                            col,
                        );
                        enabled && resp.clicked()
                    };
                    const STEP: f32 = 220.0;
                    let max_off = (self.tab_content_w - self.tab_view_w).max(0.0);
                    let overflow = max_off > 1.0;
                    let can_left = self.tab_scroll > 0.5;
                    let can_right = self.tab_scroll + 0.5 < max_off;
                    if overflow {
                        if arrow(ui, "«", can_left) {
                            self.tab_scroll_to = Some(0.0);
                        }
                        if arrow(ui, "‹", can_left) {
                            self.tab_scroll_to = Some((self.tab_scroll - STEP).max(0.0));
                        }
                    }
                    let right_reserve = if overflow { 40.0 } else { 0.0 };
                    let sa_width = (ui.available_width() - right_reserve).max(80.0);
                    let mut area = egui::ScrollArea::horizontal()
                        .id_source("tabstrip_scroll")
                        .auto_shrink([false, false])
                        .max_width(sa_width)
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                        );
                    if let Some(x) = self.tab_scroll_to.take() {
                        area = area.scroll_offset(egui::vec2(x, 0.0));
                    }
                    let out = area.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            let font_id = egui::TextStyle::Button.resolve(ui.style());
                            for (i, tab) in self.tabs.iter().enumerate() {
                                let selected = i == self.active;
                                let text = match tab {
                                    Tab::Detail { id } => self
                                        .detail_tabs
                                        .get(id)
                                        .map(|d| d.title.clone())
                                        .filter(|t| !t.is_empty())
                                        .unwrap_or_else(|| i18n::t("tab.details")),
                                    // Show the module's localized display title, not its
                                    // identifier.
                                    Tab::Module(name) => self
                                        .modules
                                        .iter()
                                        .find(|m| &m.name == name)
                                        .map(|m| localized_name(ui.ctx(), m))
                                        .unwrap_or_else(|| name.clone()),
                                    _ => tab.title(),
                                };

                                // Zed-style tab: stable width (the close slot is always
                                // reserved), the × only shows on hover or when active.
                                let pad = 10.0;
                                let close_w = 16.0;
                                let gap = 6.0;
                                let galley = ui.painter().layout_no_wrap(
                                    text,
                                    font_id.clone(),
                                    ui::color::TEXT,
                                );
                                let w = pad + galley.size().x + gap + close_w + pad;
                                let (rect, resp) = ui
                                    .allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
                                // Use the pointer position, not `resp.hovered()`: the close
                                // button below is drawn on top and would otherwise steal the
                                // hover, making the tab flicker as the × shows/hides.
                                let hovered = ui.rect_contains_pointer(rect);

                                // Smoothly fade the hover fill in, and grow the active
                                // underline out from the tab's centre toward its edges.
                                let hover_t = ui::anim_bool(
                                    ui,
                                    resp.id.with("hover"),
                                    hovered && !selected,
                                    0.14,
                                );
                                let active_t =
                                    ui::anim_bool(ui, resp.id.with("active"), selected, 0.05);

                                // Active tab adopts the panel colour; hover fades in.
                                let fill = if selected {
                                    ui::color::BG
                                } else {
                                    let e = ui::color::BG_ELEVATED;
                                    egui::Color32::from_rgba_unmultiplied(
                                        e.r(),
                                        e.g(),
                                        e.b(),
                                        (255.0 * hover_t) as u8,
                                    )
                                };
                                ui.painter().rect_filled(
                                    rect,
                                    egui::Rounding {
                                        nw: 5.0,
                                        ne: 5.0,
                                        sw: 0.0,
                                        se: 0.0,
                                    },
                                    fill,
                                );
                                if active_t > 0.0 {
                                    let half = rect.width() / 2.0 * active_t;
                                    let cx = rect.center().x;
                                    ui.painter().hline(
                                        (cx - half)..=(cx + half),
                                        rect.bottom() - 1.0,
                                        egui::Stroke::new(2.0_f32, ui::color::ACCENT),
                                    );
                                }

                                let text_t = if selected { 1.0 } else { hover_t };
                                let tcol =
                                    ui::lerp_color(ui::color::TEXT_MUTED, ui::color::TEXT, text_t);
                                let tpos = egui::pos2(
                                    rect.left() + pad,
                                    rect.center().y - galley.size().y / 2.0,
                                );
                                ui.painter().galley(tpos, galley, tcol);

                                // Close affordance — only when active or hovered.
                                let mut close_clicked = false;
                                if selected || hovered {
                                    let cc = egui::pos2(
                                        rect.right() - pad - close_w / 2.0,
                                        rect.center().y,
                                    );
                                    let crect = egui::Rect::from_center_size(
                                        cc,
                                        egui::vec2(close_w, close_w),
                                    );
                                    let cresp = ui.interact(
                                        crect,
                                        resp.id.with("close"),
                                        egui::Sense::click(),
                                    );
                                    if cresp.hovered() {
                                        ui.painter().rect_filled(
                                            crect,
                                            egui::Rounding::same(3.0),
                                            ui::color::BG_HOVER,
                                        );
                                    }
                                    ui.painter().text(
                                        cc,
                                        egui::Align2::CENTER_CENTER,
                                        "×",
                                        egui::FontId::proportional(15.0),
                                        if cresp.hovered() {
                                            ui::color::TEXT
                                        } else {
                                            ui::color::TEXT_MUTED
                                        },
                                    );
                                    if cresp.clicked() {
                                        close_idx = Some(i);
                                        close_clicked = true;
                                    }
                                }
                                if resp.clicked() && !close_clicked {
                                    switch_to = Some(i);
                                }
                            }
                        })
                    });
                    self.tab_scroll = out.state.offset.x;
                    self.tab_content_w = out.content_size.x;
                    self.tab_view_w = out.inner_rect.width();
                    if overflow {
                        if arrow(ui, "›", can_right) {
                            self.tab_scroll_to = Some((self.tab_scroll + STEP).min(max_off));
                        }
                        if arrow(ui, "»", can_right) {
                            self.tab_scroll_to = Some(max_off);
                        }
                    }
                });
            });

        // Central content for the active tab (split-borrow to mutate inputs etc).
        let mut action: Option<ui::Invoke> = None;
        let mut open_module: Option<String> = None;
        let mut remove_module: Option<String> = None;
        let mut add_module: Option<String> = None;
        let mut update_module: Option<String> = None;
        let mut do_update = false;
        let active_tab = self.active_tab();
        let update_info = self.update.clone();
        let updating = self.updating;
        // Arm the reveal on first show of the Modules tab; the filter-change replay
        // is handled inside `modules_page` so it lands on the same frame as the
        // click (avoiding a one-frame flash).
        if active_tab == Some(Tab::Modules) {
            self.modules_revealed_at.get_or_insert(now_t);
        } else {
            self.modules_revealed_at = None;
        }
        if active_tab == Some(Tab::About) {
            self.about_revealed_at.get_or_insert(now_t);
        } else {
            self.about_revealed_at = None;
        }
        if active_tab == Some(Tab::Settings) {
            self.settings_revealed_at.get_or_insert(now_t);
        } else {
            self.settings_revealed_at = None;
        }
        if active_tab == Some(Tab::Developer) {
            self.developer_revealed_at.get_or_insert(now_t);
        } else {
            self.developer_revealed_at = None;
        }
        if active_tab == Some(Tab::License) {
            self.license_revealed_at.get_or_insert(now_t);
        } else {
            self.license_revealed_at = None;
        }
        let module_reveal = match &active_tab {
            Some(Tab::Module(n)) => {
                if self.module_reveal.as_ref().map(|(m, _)| m.as_str()) != Some(n.as_str()) {
                    self.module_reveal = Some((n.clone(), now_t));
                }
                self.module_reveal.as_ref().map_or(now_t, |&(_, t)| t)
            }
            _ => {
                self.module_reveal = None;
                now_t
            }
        };
        let about_reveal = self.about_revealed_at.unwrap_or(now_t);
        let settings_reveal = self.settings_revealed_at.unwrap_or(now_t);
        let license_reveal = self.license_revealed_at.unwrap_or(now_t);
        {
            let LimenApp {
                modules,
                git_installed,
                git_meta,
                available_updates,
                view,
                view_error,
                inputs,
                output,
                busy_action,
                fatal,
                search,
                filter,
                remote,
                remote_error,
                remote_loading,
                installing,
                installing_runtime,
                dev_tab,
                logs,
                log_autoscroll,
                ui_scale,
                animations,
                language,
                dev_mode_on,
                dev_limen_path,
                dev_modules_path,
                removing,
                modules_revealed_at,
                shown_filter,
                remote_arrivals,
                developer_revealed_at,
                shown_dev_tab,
                detail_tabs,
                ..
            } = self;
            let content_margin = 16.0_f32;
            let content_frame = egui::Frame::none()
                .fill(ui::color::BG)
                .inner_margin(egui::Margin::same(content_margin));
            let content = egui::CentralPanel::default()
                .frame(content_frame)
                .show(ctx, |ui| {
                    // Framed HUD corner brackets, evenly inset from the window edge;
                    // the panel margin keeps page content padded inside them.
                    let outer = ui.max_rect().expand(content_margin);
                    ui::corner_brackets(ui.painter(), outer, 18.0, 8.0, 0.55);
                    if let Some(err) = fatal {
                        ui.colored_label(egui::Color32::LIGHT_RED, i18n::t("app.engine_failed"));
                        ui.add_space(4.0);
                        ui.monospace(err.as_str());
                        return;
                    }
                    match active_tab {
                        None => {
                            ui.add_space(24.0);
                            ui.vertical_centered(|ui| {
                                ui.label(egui::RichText::new(i18n::t("app.no_tabs")).weak());
                            });
                        }
                        Some(Tab::About) => {
                            if about_view(ui, about_reveal) {
                                open_tab = Some(Tab::License);
                            }
                        }
                        Some(Tab::License) => license_view(ui, license_reveal),
                        Some(Tab::Modules) => modules_page(
                            ui,
                            modules,
                            git_installed,
                            git_meta,
                            available_updates,
                            remote,
                            *remote_loading,
                            remote_error,
                            installing,
                            installing_runtime,
                            filter,
                            search,
                            &mut open_module,
                            &mut remove_module,
                            &mut add_module,
                            &mut update_module,
                            &mut reload,
                            modules_revealed_at,
                            shown_filter,
                            remote_arrivals,
                            removing,
                        ),
                        Some(Tab::Module(name)) => module_view(
                            ui,
                            &name,
                            module_reveal,
                            view,
                            view_error,
                            inputs,
                            output,
                            busy_action.as_ref(),
                            &mut action,
                        ),
                        Some(Tab::Detail { id }) => detail_view(ui, id, detail_tabs, &mut action),
                        Some(Tab::Settings) => settings_view(
                            ui,
                            ui_scale,
                            &mut scale_changed,
                            animations,
                            &mut anim_changed,
                            language,
                            &mut lang_changed,
                            settings_reveal,
                        ),
                        Some(Tab::Developer) => developer_view(
                            ui,
                            dev_tab,
                            inputs,
                            logs,
                            log_autoscroll,
                            dev_mode_on,
                            dev_limen_path,
                            dev_modules_path,
                            &mut dev_applied,
                            developer_revealed_at,
                            shown_dev_tab,
                        ),
                        Some(Tab::Update) => {
                            update_view(ui, update_info.as_ref(), updating, &mut do_update)
                        }
                    }
                });
            // Pop-ups belong to the tab that raised them, so this is what they
            // dim and block — the title bar and the tab strip stay live.
            self.content_rect = Some(content.response.rect);
        }

        if do_update && let Some(info) = self.update.clone() {
            self.updating = true;
            self.worker.send(Command::ApplyUpdate(info));
        }

        // Apply tab intents.
        if let Some(i) = switch_to {
            // Every tab change goes through one door, which puts the old tab's
            // state away and gives the new one its own back.
            self.activate(i);
            // A module tab that has never been shown still needs its first view.
            if let Some(Tab::Module(name)) = self.active_tab() {
                self.select_module(name);
            }
        }
        if let Some(i) = close_idx {
            self.close_tab(i);
        }
        if let Some(tab) = open_tab {
            match tab {
                Tab::Module(name) => self.select_module(name),
                other => self.open_tab(other),
            }
        }
        if scale_changed {
            self.save_ui_scale();
        }
        if anim_changed {
            ui::set_animations(self.animations);
            self.save_animations();
        }
        if lang_changed {
            self.save_language();
            // Re-play the current page's staggered entrance so it animates into
            // the new language (clearing the timers re-arms them on the next
            // frame; only the active page is non-None, so only it re-reveals).
            self.about_revealed_at = None;
            self.settings_revealed_at = None;
            self.developer_revealed_at = None;
            self.license_revealed_at = None;
            self.modules_revealed_at = None;
            // Installed cards re-resolve their description in-place (localized_desc
            // reads the module's locales/ folder, cached per language) — no engine
            // reload. The org list, though, resolved its descriptions over the
            // network at fetch time, so re-fetch it (async; cards stream back in).
            self.remote_fetched = false;
            self.remote.clear();
            self.remote_arrivals.clear();
        }
        if dev_applied {
            // Re-run both update checks now so the change is reflected without a
            // restart: the app check is otherwise startup-only, and Refresh
            // re-runs the module check against the (possibly new) source.
            self.worker.send(Command::CheckUpdate);
            self.worker.send(Command::Refresh);
        }

        if let Some(name) = open_module {
            self.select_module(name);
        }
        // Ask before removing. The confirmed name is applied further down, so
        // both paths run exactly the same removal.
        if let Some(name) = remove_module {
            self.pending_remove = Some(name);
        }
        if let Some(name) = self.confirmed_removal(ctx) {
            self.status = format!("removing {name}…");
            if self.animations {
                // Play the exit animation first; the actual removal fires when it
                // finishes (see the removal processing below).
                self.removing.insert(name, now_t);
            } else {
                self.busy = true;
                self.worker.send(Command::RemoveModule(name));
            }
        }
        // Drive in-flight removals: once a card's exit animation has run for its
        // duration, send the real removal (once); keep the entry so the card stays
        // invisible until the reload drops it, then clean it up.
        {
            let present: HashSet<&String> = self.modules.iter().map(|m| &m.name).collect();
            self.removing.retain(|name, _| present.contains(name));
            let mut fire: Vec<String> = Vec::new();
            for (name, start) in self.removing.iter_mut() {
                if *start >= 0.0 && now_t - *start >= 0.34 {
                    fire.push(name.clone());
                    *start = -1.0; // mark sent; card stays faded out
                }
            }
            for name in fire {
                self.busy = true;
                self.worker.send(Command::RemoveModule(name));
            }
            if !self.removing.is_empty() {
                ctx.request_repaint();
            }
        }
        if let Some(reference) = add_module {
            self.busy = true;
            self.installing = Some(reference.clone());
            self.status = format!("installing {reference}…");
            self.worker.send(Command::AddModule(reference));
        }
        if let Some(name) = update_module {
            // Close its tab first — its loaded UI is about to be replaced.
            if let Some(i) = self
                .tabs
                .iter()
                .position(|t| *t == Tab::Module(name.clone()))
            {
                self.close_tab(i);
            }
            self.busy = true;
            self.installing = Some(name.clone());
            self.status = format!("updating {name}…");
            self.worker.send(Command::UpdateModule(name));
        }
        if reload {
            // Re-scan the module directories from disk: pick up newly-added
            // modules and drop any whose folder is gone (not just re-list what's
            // already loaded).
            self.worker.send(Command::Reload);
            self.remote_fetched = false;
            // A reload restarts the modules, so a stored screen describes a
            // connection that no longer exists — and a stored pop-up would sit
            // over a module that has forgotten it was ever open.
            self.module_pages.clear();
        }
        if let Some(a) = action {
            // Elevated methods prompt for consent (once) before running.
            if self.action_needs_consent(&a.action) {
                self.pending_action = Some(a);
            } else {
                self.dispatch(a);
            }
        }
        // A Browse button was pressed somewhere in the view just drawn.
        self.serve_browse_request(ctx);

        // Consent dialog for a pending elevated action.
        //
        // Kept alive for a moment after it is answered so it can animate away —
        // the pending action is cleared immediately, so the *last* one is what
        // the closing frames draw.
        {
            let open = self.pending_action.is_some();
            if open {
                self.consent_showing = self.pending_action.clone();
            }
            let mut decision: Option<bool> = None;
            if let Some(pending) = self.consent_showing.clone() {
                let module = self.module_of(&pending.action.capability).cloned();
                let gone = ui::consent_dialog(ctx, open, self.content_rect, |ui| {
                    let fallback = i18n::t("perm.this_module");
                    let name = module
                        .as_ref()
                        .map(|m| m.name.as_str())
                        .unwrap_or(&fallback);
                    ui.label(
                        egui::RichText::new(
                            i18n::t("perm.wants_to_run")
                                .replace("{name}", name)
                                .replace("{method}", &pending.action.method),
                        )
                        .size(15.0),
                    );
                    if let Some(m) = &module {
                        let perms = m.permissions.summary();
                        if !perms.is_empty() {
                            ui.add_space(8.0);
                            for p in perms {
                                let admin = p.contains("administrator");
                                let col = if admin {
                                    egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)
                                } else {
                                    ui::color::TEXT_MUTED
                                };
                                ui.label(egui::RichText::new(format!("•  {p}")).color(col));
                            }
                        }
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui::primary_button(ui, &i18n::t("perm.grant_run"), egui::Vec2::ZERO)
                            .clicked()
                        {
                            decision = Some(true);
                        }
                        if ui::outline_button(ui, &i18n::t("perm.deny"), egui::Vec2::ZERO).clicked()
                        {
                            decision = Some(false);
                        }
                        ui.label(
                            egui::RichText::new(i18n::t("perm.remembered"))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    });
                });
                if gone {
                    self.consent_showing = None;
                }
                // Only a live dialog can be answered; a closing one is an
                // animation and its buttons are on their way out.
                if open {
                    match decision {
                        Some(true) => {
                            if let Some(m) = &module {
                                self.grant_trust(&m.name);
                            }
                            self.pending_action = None;
                            self.dispatch(pending); // now allowed
                        }
                        Some(false) => {
                            self.pending_action = None;
                            self.status = i18n::t("perm.denied");
                        }
                        None => {}
                    }
                }
            }
        }

        // Prefetch the org's module list once at startup (not lazily on first
        // Modules open) — the fetch is async on the worker, so it runs in the
        // background during/after the splash and the manager is already populated
        // by the time the user opens it. Also re-fires after a reload or a
        // language change (both reset `remote_fetched`).
        if !self.remote_fetched {
            self.remote_fetched = true;
            self.remote_loading = true;
            self.remote.clear();
            self.remote_arrivals.clear();
            self.remote_error = None;
            self.worker.send(Command::ListRemote);
        }

        // A module action's own confirmation, drawn with the host's dialog so it
        // matches the one the module manager uses to remove a module.
        {
            let open = self.pending_confirm.is_some();
            if open {
                self.confirm_showing = self.pending_confirm.clone();
            }
            if let Some(inv) = self.confirm_showing.clone() {
                let c = inv.confirm.clone().unwrap_or_default();
                let subject = (!c.subject.is_empty()).then_some(c.subject.as_str());
                let yes = if c.confirm_label.is_empty() {
                    i18n::t("confirm.remove_yes")
                } else {
                    c.confirm_label.clone()
                };
                let no = if c.cancel_label.is_empty() {
                    i18n::t("confirm.cancel")
                } else {
                    c.cancel_label.clone()
                };
                match ui::confirm_dialog(
                    ctx,
                    "module",
                    open,
                    &c.title,
                    subject,
                    &yes,
                    &no,
                    self.content_rect,
                ) {
                    Some(true) => {
                        self.pending_confirm = None;
                        // Run it now that it has been answered; the question is
                        // dropped so it is not asked a second time.
                        let mut go = inv;
                        go.confirm = None;
                        self.dispatch(go);
                    }
                    Some(false) => self.pending_confirm = None,
                    None => {
                        if !open {
                            self.confirm_showing = None;
                        }
                    }
                }
            }
        }

        // Module pop-ups, over everything the panels drew. Driven every frame
        // rather than only while one is open, so a closing pop-up has something
        // to animate away with.
        {
            let busy_action = self.busy_action.clone();
            let depth = self.modal_stack.len();
            let top = self
                .modal_stack
                .last()
                .or(self.modal_closing.as_ref())
                .cloned();
            let out = ui::modal_layer(
                ctx,
                top.as_ref(),
                !self.modal_stack.is_empty(),
                depth,
                self.content_rect,
                &mut self.inputs,
                busy_action.as_ref(),
            );
            if out.closed {
                self.modal_closing = None;
            }
            if out.close_all {
                self.close_modals();
            } else if out.dismissed {
                self.dismiss_modal();
            } else if let Some(inv) = out.invoke {
                // A `dismiss` button is answered here — it has nothing to ask the
                // module, so a round trip would only add a flicker.
                if inv.dismiss {
                    self.dismiss_modal();
                } else {
                    self.dispatch(inv);
                }
            }
        }

        // Closing asks first. Deliberately the *whole* window rather than a
        // tab's content: quitting is not one tab's business, and everything
        // behind the question should be inert until it is answered.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_asking = true;
        }
        if self.quit_asking {
            match ui::confirm_dialog(
                ctx,
                "quit",
                true,
                &i18n::t("confirm.quit_title"),
                None,
                &i18n::t("confirm.quit_yes"),
                &i18n::t("confirm.cancel"),
                // No bounds: this one covers the app, not a tab.
                None,
            ) {
                Some(true) => {
                    self.quit_asking = false;
                    self.quit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(false) => self.quit_asking = false,
                None => {}
            }
        } else {
            // Keep driving it so it animates away after an answer.
            let _ = ui::confirm_dialog(
                ctx,
                "quit",
                false,
                &i18n::t("confirm.quit_title"),
                None,
                &i18n::t("confirm.quit_yes"),
                &i18n::t("confirm.cancel"),
                None,
            );
        }

        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

// --------------------------------------------------------------------------- //

/// The Update tab: current/latest versions + an Update button.
fn update_view(
    ui: &mut egui::Ui,
    info: Option<&limen_core::UpdateInfo>,
    updating: bool,
    do_update: &mut bool,
) {
    ui.add_space(4.0);
    ui.heading(i18n::t("update.title"));
    ui.separator();
    ui.add_space(8.0);

    let Some(info) = info else {
        ui.label(egui::RichText::new(i18n::t("update.up_to_date")).color(ui::color::TEXT_MUTED));
        return;
    };

    ui.label(
        egui::RichText::new(format!(
            "{}: v{} -> v{}",
            i18n::t("update.available"),
            info.current,
            info.latest
        ))
        .size(15.0),
    );
    if !info.url.is_empty() {
        ui.add_space(2.0);
        ui.hyperlink_to(i18n::t("update.release_notes"), &info.url);
    }
    if !info.notes.trim().is_empty() {
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                ui::markdown(ui, info.notes.trim());
            });
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let clicked = ui
            .add_enabled_ui(!updating, |ui| {
                ui::primary_button(ui, &i18n::t("update.button"), egui::Vec2::ZERO)
            })
            .inner
            .clicked();
        if clicked {
            *do_update = true;
        }
        if updating {
            ui.add_space(6.0);
            ui.spinner();
            ui.label(egui::RichText::new(i18n::t("update.installing")).small());
        } else if info.asset_url.is_none() {
            ui.label(
                egui::RichText::new(i18n::t("update.no_binary"))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
    });
}

/// The Settings tab. `reveal_at` staggers the sections in when the tab is shown.
#[allow(clippy::too_many_arguments)]
fn settings_view(
    ui: &mut egui::Ui,
    scale: &mut f32,
    changed: &mut bool,
    animations: &mut bool,
    anim_changed: &mut bool,
    lang: &mut i18n::Lang,
    lang_changed: &mut bool,
    reveal_at: f64,
) {
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();
    let hint = |ui: &mut egui::Ui, key: &str| {
        ui.label(
            egui::RichText::new(i18n::t(key))
                .small()
                .color(ui::color::TEXT_MUTED),
        );
    };

    ui.add_space(4.0);
    reveal_item(ui, 0, reveal_at, now, animate, |ui| {
        ui.heading(i18n::t("settings.title"));
        ui.separator();
    });

    // Language — the endonyms stay untranslated (a picker always shows each
    // language in its own name).
    ui.add_space(6.0);
    reveal_item(ui, 1, reveal_at, now, animate, |ui| {
        ui.label(egui::RichText::new(i18n::t("settings.language")).strong());
        hint(ui, "settings.language_hint");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for l in i18n::Lang::ALL {
                if ui::chip(ui, l.label(), *lang == l).clicked() {
                    *lang = l;
                    *lang_changed = true;
                }
            }
        });
    });

    ui.add_space(16.0);
    reveal_item(ui, 2, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("settings.ui_scale")).strong());
        hint(ui, "settings.ui_scale_hint");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for pct in [100.0_f32, 125.0, 150.0, 175.0, 200.0] {
                let selected = (*scale - pct).abs() < 0.5;
                if ui::chip(ui, &format!("{}%", pct as u32), selected).clicked() {
                    *scale = pct;
                    *changed = true;
                }
            }
        });
    });

    ui.add_space(16.0);
    reveal_item(ui, 3, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(i18n::t("settings.animations")).strong());
        hint(ui, "settings.animations_hint");
        ui.add_space(6.0);
        if ui::toggle(ui, animations, &i18n::t("settings.animations_toggle")).changed() {
            *anim_changed = true;
        }
    });
}

/// The Developer tab's "Dev mode" sub-tab: source app/module updates from local
/// directories instead of GitHub, for testing. Session-only (resets on restart).
/// `applied` is set when the source changes so the caller can re-run the checks.
fn dev_mode_view(
    ui: &mut egui::Ui,
    dev_mode_on: &mut bool,
    dev_limen_path: &mut String,
    dev_modules_path: &mut String,
    applied: &mut bool,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(i18n::t("dev.dev_mode")).strong());
    ui.label(
        egui::RichText::new(i18n::t("dev.dev_mode_help"))
            .small()
            .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(8.0);
    egui::Grid::new("dev_mode_paths")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label(i18n::t("dev.limen_path"));
            ui::text_field(
                ui,
                dev_limen_path,
                &i18n::t("dev.limen_path_hint"),
                320.0,
                false,
            );
            ui.end_row();
            ui.label(i18n::t("dev.modules_path"));
            ui::text_field(
                ui,
                dev_modules_path,
                &i18n::t("dev.modules_path_hint"),
                320.0,
                false,
            );
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if *dev_mode_on {
            i18n::t("dev.update_dev_mode")
        } else {
            i18n::t("dev.set_dev_mode")
        };
        if ui::primary_button(ui, &label, egui::Vec2::ZERO).clicked() {
            let as_dir = |s: &str| {
                let t = s.trim();
                (!t.is_empty()).then(|| std::path::PathBuf::from(t))
            };
            limen_core::set_update_dir(as_dir(dev_limen_path));
            limen_registry::set_update_modules_dir(as_dir(dev_modules_path));
            *dev_mode_on = true;
            *applied = true;
        }
        if *dev_mode_on
            && ui::outline_button(ui, &i18n::t("dev.turn_off"), egui::Vec2::ZERO).clicked()
        {
            limen_core::set_update_dir(None);
            limen_registry::set_update_modules_dir(None);
            *dev_mode_on = false;
            *applied = true;
        }
    });

    // GitHub token (persistent) — authenticates module-registry requests, lifting
    // the 60/hr anonymous limit to 5,000/hr and making conditional (304) checks
    // free. Self-contained: the edit buffer lives in egui memory; Save persists it
    // to settings.json and applies it live.
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(i18n::t("dev.token_title")).strong());
    ui.label(
        egui::RichText::new(i18n::t("dev.token_help"))
            .small()
            .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(4.0);
    // The validation call hits the network, so it runs on a background thread; the
    // shared slot below carries its result back (`Ok(token)` = verified, save it;
    // `Err(msg)` = rejected). While the slot is present but empty, a spinner shows.
    type TokenProbe = std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>;
    let tok_id = egui::Id::new("dev_github_token_buf");
    let status_id = egui::Id::new("dev_github_token_status");
    let probe_id = egui::Id::new("dev_github_token_probe");
    let mut token = ui.data_mut(|d| {
        d.get_temp::<String>(tok_id).unwrap_or_else(|| {
            limen_core::Config::load()
                .ok()
                .and_then(|c| c.github_token)
                .unwrap_or_default()
        })
    });
    // (message, is_error) — result of the last Save, kept in egui memory.
    let mut status = ui.data_mut(|d| d.get_temp::<(String, bool)>(status_id).unwrap_or_default());

    // Collect the result of an in-flight validation, if any has finished.
    let probe = ui.data_mut(|d| d.get_temp::<TokenProbe>(probe_id));
    let mut testing = false;
    if let Some(p) = probe {
        match p.lock().unwrap().take() {
            Some(Ok(t)) => {
                if let Ok(mut cfg) = limen_core::Config::load() {
                    cfg.github_token = Some(t.clone());
                    let _ = cfg.save();
                }
                limen_registry::set_github_token(Some(t));
                status = (i18n::t("dev.token_saved"), false);
                *applied = true;
                ui.data_mut(|d| d.remove::<TokenProbe>(probe_id));
            }
            Some(Err(e)) => {
                status = (format!("{} {e}", i18n::t("dev.token_not_saved")), true);
                ui.data_mut(|d| d.remove::<TokenProbe>(probe_id));
            }
            None => {
                // Still running — keep the frame animating.
                testing = true;
                ui.ctx().request_repaint();
            }
        }
    }

    ui.horizontal(|ui| {
        ui::text_field(ui, &mut token, &i18n::t("dev.token_hint"), 320.0, true);
        ui.add_enabled_ui(!testing, |ui| {
            if ui::primary_button(ui, &i18n::t("dev.save_token"), egui::Vec2::ZERO).clicked() {
                let t = token.trim().to_string();
                if t.is_empty() {
                    // Clearing needs no test — revert to the unauthenticated default.
                    if let Ok(mut cfg) = limen_core::Config::load() {
                        cfg.github_token = None;
                        let _ = cfg.save();
                    }
                    limen_registry::set_github_token(None);
                    status = (i18n::t("dev.token_cleared"), false);
                    *applied = true;
                } else {
                    // Verify the token grants API access, off-thread, before saving.
                    let slot: TokenProbe = std::sync::Arc::new(std::sync::Mutex::new(None));
                    let sink = slot.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let res = limen_registry::test_github_token(&t).map(|()| t);
                        *sink.lock().unwrap() = Some(res);
                        ctx.request_repaint();
                    });
                    ui.data_mut(|d| d.insert_temp(probe_id, slot));
                    status = (String::new(), false);
                    ui.ctx().request_repaint();
                }
            }
        });
        if testing {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(
                egui::RichText::new(i18n::t("dev.verifying"))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
    });
    if !status.0.is_empty() {
        ui.add_space(2.0);
        let color = if status.1 {
            ui::color::ERROR
        } else {
            ui::color::SUCCESS
        };
        ui.label(egui::RichText::new(&status.0).small().color(color));
    }
    ui.data_mut(|d| {
        d.insert_temp(tok_id, token);
        d.insert_temp(status_id, status);
    });

    if *dev_mode_on {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(i18n::t("dev.dev_mode_on"))
                .small()
                .color(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)),
        );
    }
}

/// The Developer tab: sub-tabs for docs / UI kit / log console. The content fades
/// in when the tab is shown *and* whenever the sub-tab changes (`reveal_at` /
/// `shown_dev_tab` track that, reset on the same frame as the click).
#[allow(clippy::too_many_arguments)]
fn developer_view(
    ui: &mut egui::Ui,
    dev_tab: &mut DevTab,
    inputs: &mut HashMap<String, String>,
    logs: &std::collections::VecDeque<String>,
    autoscroll: &mut bool,
    dev_mode_on: &mut bool,
    dev_limen_path: &mut String,
    dev_modules_path: &mut String,
    dev_applied: &mut bool,
    revealed_at: &mut Option<f64>,
    shown_dev_tab: &mut DevTab,
) {
    ui.horizontal(|ui| {
        if ui::chip(
            ui,
            &i18n::t("dev.tab_dev_mode"),
            *dev_tab == DevTab::DevMode,
        )
        .clicked()
        {
            *dev_tab = DevTab::DevMode;
        }
        if ui::chip(ui, &i18n::t("dev.tab_ui_kit"), *dev_tab == DevTab::UiKit).clicked() {
            *dev_tab = DevTab::UiKit;
        }
        if ui::chip(ui, &i18n::t("dev.tab_console"), *dev_tab == DevTab::Console).clicked() {
            *dev_tab = DevTab::Console;
        }
    });
    ui.separator();

    // Replay the content reveal when the sub-tab changes (same frame as the click).
    let now = ui.input(|i| i.time);
    if *dev_tab != *shown_dev_tab {
        *revealed_at = Some(now);
        *shown_dev_tab = *dev_tab;
    }
    let reveal_at = revealed_at.unwrap_or(now);
    let animate = ui::animations_enabled();

    reveal_item(ui, 0, reveal_at, now, animate, |ui| match dev_tab {
        DevTab::DevMode => dev_mode_view(
            ui,
            dev_mode_on,
            dev_limen_path,
            dev_modules_path,
            dev_applied,
        ),
        DevTab::UiKit => ui::render_demo_ui(ui, inputs),
        DevTab::Console => dev_console(ui, logs, autoscroll),
    });
}

/// The Developer window's "Console" tab — all host + module log lines.
fn dev_console(
    ui: &mut egui::Ui,
    logs: &std::collections::VecDeque<String>,
    autoscroll: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {}", logs.len(), i18n::t("dev.lines")))
                .color(ui::color::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui::toggle(ui, autoscroll, &i18n::t("dev.autoscroll"));
        });
    });
    ui.separator();

    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(*autoscroll)
                .show(ui, |ui| {
                    if logs.is_empty() {
                        ui.label(
                            egui::RichText::new(i18n::t("dev.no_logs"))
                                .color(ui::color::TEXT_MUTED),
                        );
                    }
                    for line in logs {
                        ui.label(egui::RichText::new(line).monospace().size(12.0));
                    }
                });
        });
}

/// Rasterize the common glyphs (Latin + Cyrillic + digits + punctuation) into
/// egui's font atlas by laying them out once. egui rasterizes glyphs lazily on
/// first layout/paint, so the first content-heavy page (the Modules manager,
/// with many names/descriptions — worse with the variable display font) would
/// otherwise rasterize a big batch in one frame and hitch the render thread.
/// Doing it on the first frame, behind the splash, pays that cost up-front.
fn prewarm_fonts(ctx: &egui::Context) {
    // Every glyph the host UI uses — the full Latin + Ukrainian-Cyrillic
    // alphabets, digits, punctuation, and the specific symbols/emoji (`• … ▸ ↗
    // × — · “ ” ⚙ 🛠`). Any glyph/size missing here would first appear on a
    // tab, forcing egui to rebuild + re-upload the whole atlas that frame — the
    // hitch that ate the entrance animation.
    const GLYPHS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 \
        .,:;!?'\"“”-—·•…()[]{}<>%/@#&№×↗▸ ⚙🛠 \
        АБВГҐДЕЄЖЗИІЇЙКЛМНОПРСТУФХЦЧШЩЬЮЯабвгґдеєжзиіїйклмнопрстуфхцчшщьюя";
    // Cover every size the UI renders text at, for both families.
    const SIZES: [f32; 11] = [
        10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 24.0, 30.0,
    ];
    ctx.fonts(|f| {
        for &size in &SIZES {
            let _ = f.layout_no_wrap(
                GLYPHS.to_owned(),
                egui::FontId::proportional(size),
                egui::Color32::WHITE,
            );
            let _ = f.layout_no_wrap(
                GLYPHS.to_owned(),
                egui::FontId::monospace(size),
                egui::Color32::WHITE,
            );
        }
    });
}

/// An installed module's card description, translated for the active UI language
/// from its `locales/<lang>.toml`, else the manifest default. The file read is
/// cached in egui memory per (module, language), so language switches update the
/// cards with no engine reload and no per-frame disk I/O.
fn localized_desc(ui: &egui::Ui, m: &ModuleSpec) -> Option<String> {
    let lang = i18n::locale();
    let id = egui::Id::new(("moddesc", m.name.as_str(), lang.code()));
    let resolved = match ui.data(|d| d.get_temp::<Option<String>>(id)) {
        Some(v) => v,
        None => {
            let r = limen_proto::manifest::localized_description(&m.cwd, lang.code());
            ui.data_mut(|d| d.insert_temp(id, r.clone()));
            r
        }
    };
    resolved.or_else(|| m.description.clone())
}

/// An installed module's display title, translated for the active UI language
/// from its `locales/<lang>.toml` `[module] title`, else the manifest
/// `display_name`, else the module's identifier `name`. Cached like `localized_desc`.
fn localized_name(ctx: &egui::Context, m: &ModuleSpec) -> String {
    let lang = i18n::locale();
    let id = egui::Id::new(("modtitle", m.name.as_str(), lang.code()));
    let resolved = match ctx.data(|d| d.get_temp::<Option<String>>(id)) {
        Some(v) => v,
        None => {
            let r = limen_proto::manifest::localized_title(&m.cwd, lang.code());
            ctx.data_mut(|d| d.insert_temp(id, r.clone()));
            r
        }
    };
    resolved
        .or_else(|| m.display_name.clone())
        .unwrap_or_else(|| m.name.clone())
}

/// The Modules page — a Zed-Extensions-style list: installed modules plus the
/// ones available in the GitHub org (installable in a click).
#[allow(clippy::too_many_arguments)]
fn modules_page(
    ui: &mut egui::Ui,
    modules: &[ModuleSpec],
    git_installed: &HashSet<String>,
    git_meta: &HashMap<String, (String, String)>,
    available_updates: &HashMap<String, String>,
    remote: &[RemoteModule],
    remote_loading: bool,
    remote_error: &Option<String>,
    installing: &Option<String>,
    installing_runtime: &Option<String>,
    filter: &mut ModuleFilter,
    search: &mut String,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    add: &mut Option<String>,
    update: &mut Option<String>,
    reload: &mut bool,
    modules_revealed_at: &mut Option<f64>,
    shown_filter: &mut ModuleFilter,
    remote_arrivals: &HashMap<String, f64>,
    removing: &HashMap<String, f64>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading(i18n::t("modules.title"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui::outline_button(ui, &i18n::t("modules.reload"), egui::Vec2::ZERO).clicked() {
                *reload = true;
            }
        });
    });
    ui.add_space(10.0);

    // One universal search across installed (local) and org (remote) modules.
    ui::text_field(
        ui,
        search,
        &i18n::t("modules.search_hint"),
        f32::INFINITY,
        false,
    );
    ui.add_space(8.0);

    // Installed / Available filter.
    ui.horizontal(|ui| {
        for (value, key) in [
            (ModuleFilter::All, "modules.filter.all"),
            (ModuleFilter::Installed, "modules.filter.installed"),
            (ModuleFilter::Available, "modules.filter.available"),
        ] {
            if ui::chip(ui, &i18n::t(key), *filter == value).clicked() {
                *filter = value;
            }
        }
        if remote_loading {
            ui.add_space(8.0);
            ui.spinner();
        }
    });

    // The chips above may have just flipped the filter — reset the reveal timer
    // *this* frame so the new list animates in from opacity 0 rather than flashing
    // at full opacity for one frame before restarting.
    let now = ui.input(|i| i.time);
    if *filter != *shown_filter {
        *modules_revealed_at = Some(now);
        *shown_filter = *filter;
    }
    let reveal_at = modules_revealed_at.unwrap_or(now);

    ui.add_space(6.0);
    ui.separator();

    let terms = parse_query(search);
    let installed_names: HashSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    // A tag or dependency clicked on any card; applied to the search box once the
    // list is done being drawn, since `search` is what the loop above filters on.
    let mut tag_click: Option<String> = None;
    // Which installed module provides each capability, so a dependency can be
    // shown as the module behind it rather than the raw capability string.
    let providers: HashMap<&str, &ModuleSpec> = modules
        .iter()
        .flat_map(|m| m.capabilities.iter().map(move |c| (c.as_str(), m)))
        .collect();

    // `auto_shrink` off is what makes the wheel work here, not just a layout
    // tweak: a shrunk scroll area is only as wide as its widest card, and the
    // wheel is ignored anywhere outside it — so scrolling did nothing over the
    // empty space beside the list. Filling the panel makes the whole page take
    // the wheel.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let mut shown = 0;
        let animate = ui::animations_enabled();

        for m in modules.iter() {
            if *filter == ModuleFilter::Available || !module_matches(m, &terms) {
                continue;
            }
            let rt = match removing.get(m.name.as_str()) {
                Some(&s) if s < 0.0 => 1.0, // removal sent — stay faded out
                Some(&s) => (((now - s) / 0.34).clamp(0.0, 1.0)) as f32,
                None => 0.0,
            };
            let id = egui::Id::new(("modcard", m.name.as_str()));
            reveal_card(ui, id, shown, reveal_at, now, animate, rt, |ui| {
                module_card(
                    ui,
                    m,
                    git_installed.contains(&m.name),
                    git_meta.get(&m.name),
                    available_updates.get(&m.name).map(String::as_str),
                    installing,
                    installing_runtime,
                    open,
                    remove,
                    update,
                    &mut tag_click,
                    &providers,
                );
            });
            shown += 1;
        }

        // Available in the org (not already installed). These stream in from
        // GitHub in parallel, so a card has two possible entrances and takes
        // whichever is newer: it either lands *after* the page revealed, and
        // pops in alone the moment it arrives, or it was already listed and
        // joins the page's cascade at its position. Keying only off arrival —
        // as this did — meant a card fetched minutes ago had a long-past start
        // time and snapped in without animating whenever the tab was reopened.
        for r in remote {
            if *filter == ModuleFilter::Installed
                || installed_names.contains(r.name.as_str())
                || !remote_matches(r, &terms)
            {
                continue;
            }
            let id = egui::Id::new(("availcard", r.name.as_str()));
            let (start, k) = match remote_arrivals.get(&r.name) {
                Some(&a) if a > reveal_at => (a, 0),
                _ => (reveal_at, shown),
            };
            reveal_card(ui, id, k, start, now, animate, 0.0, |ui| {
                available_card(ui, r, installing, add, &mut tag_click);
            });
            shown += 1;
        }

        if let Some(err) = remote_error {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("{} {err}", i18n::t("modules.org_error")))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
        if shown == 0 && !remote_loading {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(i18n::t("modules.none_match")).color(ui::color::TEXT_MUTED),
                );
            });
        }
    });

    // Clicking a tag filters by it: drop it into the search box, which every
    // card already matches against.
    if let Some(t) = tag_click {
        *search = t;
    }
}

/// Which part of a module a search term looks at.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Field {
    /// The default: identifier and display name only, so a bare word stays
    /// precise instead of matching half the list through its description.
    Name,
    Tag,
    Description,
    Capability,
}

/// One term of a search query: `programs` (name), or `tag:ua` / `description:banned`
/// / `cap:programs.local` for a specific field.
#[derive(Clone, PartialEq, Debug)]
struct Term {
    field: Field,
    text: String,
}

/// Split a query into terms, resolving `field:value` prefixes.
///
/// Terms are ANDed, so `tag:ua programs` means "tagged ua *and* named programs".
/// An unrecognised prefix is not a field — `foo:bar` is searched literally as a
/// name, so a colon in a name never silently matches nothing.
fn parse_query(query: &str) -> Vec<Term> {
    query
        .split_whitespace()
        .filter_map(|tok| {
            let (field, text) = match tok.split_once(':') {
                Some((k, v)) => match k.to_lowercase().as_str() {
                    "name" => (Field::Name, v),
                    "tag" | "tags" => (Field::Tag, v),
                    "desc" | "description" => (Field::Description, v),
                    "cap" | "capability" | "capabilities" => (Field::Capability, v),
                    _ => (Field::Name, tok),
                },
                None => (Field::Name, tok),
            };
            let text = text.trim().to_lowercase();
            (!text.is_empty()).then_some(Term { field, text })
        })
        .collect()
}

/// Does any of `haystack` contain `needle`?
fn any_contains<'a>(haystack: impl IntoIterator<Item = &'a str>, needle: &str) -> bool {
    haystack
        .into_iter()
        .any(|h| h.to_lowercase().contains(needle))
}

fn module_matches(m: &ModuleSpec, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t.field {
        Field::Name => any_contains(
            [m.name.as_str(), m.display_name.as_deref().unwrap_or("")],
            &t.text,
        ),
        Field::Tag => any_contains(m.tags.iter().map(String::as_str), &t.text),
        Field::Description => any_contains([m.description.as_deref().unwrap_or("")], &t.text),
        Field::Capability => any_contains(m.capabilities.iter().map(String::as_str), &t.text),
    })
}

fn remote_matches(r: &RemoteModule, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t.field {
        Field::Name => any_contains([r.name.as_str(), r.title.as_deref().unwrap_or("")], &t.text),
        Field::Tag => any_contains(r.tags.iter().map(String::as_str), &t.text),
        Field::Description => any_contains([r.description.as_deref().unwrap_or("")], &t.text),
        Field::Capability => any_contains(r.capabilities.iter().map(String::as_str), &t.text),
    })
}

/// A single installed-module card, in its own rounded box. `from_git` shows the
/// GitHub action only for modules installed from a repo (manual ones get just
/// Open + Remove).
/// Wrap a list card in its animations: a staggered slide-in from the left on first
/// reveal (delayed by `k`), multiplied by `filter_t` — the card's fade as it
/// enters/leaves the list on a filter change. `filter_t` is 1 while shown, eases
/// to 0 as it leaves; persistent cards sit at 1 and don't re-animate.
#[allow(clippy::too_many_arguments)]
fn reveal_card(
    ui: &mut egui::Ui,
    id: egui::Id,
    k: usize,
    reveal_at: f64,
    now: f64,
    animate: bool,
    remove_t: f32,
    draw: impl FnOnce(&mut egui::Ui),
) {
    // Trailing gap between cards (also collapses during a removal).
    const GAP: f32 = 10.0;
    if !animate {
        draw(ui);
        ui.add_space(GAP);
        return;
    }
    // Staggered entrance (replays when the reveal timer resets: tab shown or
    // filter changed) and a smoothstep exit so the fade, slide, and — most
    // importantly — the height collapse all progress steadily.
    let enter = ui::reveal_t(ui, k, reveal_at, now, 0.03, 0.18);
    let exit = ui::smoothstep(remove_t);

    let dx = (1.0 - enter) * 28.0 + exit * 28.0;
    if remove_t > 0.0 && remove_t < 1.0 {
        ui.ctx().request_repaint(); // keep the exit animating until settled
    }

    if exit <= 0.0 {
        // Normal render — and remember the height so a later removal can collapse
        // it smoothly (measured on a non-collapsing frame).
        let inner = ui.scope(|ui| {
            ui.set_opacity(enter);
            ui.horizontal(|ui| {
                ui.add_space(dx);
                draw(ui);
            });
        });
        ui.data_mut(|d| d.insert_temp(id, inner.response.rect.height()));
        ui.add_space(GAP);
        return;
    }

    // Removing: shrink the vertical space (card height + gap) so the cards below
    // slide up to fill it, while the card fades and slides out to the right.
    let full_h: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(90.0);
    let width = ui.available_width();
    let h = full_h * (1.0 - exit);
    let (outer, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let mut child = ui.child_ui_with_id_source(
        egui::Rect::from_min_size(outer.min, egui::vec2(width, full_h)),
        egui::Layout::top_down(egui::Align::Min),
        id,
        None,
    );
    child.set_clip_rect(child.clip_rect().intersect(outer));
    child.set_opacity(enter * (1.0 - exit));
    child.horizontal(|ui| {
        ui.add_space(dx);
        draw(ui);
    });
    ui.add_space(GAP * (1.0 - exit));
}

#[allow(clippy::too_many_arguments)]
fn module_card(
    ui: &mut egui::Ui,
    m: &ModuleSpec,
    from_git: bool,
    git_meta: Option<&(String, String)>,
    latest: Option<&str>,
    installing: &Option<String>,
    installing_runtime: &Option<String>,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    update: &mut Option<String>,
    tag_click: &mut Option<String>,
    providers: &HashMap<&str, &ModuleSpec>,
) {
    // This card is mid-update; another install/update is running somewhere.
    let this_busy = installing.as_deref() == Some(m.name.as_str());
    let any_busy = installing.is_some();
    // The scripted runtime this module needs is still downloading — every module
    // sharing that runtime (e.g. all Python modules) is not launchable yet, so
    // its Open button is disabled until the interpreter is bundled.
    let runtime_busy = Runtime::for_language(m.language)
        .is_some_and(|rt| installing_runtime.as_deref() == Some(rt.display()));
    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, ui::color::BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            // Stretch the box to the full available width.
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let right_w = 112.0;
                let spacing = ui.spacing().item_spacing.x;
                let left_w = (ui.available_width() - right_w - spacing).max(200.0);

                // Left column: name, badges, description, authors.
                ui.allocate_ui_with_layout(
                    egui::vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        // Force the region to fill left_w so the right column is
                        // pushed to the box's end (allocate_ui otherwise collapses
                        // to content width).
                        ui.set_min_width(left_w);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(localized_name(ui.ctx(), m))
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!("v{}", m.version))
                                    .monospace()
                                    .color(ui::color::TEXT_MUTED),
                            );
                            for cap in &m.capabilities {
                                badge(ui, cap);
                            }
                        });
                        if let Some(desc) = localized_desc(ui, m) {
                            ui.add_space(6.0);
                            ui.label(desc);
                        }
                        if !m.tags.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                for t in &m.tags {
                                    if tag_chip(ui, t).clicked() {
                                        *tag_click = Some(format!("tag:{t}"));
                                    }
                                }
                            });
                        }
                        // Host-privilege heads-up: some methods may need admin on
                        // this machine. Informational only — no listing, no prompt.
                        if m.permissions.may_require_admin {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(i18n::t("modules.may_require_admin"))
                                    .small()
                                    .color(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)),
                            );
                        }
                        if !m.authors.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    i18n::t("modules.by"),
                                    m.authors.join(", ")
                                ))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                            );
                        }
                        // Git revision this module was installed from — branch on
                        // top, commit below.
                        if let Some((branch, commit)) = git_meta {
                            let mut lines: Vec<String> = Vec::new();
                            if !branch.is_empty() {
                                lines.push(format!("{} - {branch}", i18n::t("about.branch")));
                            }
                            if !commit.is_empty() {
                                lines.push(format!("{} - {commit}", i18n::t("about.commit")));
                            }
                            if !lines.is_empty() {
                                ui.add_space(4.0);
                                for line in lines {
                                    ui.label(
                                        egui::RichText::new(line)
                                            .monospace()
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                }
                            }
                        }
                        // Hard dependencies (capabilities it needs) and optional
                        // integrations (extra features when a provider is loaded).
                        if !m.requires.is_empty() {
                            ui.add_space(6.0);
                            dep_row(
                                ui,
                                &i18n::t("modules.requires"),
                                ui::color::TEXT_MUTED,
                                m.requires.keys(),
                                providers,
                                tag_click,
                            );
                        }
                        if !m.optional.is_empty() {
                            ui.add_space(4.0);
                            dep_row(
                                ui,
                                &i18n::t("modules.optional"),
                                ui::color::ACCENT,
                                m.optional.keys(),
                                providers,
                                tag_click,
                            );
                        }
                    },
                );

                // Right column: Open / Remove / GitHub, stacked at the box end.
                ui.allocate_ui_with_layout(
                    egui::vec2(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Max),
                    |ui| {
                        // Size every action button to the widest label shown on
                        // this card, so the column is one uniform width. `+ 40`
                        // covers the button padding (the larger, primary, one) so
                        // both outline and primary buttons land at the same width.
                        let btn_font = egui::TextStyle::Button.resolve(ui.style());
                        // Resolve each label once so the width measurement and the
                        // buttons use exactly the same (localized) text.
                        let open_lbl = i18n::t("modules.open");
                        let remove_lbl = i18n::t("modules.remove");
                        let github_lbl = i18n::t("modules.github");
                        let update_lbl = (from_git && latest.is_some()).then(|| match latest {
                            Some(v) => format!("{} {v}", i18n::t("modules.update")),
                            None => i18n::t("modules.update"),
                        });

                        let mut labels: Vec<&str> = vec![open_lbl.as_str()];
                        if let Some(u) = &update_lbl {
                            labels.push(u);
                        }
                        labels.push(&remove_lbl);
                        if from_git && m.repo.is_some() {
                            labels.push(&github_lbl);
                        }
                        let max_text = labels
                            .iter()
                            .map(|l| {
                                ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        l.to_uppercase(),
                                        btn_font.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .size()
                                    .x
                                })
                            })
                            .fold(0.0_f32, f32::max);
                        let bw = egui::vec2(max_text + 40.0, ui.spacing().interact_size.y);
                        if this_busy {
                            // Mid-update: spinner in place of the action buttons.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(i18n::t("modules.updating"))
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                            return;
                        }
                        // Open is disabled while this module's runtime downloads.
                        let open_clicked = ui
                            .add_enabled_ui(!runtime_busy, |ui| {
                                ui::outline_button(ui, &open_lbl, bw)
                            })
                            .inner
                            .clicked();
                        if open_clicked {
                            *open = Some(m.name.clone());
                        }
                        if runtime_busy {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new(i18n::t("modules.preparing_runtime"))
                                        .small()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            });
                        }
                        if let Some(update_lbl) = &update_lbl {
                            // Update only shows when a newer release exists.
                            let clicked = ui
                                .add_enabled_ui(!any_busy, |ui| {
                                    ui::primary_button(ui, update_lbl, bw)
                                })
                                .inner
                                .clicked();
                            if clicked {
                                *update = Some(m.name.clone());
                            }
                        }
                        // Red: this one deletes the module and everything under it.
                        if ui::danger_button(ui, &remove_lbl, bw).clicked() {
                            *remove = Some(m.name.clone());
                        }
                        // GitHub only for git-installed modules.
                        if from_git
                            && let Some(repo) = &m.repo
                            && ui::outline_button(ui, &github_lbl, bw).clicked()
                        {
                            ui.output_mut(|o| {
                                o.open_url = Some(egui::OpenUrl::new_tab(repo_url(repo)));
                            });
                        }
                    },
                );
            });
        });
}

/// An "available in the org, not installed" card, with an Install action.
fn available_card(
    ui: &mut egui::Ui,
    r: &RemoteModule,
    installing: &Option<String>,
    add: &mut Option<String>,
    tag_click: &mut Option<String>,
) {
    // While any install runs, every Install button is disabled; the one being
    // installed shows a spinner in place of the button.
    let this_installing = installing.as_deref() == Some(r.repo.as_str());
    let any_installing = installing.is_some();
    egui::Frame::none()
        .fill(ui::color::BG_ELEVATED)
        .stroke(egui::Stroke::new(1.0_f32, ui::color::BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let right_w = 112.0;
                let spacing = ui.spacing().item_spacing.x;
                let left_w = (ui.available_width() - right_w - spacing).max(200.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(left_w);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(r.title.as_deref().unwrap_or(&r.name))
                                    .size(16.0)
                                    .strong(),
                            );
                            if let Some(v) = &r.version {
                                ui.label(
                                    egui::RichText::new(format!("v{v}"))
                                        .monospace()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            }
                            for cap in &r.capabilities {
                                badge(ui, cap);
                            }
                            badge(ui, "not installed");
                        });
                        if let Some(desc) = &r.description {
                            ui.add_space(6.0);
                            ui.label(desc);
                        }
                        if !r.tags.is_empty() {
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                for t in &r.tags {
                                    if tag_chip(ui, t).clicked() {
                                        *tag_click = Some(format!("tag:{t}"));
                                    }
                                }
                            });
                        }
                        // Git status of what a fresh install would fetch — same
                        // shape as installed modules (branch / commit).
                        if let Some(b) = &r.branch {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("branch - {b}"))
                                    .monospace()
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
                            );
                        }
                        if let Some(c) = &r.commit {
                            ui.label(
                                egui::RichText::new(format!("commit - {c}"))
                                    .monospace()
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
                            );
                        }
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(&r.repo)
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Max),
                    |ui| {
                        // Uniform button width = the widest label (Install / GitHub).
                        let btn_font = egui::TextStyle::Button.resolve(ui.style());
                        let install_lbl = i18n::t("modules.install");
                        let github_lbl = i18n::t("modules.github");
                        let max_text = [&install_lbl, &github_lbl]
                            .iter()
                            .map(|l| {
                                ui.fonts(|f| {
                                    f.layout_no_wrap(
                                        l.to_uppercase(),
                                        btn_font.clone(),
                                        egui::Color32::WHITE,
                                    )
                                    .size()
                                    .x
                                })
                            })
                            .fold(0.0_f32, f32::max);
                        let bw = egui::vec2(max_text + 40.0, ui.spacing().interact_size.y);
                        if this_installing {
                            // This module is downloading — spinner + label.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(i18n::t("modules.installing"))
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                        } else {
                            // Disable while another install is in flight.
                            let clicked = ui
                                .add_enabled_ui(!any_installing, |ui| {
                                    ui::primary_button(ui, &install_lbl, bw)
                                })
                                .inner
                                .clicked();
                            if clicked {
                                *add = Some(r.repo.clone());
                            }
                        }
                        if ui::outline_button(ui, &github_lbl, bw).clicked() {
                            let url = r.url.clone();
                            ui.output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url)));
                        }
                    },
                );
            });
        });
}

/// Turn an `owner/repo` (or full URL) into a browsable GitHub URL.
fn repo_url(repo: &str) -> String {
    if repo.contains("://") {
        repo.to_string()
    } else {
        format!("https://github.com/{repo}")
    }
}

/// A dependency row: `Requires: programs`, naming the *modules* that provide the
/// capabilities rather than the capability strings — `programs`, not
/// `programs.local`.
///
/// A name is clickable when something installed provides it; clicking searches
/// for that module so its card comes into view. When nothing provides it, the
/// capability string is shown as plain text instead — still visible, but there
/// is nothing to jump to.
fn dep_row<'a>(
    ui: &mut egui::Ui,
    prefix: &str,
    prefix_color: egui::Color32,
    caps: impl Iterator<Item = &'a String>,
    providers: &HashMap<&str, &ModuleSpec>,
    find: &mut Option<String>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.label(egui::RichText::new(prefix).small().color(prefix_color));
        for (i, cap) in caps.enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("·")
                        .small()
                        .color(ui::color::TEXT_MUTED),
                );
            }
            match providers.get(cap.as_str()) {
                Some(p) => {
                    let clicked = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(&p.name)
                                    .small()
                                    .underline()
                                    .color(ui::color::ACCENT),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked();
                    if clicked {
                        *find = Some(p.name.clone());
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new(cap)
                            .small()
                            .color(ui::color::TEXT_MUTED),
                    );
                }
            }
        }
    });
}

/// A small rounded pill, like Zed's category tags.
/// A clickable tag chip, drawn as `#tag`.
///
/// Distinct from [`badge`] (capabilities), which is inert: a tag is the user's
/// handle for filtering, so it reacts to hover and clicking one puts it in the
/// search box. Laid out by measuring the galley first, so hover is correct on
/// the same frame rather than a frame late.
fn tag_chip(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        format!("#{text}"),
        egui::FontId::proportional(11.0),
        ui::color::ACCENT,
    );
    let pad = egui::vec2(7.0, 2.5);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered();
        let p = ui.painter();
        p.rect_filled(
            rect,
            egui::Rounding::same(4.0),
            if hot {
                ui::color::BG_HOVER
            } else {
                ui::color::BG_WIDGET
            },
        );
        p.rect_stroke(
            rect,
            egui::Rounding::same(4.0),
            egui::Stroke::new(
                1.0_f32,
                if hot {
                    ui::color::ACCENT
                } else {
                    ui::color::BORDER
                },
            ),
        );
        p.galley(
            rect.min + pad,
            galley,
            if hot {
                ui::color::ACCENT_BRIGHT
            } else {
                ui::color::ACCENT
            },
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::none()
        .fill(ui::color::BG_WIDGET)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(11.0)
                    .color(ui::color::TEXT_MUTED),
            );
        });
}

/// The About page. Returns `true` if the "License" button was clicked.
/// Fade a single About-screen element in, staggered by `k`, so the block
/// assembles itself one line at a time when the tab is shown.
fn reveal_item(
    ui: &mut egui::Ui,
    k: usize,
    reveal_at: f64,
    now: f64,
    animate: bool,
    draw: impl FnOnce(&mut egui::Ui),
) {
    if !animate {
        draw(ui);
        return;
    }
    let t = ui::reveal_t(ui, k, reveal_at, now, 0.02, 0.12);
    ui.scope(|ui| {
        ui.set_opacity(t);
        draw(ui);
    });
}

/// The Barracuda logo, embedded as its source SVG (a flat set of straight-edged
/// facets — only `M`/`l`/`z` path commands, no curves), so it can be rendered
/// with the egui painter like the Limen mark instead of pulling in an SVG/image
/// dependency.
const BARRACUDA_SVG: &str = include_str!("../../resources/barracuda-white.svg");

/// Parsed Barracuda artwork: each facet as a polygon of points, plus the overall
/// bounding box (for aspect-correct fitting into any target rect).
struct Barracuda {
    polys: Vec<Vec<[f32; 2]>>,
    min: egui::Pos2,
    max: egui::Pos2,
}

/// Parse one SVG path `d` string into a polyline of absolute points. Supports the
/// commands the logo actually uses — `M`/`m`, `L`/`l`, `H`/`h`, `V`/`v`, `C`/`c`
/// (cubic béziers, flattened to short segments), and `Z`/`z` — in both absolute
/// (uppercase) and relative (lowercase) forms, including implicit operand repeats.
fn parse_facet(d: &str) -> Vec<[f32; 2]> {
    let b = d.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut cur = [0.0f32, 0.0f32];
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let mut cmd = 0u8;

    let read_number = |b: &[u8], i: &mut usize| -> Option<f32> {
        while *i < b.len() && matches!(b[*i], b',' | b' ' | b'\n' | b'\t' | b'\r') {
            *i += 1;
        }
        let start = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        while *i < b.len() && (b[*i].is_ascii_digit() || b[*i] == b'.') {
            *i += 1;
        }
        if *i == start {
            return None;
        }
        std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
    };

    while i < n {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            i += 1;
            continue;
        }
        if matches!(c, b',' | b' ' | b'\n' | b'\t' | b'\r') {
            i += 1;
            continue;
        }
        let rel = cmd.is_ascii_lowercase();
        match cmd.to_ascii_uppercase() {
            b'H' => {
                let Some(x) = read_number(b, &mut i) else {
                    break;
                };
                cur[0] = if rel { cur[0] + x } else { x };
                pts.push(cur);
            }
            b'V' => {
                let Some(y) = read_number(b, &mut i) else {
                    break;
                };
                cur[1] = if rel { cur[1] + y } else { y };
                pts.push(cur);
            }
            b'C' => {
                let mut num = [0.0f32; 6];
                let mut ok = true;
                for slot in &mut num {
                    match read_number(b, &mut i) {
                        Some(v) => *slot = v,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    break;
                }
                let abs = |dx: f32, dy: f32| {
                    if rel {
                        [cur[0] + dx, cur[1] + dy]
                    } else {
                        [dx, dy]
                    }
                };
                let p0 = cur;
                let c1 = abs(num[0], num[1]);
                let c2 = abs(num[2], num[3]);
                let end = abs(num[4], num[5]);
                // Flatten the cubic into a few straight segments.
                const STEPS: usize = 8;
                for k in 1..=STEPS {
                    let t = k as f32 / STEPS as f32;
                    let u = 1.0 - t;
                    let bx = u * u * u * p0[0]
                        + 3.0 * u * u * t * c1[0]
                        + 3.0 * u * t * t * c2[0]
                        + t * t * t * end[0];
                    let by = u * u * u * p0[1]
                        + 3.0 * u * u * t * c1[1]
                        + 3.0 * u * t * t * c2[1]
                        + t * t * t * end[1];
                    pts.push([bx, by]);
                }
                cur = end;
            }
            // M / L (and their implicit repeats): a single coordinate pair. After
            // an `M`, implicit following pairs are linetos, so keep `cmd` as-is —
            // both M and L consume one pair here, which is the desired behaviour.
            _ => {
                let Some(x) = read_number(b, &mut i) else {
                    break;
                };
                let Some(y) = read_number(b, &mut i) else {
                    break;
                };
                cur = if rel {
                    [cur[0] + x, cur[1] + y]
                } else {
                    [x, y]
                };
                pts.push(cur);
            }
        }
    }
    pts
}

/// Parse (once) the embedded Barracuda SVG into paintable facets.
fn barracuda_art() -> &'static Barracuda {
    static ART: std::sync::OnceLock<Barracuda> = std::sync::OnceLock::new();
    ART.get_or_init(|| {
        let mut polys: Vec<Vec<[f32; 2]>> = Vec::new();
        // Each real facet lives in a `<path d="…">`; splitting on `<path` and
        // taking the first `d="` per chunk skips the metadata <g id="…"> nodes.
        for seg in BARRACUDA_SVG.split("<path").skip(1) {
            let Some(s) = seg.find("d=\"") else { continue };
            let rest = &seg[s + 3..];
            let Some(e) = rest.find('"') else { continue };
            let pts = parse_facet(&rest[..e]);
            if pts.len() >= 3 {
                polys.push(pts);
            }
        }
        let mut min = egui::pos2(f32::MAX, f32::MAX);
        let mut max = egui::pos2(f32::MIN, f32::MIN);
        for p in polys.iter().flatten() {
            min.x = min.x.min(p[0]);
            min.y = min.y.min(p[1]);
            max.x = max.x.max(p[0]);
            max.y = max.y.max(p[1]);
        }
        Barracuda { polys, min, max }
    })
}

/// Draw the Barracuda logo filled with `color`, fitted (aspect-correct, centered)
/// into `rect`.
fn draw_barracuda(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let art = barracuda_art();
    let src = art.max - art.min;
    if src.x <= 0.0 || src.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / src.x).min(rect.height() / src.y);
    let drawn = src * scale;
    let origin = rect.center() - drawn * 0.5;
    let map = |p: &[f32; 2]| {
        egui::pos2(
            origin.x + (p[0] - art.min.x) * scale,
            origin.y + (p[1] - art.min.y) * scale,
        )
    };
    for poly in &art.polys {
        let pts: Vec<egui::Pos2> = poly.iter().map(map).collect();
        painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
}

/// Splash phase durations (seconds): a fully-transparent blank lead (so the
/// window's clumsy opaque startup frames read as transparent and are trimmable),
/// then fade transparent→opaque, hold fully opaque, then the exit animation.
/// Their sum is the total splash length.
const SPLASH_LEAD: f32 = 0.4;
const SPLASH_FADE_IN: f32 = 0.2;
const SPLASH_HOLD: f32 = 0.75;
const SPLASH_ANIM: f32 = 0.25;
const SPLASH_SECS: f32 = SPLASH_LEAD + SPLASH_FADE_IN + SPLASH_HOLD + SPLASH_ANIM;

/// Paint the startup splash: the Limen mark + Barracuda logo centre-screen. When
/// `animated`, it's fully transparent for `SPLASH_LEAD`, then fades in over
/// `SPLASH_FADE_IN`, holds for `SPLASH_HOLD`, then runs the `SPLASH_ANIM` exit
/// (scanline sweep + fade out). When not, the marks simply appear (opaque, no
/// motion) after the transparent lead. `t` is elapsed seconds since it began.
fn splash_screen(ctx: &egui::Context, t: f32, animated: bool) {
    egui::CentralPanel::default()
        // Transparent frame — the framebuffer is cleared transparent during the
        // splash (see `clear_color`), so only the icons show over the desktop.
        .frame(egui::Frame::none())
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            // Envelope: blank transparent lead, then either fade+scale in / hold /
            // fade out (animated) or a plain opaque appearance (static). `vis`
            // fades each element's own alpha (the window is transparent — there's
            // no background to veil against).
            let (vis, scale) = if animated {
                let vin = ui::ease_out(((t - SPLASH_LEAD) / SPLASH_FADE_IN).clamp(0.0, 1.0));
                let exit = ((t - (SPLASH_LEAD + SPLASH_FADE_IN + SPLASH_HOLD)) / SPLASH_ANIM)
                    .clamp(0.0, 1.0);
                let vout = 1.0 - ui::smoothstep(exit);
                (vin * vout, 0.92 + 0.08 * vin)
            } else {
                (if t >= SPLASH_LEAD { 1.0 } else { 0.0 }, 1.0)
            };

            let mark = 168.0 * scale;
            let gap = 44.0;
            let cx = rect.center().x;
            let cy = rect.center().y - 8.0;
            let half = mark / 2.0 + gap / 2.0 + 0.5;
            let painter = ui.painter();

            // Left: Limen mark · amber divider · right: Barracuda logo.
            let r1 =
                egui::Rect::from_center_size(egui::pos2(cx - half, cy), egui::vec2(mark, mark));
            let r2 =
                egui::Rect::from_center_size(egui::pos2(cx + half, cy), egui::vec2(mark, mark));
            draw_brand(painter, r1, vis, false);
            draw_barracuda(painter, r2, ui::with_alpha(ui::color::ACCENT_BRIGHT, vis));
            let dh = 92.0 * scale;
            painter.vline(
                cx,
                (cy - dh / 2.0)..=(cy + dh / 2.0),
                egui::Stroke::new(1.0_f32, ui::with_alpha(ui::color::ACCENT, 0.4 * vis)),
            );

            // A single amber scanline sweeps down across the marks (animated only),
            // over the hold into the start of the exit.
            if animated {
                let scan = ((t - SPLASH_LEAD - SPLASH_FADE_IN) / SPLASH_HOLD).clamp(0.0, 1.0);
                if scan > 0.0 && scan < 1.0 {
                    let y = egui::lerp((cy - mark * 0.7)..=(cy + mark * 0.7), scan);
                    let a = (1.0 - (scan * 2.0 - 1.0).abs()) * 0.5 * vis;
                    painter.hline(
                        (cx - mark * 1.4)..=(cx + mark * 1.4),
                        y,
                        egui::Stroke::new(1.5_f32, ui::with_alpha(ui::color::ACCENT_BRIGHT, a)),
                    );
                }
            }
        });
}

fn about_view(ui: &mut egui::Ui, reveal_at: f64) -> bool {
    let muted = ui::color::TEXT_MUTED;
    let mut license_clicked = false;
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();

    // Scroll so nothing (footer, button) is clipped on short windows / high scale.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Push the block toward the vertical middle.
            let top = (ui.available_height() * 0.10).clamp(20.0, 120.0);
            ui.add_space(top);

            ui.vertical_centered(|ui| {
                ui.set_max_width(480.0);

                reveal_item(ui, 0, reveal_at, now, animate, |ui| {
                    // The Limen mark paired with the Barracuda logo — one product.
                    // Allocate the whole row as one block so `vertical_centered`
                    // centers it.
                    let mark = 84.0_f32;
                    let gap = 12.0_f32;
                    let row_w = mark + gap + 1.0 + gap + mark;
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(row_w, mark), egui::Sense::hover());

                    let r1 = egui::Rect::from_min_size(rect.left_top(), egui::vec2(mark, mark));
                    draw_brand(ui.painter(), r1, 1.0, false);

                    let sep_x = rect.left() + mark + gap + 0.5;
                    let half = 26.0_f32;
                    ui.painter().vline(
                        sep_x,
                        (rect.center().y - half)..=(rect.center().y + half),
                        egui::Stroke::new(1.0_f32, ui::with_alpha(ui::color::ACCENT, 0.35)),
                    );

                    let r2 = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - mark, rect.top()),
                        egui::vec2(mark, mark),
                    );
                    draw_barracuda(ui.painter(), r2, ui::color::ACCENT_BRIGHT);
                });

                ui.add_space(14.0);
                reveal_item(ui, 1, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new("LIMEN").size(30.0).strong());
                });
                reveal_item(ui, 2, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "v{} · {} {}",
                            env!("CARGO_PKG_VERSION"),
                            env!("LIMEN_GIT_BRANCH"),
                            i18n::t("about.branch"),
                        ))
                        .monospace()
                        .color(muted),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            i18n::t("about.commit"),
                            env!("LIMEN_GIT_COMMIT")
                        ))
                        .monospace()
                        .small()
                        .color(muted),
                    );
                });

                ui.add_space(18.0);
                reveal_item(ui, 3, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new(i18n::t("about.tagline")).size(15.0));
                });

                ui.add_space(18.0);
                reveal_item(ui, 4, reveal_at, now, animate, |ui| {
                    ui.separator();
                    ui.add_space(14.0);
                    // Both buttons share the wider label's width so they match,
                    // stacked (License on top, GitHub below).
                    let btn_font = egui::TextStyle::Button.resolve(ui.style());
                    let lic = i18n::t("about.view_license");
                    let gh = i18n::t("about.github");
                    let max_text = [&lic, &gh]
                        .iter()
                        .map(|l| {
                            ui.fonts(|f| {
                                f.layout_no_wrap(
                                    l.to_uppercase(),
                                    btn_font.clone(),
                                    egui::Color32::WHITE,
                                )
                                .size()
                                .x
                            })
                        })
                        .fold(0.0_f32, f32::max);
                    let bw = egui::vec2(max_text + 32.0, ui.spacing().interact_size.y);
                    if ui::outline_button(ui, &lic, bw).clicked() {
                        license_clicked = true;
                    }
                    ui.add_space(8.0);
                    if ui::outline_button(ui, &gh, bw).clicked() {
                        let url = format!("https://github.com/{}", limen_core::update::APP_REPO);
                        ui.output_mut(|o| o.open_url = Some(egui::OpenUrl::new_tab(url)));
                    }
                });

                ui.add_space(16.0);
                reveal_item(ui, 5, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new(i18n::t("about.created_by"))
                            .small()
                            .color(muted),
                    );
                });
                ui.add_space(12.0);
            });
        });

    license_clicked
}

/// The License page — the embedded GPLv3 text, scrollable.
fn license_view(ui: &mut egui::Ui, reveal_at: f64) {
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();
    // Center the license in a fixed-width column.
    ui.vertical_centered(|ui| {
        ui.set_max_width(720.0);
        reveal_item(ui, 0, reveal_at, now, animate, |ui| {
            ui.heading(i18n::t("license.title"));
            ui.label(egui::RichText::new(i18n::t("license.intro")).color(ui::color::TEXT_MUTED));
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
        });
        reveal_item(ui, 1, reveal_at, now, animate, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut text = LICENSE_TEXT;
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_width(f32::INFINITY)
                            .code_editor(),
                    );
                });
        });
    });
}

/// Draw the Limen brand mark — the diamond ring with a solid core — retinted to
/// the Barracuda amber-HUD palette: a warm near-black tile under an amber→orange
/// diagonal-gradient diamond, so it reads as one product with the Barracuda logo.
///
/// (The OS icon files — `resources/icon.png`/`.ico` — still carry the original
/// cyan mark; regenerating those needs image tooling and is a separate step.)
fn draw_brand(painter: &egui::Painter, rect: egui::Rect, alpha: f32, show_tile: bool) {
    use egui::{Color32, Mesh, Pos2, Shape};

    // Scale every colour's alpha by `alpha` (1.0 = opaque) so the mark can fade
    // as a whole — used by the transparent startup splash.
    let fa = |c: Color32| {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * alpha).round() as u8)
    };
    // Diamond: one diagonal gradient across the whole device — light at the
    // top-left, dark at the bottom-right — so the top *and left* vertices are
    // bright amber while the right and bottom ones deepen to orange.
    let light = Color32::from_rgb(0xf4, 0xc0, 0x78); // bright amber
    let dark = Color32::from_rgb(0xf9, 0x73, 0x16); // orange

    let s = rect.width().min(rect.height()) / 256.0;

    // The dark rounded tile (the app-icon background). Skipped for the splash,
    // where the mark floats transparently beside the Barracuda logo.
    if show_tile {
        let tile_top = fa(Color32::from_rgb(0x24, 0x1a, 0x10));
        let tile_bottom = fa(Color32::from_rgb(0x0d, 0x0a, 0x06));
        let tile = egui::Rect::from_center_size(rect.center(), egui::vec2(224.0 * s, 224.0 * s));
        painter.add(Shape::mesh(rounded_rect_mesh(
            tile,
            44.0 * s,
            tile_top,
            tile_bottom,
        )));
    }

    let c = rect.center();
    let diamond = |half: f32| {
        [
            Pos2::new(c.x, c.y - half),
            Pos2::new(c.x + half, c.y),
            Pos2::new(c.x, c.y + half),
            Pos2::new(c.x - half, c.y),
        ]
    };
    let outer = diamond(88.0 * s);
    let inner = diamond(48.0 * s);
    let core = diamond(20.0 * s);

    // Position along the top-left → bottom-right diagonal, normalized to 0..1.
    let span = 2.0 * 88.0 * s;
    let shade = |p: Pos2| {
        fa(lerp_color(
            light,
            dark,
            ((p.x - c.x) + (p.y - c.y)) / span + 0.5,
        ))
    };

    let mut mesh = Mesh::default();
    let mut quad = |pts: [Pos2; 4]| {
        let base = mesh.vertices.len() as u32;
        for p in pts {
            mesh.colored_vertex(p, shade(p));
        }
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    };
    // The ring as four convex trapezoids. Building it this way — rather than
    // punching a hole with the background color — keeps the tile's own gradient
    // visible through the middle.
    for i in 0..4 {
        let j = (i + 1) % 4;
        quad([outer[i], outer[j], inner[j], inner[i]]);
    }
    // The core is not a lighter accent: it's the same gradient, continued.
    quad(core);
    painter.add(Shape::mesh(mesh));

    // A flat highlight runs just inside the outer edge — the one part of the
    // device that doesn't follow the gradient. It measures ~1.4px on the icon's
    // 256px grid, so at the sizes drawn here it lands sub-pixel and reads as a
    // faint sheen along the edge rather than a distinct line.
    painter.add(Shape::closed_line(
        outer.to_vec(),
        egui::Stroke::new(1.4 * s, fa(Color32::from_rgb(0xff, 0xe0, 0xb0))),
    ));
}

/// Linear blend between two colors, `t` clamped to `0..=1`.
fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A rounded rectangle as a colored mesh, filled with a vertical `top`→`bottom`
/// gradient. `egui`'s `rect_filled` takes a single color, so a gradient fill has
/// to be built by hand: walk the outline, then fan-triangulate from the center
/// (the shape is convex, so a fan is exact).
fn rounded_rect_mesh(
    rect: egui::Rect,
    radius: f32,
    top: egui::Color32,
    bottom: egui::Color32,
) -> egui::Mesh {
    /// Segments per corner arc — enough to look round at the sizes we draw.
    const SEGMENTS: usize = 8;

    let radius = radius.min(rect.width().min(rect.height()) / 2.0);
    // Arc centers with their start/end angles, walked clockwise from top-left.
    let corners = [
        (
            egui::pos2(rect.left() + radius, rect.top() + radius),
            180.0f32,
            270.0f32,
        ),
        (
            egui::pos2(rect.right() - radius, rect.top() + radius),
            270.0,
            360.0,
        ),
        (
            egui::pos2(rect.right() - radius, rect.bottom() - radius),
            0.0,
            90.0,
        ),
        (
            egui::pos2(rect.left() + radius, rect.bottom() - radius),
            90.0,
            180.0,
        ),
    ];
    let mut outline = Vec::with_capacity(4 * (SEGMENTS + 1));
    for (center, from, to) in corners {
        for i in 0..=SEGMENTS {
            let a = (from + (to - from) * i as f32 / SEGMENTS as f32).to_radians();
            outline.push(egui::pos2(
                center.x + radius * a.cos(),
                center.y + radius * a.sin(),
            ));
        }
    }

    let shade = |y: f32| lerp_color(top, bottom, (y - rect.top()) / rect.height());
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), shade(rect.center().y));
    for p in &outline {
        mesh.colored_vertex(*p, shade(p.y));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    mesh
}

#[allow(clippy::too_many_arguments)]
/// Render a detail tab opened from a table row action (e.g. "About device").
/// Its view is interactive too, so buttons inside it (e.g. "Open path") dispatch.
fn detail_view(
    ui: &mut egui::Ui,
    id: u64,
    detail_tabs: &mut HashMap<u64, DetailTab>,
    action: &mut Option<ui::Invoke>,
) {
    let Some(tab) = detail_tabs.get_mut(&id) else {
        ui.label(i18n::t("detail.unavailable"));
        return;
    };
    let heading = if tab.title.is_empty() {
        i18n::t("tab.details")
    } else {
        tab.title.clone()
    };
    if let Some(err) = &tab.error {
        ui.heading(&heading);
        ui.separator();
        ui.colored_label(egui::Color32::LIGHT_RED, err);
        return;
    }
    match &tab.view {
        Some(v) => {
            let heading = if v.title.is_empty() {
                heading
            } else {
                v.title.clone()
            };
            ui.heading(heading);
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Dev UI-kit preview: no entrance animation.
                    if let Some(a) = ui::render_view(ui, v, &mut tab.inputs, None, 0.0) {
                        *action = Some(a);
                    }
                });
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(i18n::t("detail.loading"));
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn module_view(
    ui: &mut egui::Ui,
    name: &str,
    reveal_at: f64,
    view: &Option<ui::View>,
    view_error: &Option<String>,
    inputs: &mut HashMap<String, String>,
    output: &str,
    busy_action: Option<&ui::Action>,
    action: &mut Option<ui::Invoke>,
) {
    match view {
        Some(v) => {
            ui.heading(if v.title.is_empty() { name } else { &v.title });
            ui.separator();
            // Scroll the module content so tall views (long tables, big scale)
            // don't overflow the window.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // The in-flight button shows a spinner (via busy_action).
                    if let Some(a) = ui::render_view(ui, v, inputs, busy_action, reveal_at) {
                        *action = Some(a);
                    }

                    // Show the Result pane only when a method returned output.
                    if !output.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new(i18n::t("module.result")).strong());
                        let mut text = output;
                        ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .desired_width(f32::INFINITY)
                                .code_editor(),
                        );
                    }
                });
        }
        None => {
            if let Some(err) = view_error {
                ui.heading(name);
                ui.separator();
                // Rendered as Markdown so failure messages can bold the required
                // capability/module (e.g. "requires capability **crowdstrike**").
                ui::markdown(ui, err);
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(i18n::t("detail.loading"));
                });
            }
        }
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    /// The remote and installed matchers share their term logic, so the simpler
    /// struct exercises both. `RemoteModule` needs no launch/permission setup.
    fn spec() -> RemoteModule {
        RemoteModule {
            name: "programs-banlist-ua".into(),
            title: Some("Banned Programs in Ukraine".into()),
            repo: "CRC-BARRACUDA/limen-programs-banlist-ua".into(),
            description: Some("Flags installed software banned in Ukraine.".into()),
            url: String::new(),
            version: Some("0.1.0".into()),
            capabilities: vec!["banlist.ua".into()],
            tags: ["ua", "programs", "ssscip.banlist"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            branch: None,
            commit: None,
        }
    }

    #[test]
    fn a_bare_word_searches_names_only() {
        // "banned" is in the display name -> hit.
        assert!(remote_matches(&spec(), &parse_query("banned")));
        // "software" appears only in the description -> miss, by design.
        assert!(!remote_matches(&spec(), &parse_query("software")));
    }

    #[test]
    fn qualifiers_target_their_field() {
        let m = spec();
        assert!(remote_matches(&m, &parse_query("tag:ua")));
        assert!(remote_matches(&m, &parse_query("description:software")));
        assert!(remote_matches(&m, &parse_query("cap:banlist")));
        assert!(!remote_matches(&m, &parse_query("tag:windows")));
        // A tag value is not a name, and vice versa.
        assert!(!remote_matches(&m, &parse_query("name:ssscip")));
        assert!(remote_matches(&m, &parse_query("tag:ssscip")));
    }

    #[test]
    fn terms_are_anded_and_shorthands_work() {
        let m = spec();
        assert!(remote_matches(&m, &parse_query("tag:ua programs")));
        // Second term fails -> whole query fails.
        assert!(!remote_matches(&m, &parse_query("tag:ua nonsense")));
        assert!(remote_matches(&m, &parse_query("desc:banned")));
    }

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(parse_query("   ").is_empty());
        assert!(remote_matches(&spec(), &[]));
    }

    /// An unknown prefix must not be read as a field, or it would silently match
    /// nothing instead of being searched literally.
    #[test]
    fn an_unknown_prefix_is_searched_literally() {
        let terms = parse_query("weird:thing");
        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].field, Field::Name);
        assert_eq!(terms[0].text, "weird:thing");
    }
}

#[cfg(test)]
mod barracuda_tests {
    use super::*;

    #[test]
    fn barracuda_svg_parses_into_facets() {
        let art = barracuda_art();
        // All 53 <path> facets parse into drawable polygons (curves flattened).
        assert_eq!(art.polys.len(), 53, "expected 53 drawable facets");
        assert!(art.polys.iter().all(|p| p.len() >= 3));
        // Bounding box must sit inside the SVG viewBox (418,31 .. 1118,731).
        assert!(art.min.x >= 418.0 && art.min.y >= 31.0, "min {:?}", art.min);
        assert!(
            art.max.x <= 1118.0 && art.max.y <= 731.0,
            "max {:?}",
            art.max
        );
        assert!(art.max.x - art.min.x > 100.0 && art.max.y - art.min.y > 100.0);
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    fn page(marker: &str) -> ModulePage {
        let view: ui::View =
            serde_json::from_str(&format!(r#"{{"title":"{marker}","widgets":[]}}"#)).unwrap();
        let popup: ui::View = serde_json::from_str(
            r#"{"title":"Scan settings","modal":"loki.settings","widgets":[]}"#,
        )
        .unwrap();
        ModulePage {
            view: Some(view),
            modal_stack: vec![popup],
            inputs: [("target".to_string(), format!("/srv/{marker}"))]
                .into_iter()
                .collect(),
            busy: true,
            ..Default::default()
        }
    }

    fn title(p: &ModulePage) -> String {
        p.view.as_ref().map(|v| v.title.clone()).unwrap_or_default()
    }

    /// The point of the whole thing: what a tab was showing — including an open
    /// pop-up and a half-filled form — is still there when you come back.
    #[test]
    fn a_tab_keeps_its_screen_while_another_is_shown() {
        let mut stored = HashMap::new();

        // On "loki", with a settings pop-up open. Switch to "banlist".
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        assert!(onscreen.view.is_none(), "banlist has never been shown");
        assert!(onscreen.modal_stack.is_empty());

        // Come back. Everything is as it was left.
        let back = swap_page(onscreen, &mut stored, Some("banlist"), Some("loki"));
        assert_eq!(title(&back), "loki");
        assert_eq!(back.modal_stack.len(), 1, "the pop-up survived the trip");
        assert_eq!(back.modal_stack[0].modal.as_deref(), Some("loki.settings"));
        assert_eq!(back.inputs.get("target").unwrap(), "/srv/loki");
        assert!(back.busy, "and so did what it was waiting on");
    }

    /// A non-module tab starts empty. Without this the last module's view — and
    /// its pop-up — would show behind an About page.
    #[test]
    fn a_tab_that_is_not_a_module_shows_nothing() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), None);
        assert!(onscreen.view.is_none());
        assert!(onscreen.modal_stack.is_empty());
        assert!(!onscreen.busy);
        // ...and loki's screen was not lost, only set aside.
        assert_eq!(title(stored.get("loki").unwrap()), "loki");
    }

    /// Leaving a page that belongs to no tab drops it rather than filing it
    /// under someone else's name.
    #[test]
    fn a_page_with_no_tab_is_not_kept() {
        let mut stored = HashMap::new();
        let _ = swap_page(page("about"), &mut stored, None, Some("loki"));
        assert!(stored.is_empty(), "nothing was filed: {:?}", stored.keys());
    }

    /// Two tabs do not share a form. Their inputs are the most obvious thing to
    /// leak, since the fields they name are usually identical.
    #[test]
    fn two_module_tabs_do_not_share_their_inputs() {
        let mut stored = HashMap::new();
        let mut onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        onscreen
            .inputs
            .insert("target".to_string(), "/srv/banlist".to_string());

        let loki = swap_page(onscreen, &mut stored, Some("banlist"), Some("loki"));
        assert_eq!(loki.inputs.get("target").unwrap(), "/srv/loki");
        let banlist = swap_page(loki, &mut stored, Some("loki"), Some("banlist"));
        assert_eq!(banlist.inputs.get("target").unwrap(), "/srv/banlist");
    }

    /// Closing a tab forgets it: reopening the module starts fresh rather than
    /// resurrecting a pop-up from a tab the user closed.
    #[test]
    fn a_closed_tab_leaves_nothing_behind() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), None);
        assert!(stored.contains_key("loki"));

        stored.remove("loki"); // what close_tab does
        let reopened = swap_page(onscreen, &mut stored, None, Some("loki"));
        assert!(reopened.view.is_none());
        assert!(reopened.modal_stack.is_empty());
    }

    /// A reload restarts every module, so a stored screen describes a connection
    /// that no longer exists — and a stored pop-up would sit over a module that
    /// has forgotten it was ever open.
    #[test]
    fn a_reload_clears_what_every_tab_was_showing() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        let _ = swap_page(page("banlist"), &mut stored, Some("banlist"), None);
        assert_eq!(stored.len(), 2);

        stored.clear(); // what the reload does
        let after = swap_page(onscreen, &mut stored, None, Some("loki"));
        assert!(after.view.is_none(), "nothing survives a reload");
    }
}
