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
use std::time::Duration;

use eframe::egui;
use limen_core::{ModuleSpec, Runtime};
use limen_registry::RemoteModule;

use crate::ui;
use crate::worker::{Command, Event, RunTag, Worker};

/// An open tab. All tabs are closable.
#[derive(Clone, PartialEq)]
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
            Tab::About => "About".into(),
            Tab::License => "License".into(),
            Tab::Modules => "Modules".into(),
            Tab::Module(n) => n.clone(),
            Tab::Settings => "Settings".into(),
            Tab::Developer => "Developer".into(),
            Tab::Update => "Update".into(),
            // The real label comes from the stored view's title (looked up in the
            // tab bar); this is only a fallback.
            Tab::Detail { .. } => "Details".into(),
        }
    }
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
    /// An elevated action awaiting the user's consent (shown as a dialog).
    pending_action: Option<ui::Invoke>,

    /// Open detail tabs (from row actions), keyed by the id in `Tab::Detail`.
    detail_tabs: HashMap<u64, DetailTab>,
    /// Monotonic id for the next detail tab.
    next_detail_id: u64,

    /// Open tabs (in order) and the active index.
    tabs: Vec<Tab>,
    active: usize,
    /// Per-module visit counts, for the "frequent" quick-open chips.
    visits: HashMap<String, u32>,

    view: Option<ui::View>,
    view_error: Option<String>,
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

    /// Module names pinned to the tab bar, in order (persisted in settings).
    pinned: Vec<String>,

    /// Global UI scale as a percentage (persisted in settings).
    ui_scale: f32,

    /// Whether UI animations are enabled (persisted in settings).
    animations: bool,

    /// When the About tab was last shown — drives its staggered content reveal.
    about_revealed_at: Option<f64>,
    /// Reveal timers for the Settings/Developer/License tab entrance animations.
    settings_revealed_at: Option<f64>,
    developer_revealed_at: Option<f64>,
    license_revealed_at: Option<f64>,
    /// Developer sub-tab the reveal was started for; a change replays it.
    shown_dev_tab: DevTab,
    /// When the Modules tab was last shown — drives the staggered list reveal.
    modules_revealed_at: Option<f64>,
    /// The filter the current reveal was started for; a change replays the reveal.
    shown_filter: ModuleFilter,
    /// When the async GitHub list arrived — the reveal base for the available
    /// cards, so they animate in when they load (not from tab-open).
    remote_revealed_at: Option<f64>,

    /// Modules mid-removal: name → animation start time (or a negative sentinel
    /// once the actual removal has been sent). Drives the exit animation.
    removing: HashMap<String, f64>,

    /// An available app update (from the background check), if any.
    update: Option<limen_core::UpdateInfo>,
    /// True while an update download/install is in flight.
    updating: bool,
    /// A portable interpreter currently being installed (e.g. "Python"), if any.
    installing_runtime: Option<String>,
}

impl LimenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Vec<std::path::PathBuf>) -> Self {
        ui::apply_theme(&cc.egui_ctx);
        let animations = limen_core::Config::load()
            .map(|c| c.animations)
            .unwrap_or(true);
        ui::set_animations(animations);
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
            detail_tabs: HashMap::new(),
            next_detail_id: 0,
            tabs: vec![Tab::About, Tab::Modules],
            active: 0,
            visits: HashMap::new(),
            view: None,
            view_error: None,
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
            pinned: limen_core::Config::load()
                .map(|c| c.pinned_modules)
                .unwrap_or_default(),
            ui_scale: {
                let pct = limen_core::Config::load()
                    .map(|c| c.ui_scale_percent)
                    .unwrap_or(0);
                if pct == 0 { 100.0 } else { pct as f32 }
            },
            animations,
            about_revealed_at: None,
            settings_revealed_at: None,
            developer_revealed_at: None,
            license_revealed_at: None,
            shown_dev_tab: DevTab::DevMode,
            modules_revealed_at: None,
            shown_filter: ModuleFilter::All,
            remote_revealed_at: None,
            removing: HashMap::new(),
            update: None,
            updating: false,
            installing_runtime: None,
        }
    }

    /// The active tab (cloned).
    fn active_tab(&self) -> Option<Tab> {
        self.tabs.get(self.active).cloned()
    }

    /// Open `tab` (focus it if already open, else append), and activate it.
    fn open_tab(&mut self, tab: Tab) {
        match self.tabs.iter().position(|t| *t == tab) {
            Some(i) => self.active = i,
            None => {
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
            }
        }
    }

    /// Close the tab at `index`.
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        // Free a detail tab's stored view when its tab closes.
        if let Tab::Detail { id } = self.tabs[index] {
            self.detail_tabs.remove(&id);
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if self.active > index {
            self.active -= 1;
        }
    }

    /// The `n` most-visited installed modules that aren't already open as tabs.
    fn frequent_modules(&self, n: usize) -> Vec<String> {
        let mut v: Vec<(&String, u32)> = self
            .visits
            .iter()
            .filter(|(name, c)| {
                **c > 0
                    && self.modules.iter().any(|m| &m.name == *name)
                    && !self.tabs.iter().any(|t| *t == Tab::Module((*name).clone()))
            })
            .map(|(name, c)| (name, *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v.into_iter()
            .take(n)
            .map(|(name, _)| name.clone())
            .collect()
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

    /// Pin or unpin a module, then persist the pin list to settings.json.
    fn toggle_pin(&mut self, name: &str) {
        if let Some(i) = self.pinned.iter().position(|n| n == name) {
            self.pinned.remove(i);
        } else {
            self.pinned.push(name.to_string());
        }
        // Persist without clobbering other settings.
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.pinned_modules = self.pinned.clone();
            let _ = cfg.save();
        }
    }

    /// Recompute which sensitive modules the user has granted (trusted at their
    /// current digest). Cheap enough to run on module load / after a grant.
    fn refresh_trust(&mut self) {
        let trust =
            limen_registry::TrustStore::load(&limen_core::paths::home()).unwrap_or_default();
        self.trusted.clear();
        for m in &self.modules {
            if !m.permissions.sensitive() {
                continue;
            }
            if let Ok(digest) = limen_registry::digest_dir(&m.cwd)
                && trust.is_trusted(&m.name, &digest)
            {
                self.trusted.insert(m.name.clone());
            }
        }
    }

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
                    self.refresh_trust();
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
                Event::RemoteModules(result) => {
                    self.remote_loading = false;
                    match result {
                        Ok(list) => {
                            self.remote = list;
                            self.remote_error = None;
                            // Timestamp the load so the available cards animate in
                            // when they actually arrive (async), not from tab-open.
                            self.remote_revealed_at = Some(now);
                        }
                        Err(e) => self.remote_error = Some(e),
                    }
                }
                Event::RunDone { tag, result } => match tag {
                    RunTag::Ui { module } => {
                        // Ignore late results if the active tab isn't that module.
                        if self.active_tab() == Some(Tab::Module(module)) {
                            self.busy = false;
                            match result {
                                Ok(v) => match serde_json::from_value::<ui::View>(v) {
                                    Ok(view) => {
                                        self.view = Some(view);
                                        self.view_error = None;
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
                                        self.view = Some(view);
                                        self.view_error = None;
                                        self.output.clear();
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
                    self.refresh_trust();
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
        self.open_tab(Tab::Module(name.clone()));
        *self.visits.entry(name.clone()).or_insert(0) += 1;
        self.view = None;
        self.view_error = None;
        self.inputs.clear();
        self.output.clear();
        // A module that failed to start has no live connection — show why, here,
        // instead of trying to call it (or blocking the whole app).
        if let Some(err) = self.failed.get(&name) {
            self.view_error = Some(format!("This module failed to start:\n\n{err}"));
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

    fn dispatch(&mut self, invoke: ui::Invoke) {
        // Base params come from the active view's inputs (a module tab's search
        // box etc.); a detail tab has no shared inputs. Row/menu args (the row
        // `id`, `via`, …) are merged on top.
        let mut params: serde_json::Map<String, serde_json::Value> = match self.active_tab() {
            Some(Tab::Module(_)) => match self
                .view
                .as_ref()
                .map(|v| ui::collect_params(v, &self.inputs))
            {
                Some(serde_json::Value::Object(m)) => m,
                _ => serde_json::Map::new(),
            },
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now_t = ctx.input(|i| i.time);
        self.drain_events(now_t);

        // Apply the global UI scale (set_zoom_factor no-ops if unchanged).
        ctx.set_zoom_factor(self.ui_scale / 100.0);

        // Reload is requested from the Modules page (set during the central panel).
        let mut reload = false;

        // Intents collected while rendering, applied after.
        let mut open_tab: Option<Tab> = None;
        let mut switch_to: Option<usize> = None;
        let mut close_idx: Option<usize> = None;
        let mut scale_changed = false;
        let mut anim_changed = false;
        let mut dev_applied = false;

        // Title bar: brand + quick-open buttons + status.
        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(0x21, 0x25, 0x2b))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // App icon (the ◈ brand mark) in place of the wordmark.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    draw_brand(ui.painter(), rect);
                    ui.add_space(12.0);
                    let active = self.active_tab();
                    if ui::chip(ui, "About", active == Some(Tab::About)).clicked() {
                        open_tab = Some(Tab::About);
                    }
                    if ui::chip(ui, "Modules", active == Some(Tab::Modules)).clicked() {
                        open_tab = Some(Tab::Modules);
                    }
                    // "Update available" pill, next to Modules.
                    if self.update.is_some() {
                        ui.add_space(6.0);
                        if ui::pill(
                            ui,
                            "Update available",
                            egui::Color32::from_rgb(0xe6, 0x9a, 0x5c),
                        )
                        .on_hover_text("A newer version is available")
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
                            egui::RichText::new(format!("Installing {rt}…"))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui::chip(ui, "🛠", active == Some(Tab::Developer))
                            .on_hover_text("Developer")
                            .clicked()
                        {
                            open_tab = Some(Tab::Developer);
                        }
                        if ui::chip(ui, "⚙", active == Some(Tab::Settings))
                            .on_hover_text("Settings")
                            .clicked()
                        {
                            open_tab = Some(Tab::Settings);
                        }
                    });
                });
            });

        // Tab strip: open tabs with close buttons; plus "frequent" quick-open
        // chips on the right.
        let frequent = self.frequent_modules(4);
        egui::TopBottomPanel::top("tabstrip")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(0x18, 0x1a, 0x1f))
                    .inner_margin(egui::Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
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
                                .unwrap_or_else(|| "Details".into()),
                            _ => tab.title(),
                        };

                        // Zed-style tab: stable width (the close slot is always
                        // reserved), the × only shows on hover or when active.
                        let pad = 10.0;
                        let close_w = 16.0;
                        let gap = 6.0;
                        let galley =
                            ui.painter()
                                .layout_no_wrap(text, font_id.clone(), ui::color::TEXT);
                        let w = pad + galley.size().x + gap + close_w + pad;
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
                        // Use the pointer position, not `resp.hovered()`: the close
                        // button below is drawn on top and would otherwise steal the
                        // hover, making the tab flicker as the × shows/hides.
                        let hovered = ui.rect_contains_pointer(rect);

                        // Smoothly fade the hover fill in, and grow the active
                        // underline out from the tab's centre toward its edges.
                        let hover_t =
                            ui::anim_bool(ui, resp.id.with("hover"), hovered && !selected, 0.14);
                        let active_t = ui::anim_bool(ui, resp.id.with("active"), selected, 0.05);

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
                        let tcol = ui::lerp_color(ui::color::TEXT_MUTED, ui::color::TEXT, text_t);
                        let tpos =
                            egui::pos2(rect.left() + pad, rect.center().y - galley.size().y / 2.0);
                        ui.painter().galley(tpos, galley, tcol);

                        // Close affordance — only when active or hovered.
                        let mut close_clicked = false;
                        if selected || hovered {
                            let cc =
                                egui::pos2(rect.right() - pad - close_w / 2.0, rect.center().y);
                            let crect =
                                egui::Rect::from_center_size(cc, egui::vec2(close_w, close_w));
                            let cresp =
                                ui.interact(crect, resp.id.with("close"), egui::Sense::click());
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
                    // Frequently-visited modules not already open.
                    if !frequent.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for name in frequent.iter().rev() {
                                if ui::chip(ui, &format!("↗ {name}"), false)
                                    .on_hover_text("Frequently used — open")
                                    .clicked()
                                {
                                    open_tab = Some(Tab::Module(name.clone()));
                                }
                            }
                        });
                    }
                });
            });

        // Central content for the active tab (split-borrow to mutate inputs etc).
        let mut action: Option<ui::Invoke> = None;
        let mut open_module: Option<String> = None;
        let mut remove_module: Option<String> = None;
        let mut add_module: Option<String> = None;
        let mut update_module: Option<String> = None;
        let mut toggle_pin: Option<String> = None;
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
        let about_reveal = self.about_revealed_at.unwrap_or(now_t);
        let settings_reveal = self.settings_revealed_at.unwrap_or(now_t);
        let license_reveal = self.license_revealed_at.unwrap_or(now_t);
        {
            let LimenApp {
                modules,
                git_installed,
                git_meta,
                available_updates,
                pinned,
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
                dev_mode_on,
                dev_limen_path,
                dev_modules_path,
                removing,
                modules_revealed_at,
                shown_filter,
                remote_revealed_at,
                developer_revealed_at,
                shown_dev_tab,
                detail_tabs,
                ..
            } = self;
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(err) = fatal {
                    ui.colored_label(egui::Color32::LIGHT_RED, "The engine failed to start:");
                    ui.add_space(4.0);
                    ui.monospace(err.as_str());
                    return;
                }
                match active_tab {
                    None => {
                        ui.add_space(24.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("No open tabs — use the buttons above.").weak(),
                            );
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
                        pinned,
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
                        &mut toggle_pin,
                        &mut reload,
                        modules_revealed_at,
                        shown_filter,
                        *remote_revealed_at,
                        removing,
                    ),
                    Some(Tab::Module(name)) => module_view(
                        ui,
                        &name,
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
        }

        if do_update && let Some(info) = self.update.clone() {
            self.updating = true;
            self.worker.send(Command::ApplyUpdate(info));
        }

        // Apply tab intents.
        if let Some(i) = switch_to {
            match self.tabs.get(i).cloned() {
                Some(Tab::Module(name)) => self.select_module(name), // re-fetch its UI
                _ => self.active = i,
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
        if let Some(name) = remove_module {
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
        if let Some(name) = toggle_pin {
            self.toggle_pin(&name);
        }
        if reload {
            // Re-scan the module directories from disk: pick up newly-added
            // modules and drop any whose folder is gone (not just re-list what's
            // already loaded).
            self.worker.send(Command::Reload);
            self.remote_fetched = false;
        }
        if let Some(a) = action {
            // Elevated methods prompt for consent (once) before running.
            if self.action_needs_consent(&a.action) {
                self.pending_action = Some(a);
            } else {
                self.dispatch(a);
            }
        }

        // Consent dialog for a pending elevated action.
        if let Some(pending) = self.pending_action.clone() {
            let module = self.module_of(&pending.action.capability).cloned();
            let mut decision: Option<bool> = None; // Some(true)=grant, Some(false)=deny
            egui::Window::new("Permission required")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    let name = module
                        .as_ref()
                        .map(|m| m.name.as_str())
                        .unwrap_or("This module");
                    ui.label(
                        egui::RichText::new(format!(
                            "“{name}” wants to run “{}”, which needs elevated permissions.",
                            pending.action.method
                        ))
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
                        if ui::primary_button(ui, "Grant & Run", egui::Vec2::ZERO).clicked() {
                            decision = Some(true);
                        }
                        if ui::outline_button(ui, "Deny", egui::Vec2::ZERO).clicked() {
                            decision = Some(false);
                        }
                        ui.label(
                            egui::RichText::new("Approval is remembered for this version.")
                                .small()
                                .color(ui::color::TEXT_MUTED),
                        );
                    });
                });
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
                    self.status = "permission denied".to_string();
                }
                None => {}
            }
        }

        // Fetch the org's module list the first time we land on the Modules page.
        if self.active_tab() == Some(Tab::Modules) && !self.remote_fetched {
            self.remote_fetched = true;
            self.remote_loading = true;
            self.worker.send(Command::ListRemote);
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
    ui.heading("Update");
    ui.separator();
    ui.add_space(8.0);

    let Some(info) = info else {
        ui.label(egui::RichText::new("Limen is up to date.").color(ui::color::TEXT_MUTED));
        return;
    };

    ui.label(
        egui::RichText::new(format!(
            "A new version is available: v{} -> v{}",
            info.current, info.latest
        ))
        .size(15.0),
    );
    if !info.url.is_empty() {
        ui.add_space(2.0);
        ui.hyperlink_to("Release notes ↗", &info.url);
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
                ui::primary_button(ui, "Update", egui::Vec2::ZERO)
            })
            .inner
            .clicked();
        if clicked {
            *do_update = true;
        }
        if updating {
            ui.add_space(6.0);
            ui.spinner();
            ui.label(egui::RichText::new("downloading & installing… the app will restart").small());
        } else if info.asset_url.is_none() {
            ui.label(
                egui::RichText::new("no prebuilt binary for this platform — see release notes")
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
    });
}

/// The Settings tab. `reveal_at` staggers the sections in when the tab is shown.
fn settings_view(
    ui: &mut egui::Ui,
    scale: &mut f32,
    changed: &mut bool,
    animations: &mut bool,
    anim_changed: &mut bool,
    reveal_at: f64,
) {
    let now = ui.input(|i| i.time);
    let animate = ui::animations_enabled();

    ui.add_space(4.0);
    reveal_item(ui, 0, reveal_at, now, animate, |ui| {
        ui.heading("Settings");
        ui.separator();
    });
    ui.add_space(6.0);
    reveal_item(ui, 1, reveal_at, now, animate, |ui| {
        ui.label(egui::RichText::new("UI scale").strong());
        ui.label(
            egui::RichText::new("Make the whole interface bigger or smaller.")
                .small()
                .color(ui::color::TEXT_MUTED),
        );
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
    reveal_item(ui, 2, reveal_at, now, animate, |ui| {
        ui.separator();
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Animations").strong());
        ui.label(
            egui::RichText::new("Smoothly animate buttons and transitions.")
                .small()
                .color(ui::color::TEXT_MUTED),
        );
        ui.add_space(6.0);
        if ui::toggle(ui, animations, "Enable animations").changed() {
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
    ui.label(egui::RichText::new("Dev mode").strong());
    ui.label(
        egui::RichText::new(
            "Source app and module updates from local directories instead of \
             GitHub — for testing update flows. A relative path resolves against \
             the working directory. Resets when Limen restarts.",
        )
        .small()
        .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(8.0);
    egui::Grid::new("dev_mode_paths")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("Limen update path");
            ui::text_field(
                ui,
                dev_limen_path,
                "dir of Limen-<ver>-<platform>.tar.gz",
                320.0,
            );
            ui.end_row();
            ui.label("Module update path");
            ui::text_field(ui, dev_modules_path, "dir of <name>/ module folders", 320.0);
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if *dev_mode_on {
            "Update dev mode"
        } else {
            "Set dev mode"
        };
        if ui::primary_button(ui, label, egui::Vec2::ZERO).clicked() {
            let as_dir = |s: &str| {
                let t = s.trim();
                (!t.is_empty()).then(|| std::path::PathBuf::from(t))
            };
            limen_core::set_update_dir(as_dir(dev_limen_path));
            limen_registry::set_update_modules_dir(as_dir(dev_modules_path));
            *dev_mode_on = true;
            *applied = true;
        }
        if *dev_mode_on && ui::outline_button(ui, "Turn off", egui::Vec2::ZERO).clicked() {
            limen_core::set_update_dir(None);
            limen_registry::set_update_modules_dir(None);
            *dev_mode_on = false;
            *applied = true;
        }
    });
    if *dev_mode_on {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Dev mode on — updates sourced from the paths above.")
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
        if ui::chip(ui, "Dev mode", *dev_tab == DevTab::DevMode).clicked() {
            *dev_tab = DevTab::DevMode;
        }
        if ui::chip(ui, "UI Kit", *dev_tab == DevTab::UiKit).clicked() {
            *dev_tab = DevTab::UiKit;
        }
        if ui::chip(ui, "Console", *dev_tab == DevTab::Console).clicked() {
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
        ui.label(egui::RichText::new(format!("{} lines", logs.len())).color(ui::color::TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui::toggle(ui, autoscroll, "Autoscroll");
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
                        ui.label(egui::RichText::new("No logs yet.").color(ui::color::TEXT_MUTED));
                    }
                    for line in logs {
                        ui.label(egui::RichText::new(line).monospace().size(12.0));
                    }
                });
        });
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
    pinned: &[String],
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
    toggle_pin: &mut Option<String>,
    reload: &mut bool,
    modules_revealed_at: &mut Option<f64>,
    shown_filter: &mut ModuleFilter,
    remote_revealed_at: Option<f64>,
    removing: &HashMap<String, f64>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Modules");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui::outline_button(ui, "Reload", egui::Vec2::ZERO).clicked() {
                *reload = true;
            }
        });
    });
    ui.add_space(10.0);

    // One universal search across installed (local) and org (remote) modules.
    ui::text_field(
        ui,
        search,
        "Search modules — local and in the org…",
        f32::INFINITY,
    );
    ui.add_space(8.0);

    // Installed / Available filter.
    ui.horizontal(|ui| {
        for (value, label) in [
            (ModuleFilter::All, "All"),
            (ModuleFilter::Installed, "Installed"),
            (ModuleFilter::Available, "Available"),
        ] {
            if ui::chip(ui, label, *filter == value).clicked() {
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

    let query = search.to_lowercase();
    let installed_names: HashSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);
        let mut shown = 0;
        let animate = ui::animations_enabled();

        // Installed modules — pinned first (in pin order), then the rest in their
        // existing order (stable sort keeps non-pinned relative order).
        let mut ordered: Vec<&ModuleSpec> = modules.iter().collect();
        ordered.sort_by_key(|m| {
            pinned
                .iter()
                .position(|n| n == &m.name)
                .unwrap_or(usize::MAX)
        });
        for m in ordered {
            if *filter == ModuleFilter::Available || !module_matches(m, &query) {
                continue;
            }
            let rt = match removing.get(m.name.as_str()) {
                Some(&s) if s < 0.0 => 1.0, // removal sent — stay faded out
                Some(&s) => (((now - s) / 0.34).clamp(0.0, 1.0)) as f32,
                None => 0.0,
            };
            let is_pinned = pinned.iter().any(|n| n == &m.name);
            let id = egui::Id::new(("modcard", m.name.as_str()));
            reveal_card(ui, id, shown, reveal_at, now, animate, rt, |ui| {
                module_card(
                    ui,
                    m,
                    git_installed.contains(&m.name),
                    git_meta.get(&m.name),
                    available_updates.get(&m.name).map(String::as_str),
                    is_pinned,
                    installing,
                    installing_runtime,
                    open,
                    remove,
                    update,
                    toggle_pin,
                );
            });
            shown += 1;
        }

        // Available in the org (not already installed). These load asynchronously,
        // so their entrance is timed from when the list arrived (or the last filter
        // switch, whichever is later) with its own stagger index.
        let avail_reveal = reveal_at.max(remote_revealed_at.unwrap_or(reveal_at));
        let mut avail_i = 0usize;
        for r in remote {
            if *filter == ModuleFilter::Installed
                || installed_names.contains(r.name.as_str())
                || !remote_matches(r, &query)
            {
                continue;
            }
            let id = egui::Id::new(("availcard", r.name.as_str()));
            reveal_card(ui, id, avail_i, avail_reveal, now, animate, 0.0, |ui| {
                available_card(ui, r, installing, add);
            });
            avail_i += 1;
            shown += 1;
        }

        if let Some(err) = remote_error {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("Couldn't list the org: {err}"))
                    .small()
                    .color(ui::color::TEXT_MUTED),
            );
        }
        if shown == 0 && !remote_loading {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("No modules match.").color(ui::color::TEXT_MUTED));
            });
        }
    });
}

fn module_matches(m: &ModuleSpec, query: &str) -> bool {
    query.is_empty()
        || m.name.to_lowercase().contains(query)
        || m.description
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains(query)
        || m.capabilities
            .iter()
            .any(|c| c.to_lowercase().contains(query))
}

fn remote_matches(r: &RemoteModule, query: &str) -> bool {
    query.is_empty()
        || r.name.to_lowercase().contains(query)
        || r.description
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .contains(query)
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
    let enter = ui::reveal_t(ui, k, reveal_at, now, 0.05, 0.30);
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
    pinned: bool,
    installing: &Option<String>,
    installing_runtime: &Option<String>,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    update: &mut Option<String>,
    toggle_pin: &mut Option<String>,
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
                            ui.label(egui::RichText::new(&m.name).size(16.0).strong());
                            ui.label(
                                egui::RichText::new(format!("v{}", m.version))
                                    .monospace()
                                    .color(ui::color::TEXT_MUTED),
                            );
                            for cap in &m.capabilities {
                                badge(ui, cap);
                            }
                        });
                        if let Some(desc) = &m.description {
                            ui.add_space(6.0);
                            ui.label(desc);
                        }
                        // Host-privilege heads-up: some methods may need admin on
                        // this machine. Informational only — no listing, no prompt.
                        if m.permissions.may_require_admin {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(
                                    "Some methods may require elevated permissions.",
                                )
                                .small()
                                .color(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c)),
                            );
                        }
                        if !m.authors.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("by {}", m.authors.join(", ")))
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
                            );
                        }
                        // Git revision this module was installed from — branch on
                        // top, commit below.
                        if let Some((branch, commit)) = git_meta {
                            let mut lines: Vec<String> = Vec::new();
                            if !branch.is_empty() {
                                lines.push(format!("branch - {branch}"));
                            }
                            if !commit.is_empty() {
                                lines.push(format!("commit - {commit}"));
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
                            ui.label(
                                egui::RichText::new(format!(
                                    "Requires: {}",
                                    m.requires.keys().cloned().collect::<Vec<_>>().join(", ")
                                ))
                                .small()
                                .color(ui::color::TEXT_MUTED),
                            );
                        }
                        if !m.optional.is_empty() {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "Optional: {}",
                                    m.optional.keys().cloned().collect::<Vec<_>>().join(", ")
                                ))
                                .small()
                                .color(ui::color::ACCENT),
                            );
                        }
                    },
                );

                // Right column: Open / Remove / GitHub, stacked at the box end.
                ui.allocate_ui_with_layout(
                    egui::vec2(right_w, 0.0),
                    egui::Layout::top_down(egui::Align::Max),
                    |ui| {
                        // add_sized centers-and-justifies, so the label is
                        // centered within each fixed-width button.
                        let bw = egui::vec2(96.0, ui.spacing().interact_size.y);
                        if this_busy {
                            // Mid-update: spinner in place of the action buttons.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Updating…")
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                            return;
                        }
                        // Open is disabled while this module's runtime downloads.
                        let open_clicked = ui
                            .add_enabled_ui(!runtime_busy, |ui| ui::outline_button(ui, "Open", bw))
                            .inner
                            .clicked();
                        if open_clicked {
                            *open = Some(m.name.clone());
                        }
                        if runtime_busy {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    egui::RichText::new("Preparing runtime…")
                                        .small()
                                        .color(ui::color::TEXT_MUTED),
                                );
                            });
                        }
                        if from_git && latest.is_some() {
                            // Update only shows when a newer release exists.
                            let label = match latest {
                                Some(v) => format!("Update {v}"),
                                None => "Update".into(),
                            };
                            let clicked = ui
                                .add_enabled_ui(!any_busy, |ui| ui::primary_button(ui, &label, bw))
                                .inner
                                .clicked();
                            if clicked {
                                *update = Some(m.name.clone());
                            }
                        }
                        let pin_label = if pinned { "📌 Unpin" } else { "📌 Pin" };
                        if ui::outline_button(ui, pin_label, bw).clicked() {
                            *toggle_pin = Some(m.name.clone());
                        }
                        if ui::outline_button(ui, "Remove", bw).clicked() {
                            *remove = Some(m.name.clone());
                        }
                        // GitHub only for git-installed modules.
                        if from_git
                            && let Some(repo) = &m.repo
                            && ui::outline_button(ui, "GitHub ↗", bw).clicked()
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
                            ui.label(egui::RichText::new(&r.name).size(16.0).strong());
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
                        let bw = egui::vec2(96.0, ui.spacing().interact_size.y);
                        if this_installing {
                            // This module is downloading — spinner + label.
                            ui.allocate_ui_with_layout(
                                bw,
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Installing…")
                                            .small()
                                            .color(ui::color::TEXT_MUTED),
                                    );
                                },
                            );
                        } else {
                            // Disable while another install is in flight.
                            let clicked = ui
                                .add_enabled_ui(!any_installing, |ui| {
                                    ui::primary_button(ui, "Install", bw)
                                })
                                .inner
                                .clicked();
                            if clicked {
                                *add = Some(r.repo.clone());
                            }
                        }
                        if ui::outline_button(ui, "GitHub ↗", bw).clicked() {
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

/// A small rounded pill, like Zed's category tags.
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

/// The Limen tagline.
const TAGLINE: &str = "A modular tool for security and analysis.";

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
    let t = ui::reveal_t(ui, k, reveal_at, now, 0.035, 0.18);
    ui.scope(|ui| {
        ui.set_opacity(t);
        draw(ui);
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
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(92.0, 92.0), egui::Sense::hover());
                    draw_brand(ui.painter(), rect);
                });

                ui.add_space(14.0);
                reveal_item(ui, 1, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new("LIMEN").size(30.0).strong());
                });
                reveal_item(ui, 2, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "v{} · {} branch",
                            env!("CARGO_PKG_VERSION"),
                            env!("LIMEN_GIT_BRANCH"),
                        ))
                        .monospace()
                        .color(muted),
                    );
                    ui.label(
                        egui::RichText::new(format!("commit {}", env!("LIMEN_GIT_COMMIT")))
                            .monospace()
                            .small()
                            .color(muted),
                    );
                });

                ui.add_space(18.0);
                reveal_item(ui, 3, reveal_at, now, animate, |ui| {
                    ui.label(egui::RichText::new(TAGLINE).size(15.0));
                });

                ui.add_space(18.0);
                reveal_item(ui, 4, reveal_at, now, animate, |ui| {
                    ui.separator();
                    ui.add_space(14.0);
                    if ui::outline_button(ui, "View license", egui::Vec2::ZERO).clicked() {
                        license_clicked = true;
                    }
                });

                ui.add_space(16.0);
                reveal_item(ui, 5, reveal_at, now, animate, |ui| {
                    ui.label(
                        egui::RichText::new("Powered by CRC BARRACUDA")
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
            ui.heading("License");
            ui.label(
                egui::RichText::new(
                    "Limen is free software: you can redistribute it and/or modify it under \
                     the terms of the GNU General Public License, version 3 or later.",
                )
                .color(ui::color::TEXT_MUTED),
            );
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

/// Draw the Limen brand mark — the same device as `resources/icon.png`: a
/// cyan→blue diamond ring with a solid core, on a dark rounded tile.
///
/// Colors and proportions are sampled from that file (on its 256px grid) so the
/// in-app mark matches the icon Explorer, the taskbar, and the window frame show.
fn draw_brand(painter: &egui::Painter, rect: egui::Rect) {
    use egui::{Color32, Mesh, Pos2, Shape};

    // Tile: a vertical gradient, lighter at the top.
    let tile_top = Color32::from_rgb(0x29, 0x2f, 0x3e);
    let tile_bottom = Color32::from_rgb(0x14, 0x17, 0x1d);
    // Diamond: one diagonal gradient across the whole device — light at the
    // top-left, dark at the bottom-right — so the top *and left* vertices are
    // light while the right and bottom ones are dark.
    let light = Color32::from_rgb(0x6a, 0xd0, 0xeb);
    let dark = Color32::from_rgb(0x49, 0xb0, 0xe1);

    let s = rect.width().min(rect.height()) / 256.0;

    // The tile is inset within the icon's box and has rounded corners, so it
    // needs a colored mesh rather than `rect_filled` to carry the gradient.
    let tile = egui::Rect::from_center_size(rect.center(), egui::vec2(224.0 * s, 224.0 * s));
    painter.add(Shape::mesh(rounded_rect_mesh(
        tile,
        44.0 * s,
        tile_top,
        tile_bottom,
    )));

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
    let shade = |p: Pos2| lerp_color(light, dark, ((p.x - c.x) + (p.y - c.y)) / span + 0.5);

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
        egui::Stroke::new(1.4 * s, Color32::from_rgb(0xb4, 0xf6, 0xfc)),
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
        ui.label("This detail view is no longer available.");
        return;
    };
    let heading = if tab.title.is_empty() {
        "Details"
    } else {
        tab.title.as_str()
    };
    if let Some(err) = &tab.error {
        ui.heading(heading);
        ui.separator();
        ui.colored_label(egui::Color32::LIGHT_RED, err);
        return;
    }
    match &tab.view {
        Some(v) => {
            let heading = if v.title.is_empty() {
                heading
            } else {
                v.title.as_str()
            };
            ui.heading(heading);
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(a) = ui::render_view(ui, v, &mut tab.inputs, None) {
                        *action = Some(a);
                    }
                });
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading…");
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn module_view(
    ui: &mut egui::Ui,
    name: &str,
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
                    if let Some(a) = ui::render_view(ui, v, inputs, busy_action) {
                        *action = Some(a);
                    }

                    // Show the Result pane only when a method returned output.
                    if !output.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("Result").strong());
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
                    ui.label("loading…");
                });
            }
        }
    }
}
