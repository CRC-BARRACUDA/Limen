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
use limen_core::ModuleSpec;
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
        }
    }
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
    pending_action: Option<ui::Action>,

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
        let animations = limen_core::Config::load().map(|c| c.animations).unwrap_or(true);
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
            pinned: limen_core::Config::load().map(|c| c.pinned_modules).unwrap_or_default(),
            ui_scale: {
                let pct = limen_core::Config::load().map(|c| c.ui_scale_percent).unwrap_or(0);
                if pct == 0 { 100.0 } else { pct as f32 }
            },
            animations,
            about_revealed_at: None,
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
        v.into_iter().take(n).map(|(name, _)| name.clone()).collect()
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
        let trust = limen_registry::TrustStore::load(&limen_core::paths::home())
            .unwrap_or_default();
        self.trusted.clear();
        for m in &self.modules {
            if !m.permissions.sensitive() {
                continue;
            }
            if let Ok(digest) = limen_registry::digest_dir(&m.cwd)
                && trust.is_trusted(&m.name, &digest) {
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
        let mut trust = limen_registry::TrustStore::load(&limen_core::paths::home())
            .unwrap_or_default();
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
                            Ok(v) => {
                                self.output =
                                    serde_json::to_string_pretty(&v).unwrap_or_else(|e| e.to_string())
                            }
                            Err(e) => self.output = format!("error: {e}"),
                        }
                        self.status = "done".to_string();
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

    fn dispatch(&mut self, action: ui::Action) {
        let params = self
            .view
            .as_ref()
            .map(|v| ui::collect_params(v, &self.inputs))
            .unwrap_or_else(|| serde_json::json!({}));
        self.busy = true;
        self.busy_action = Some(action.clone());
        self.output.clear(); // the button spinner shows progress, not the Result pane
        self.status = format!("{}.{}", action.capability, action.method);
        self.worker.send(Command::Run {
            tag: RunTag::Action,
            capability: action.capability,
            method: action.method,
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
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                    draw_brand(ui.painter(), rect);
                    ui.add_space(12.0);
                    if ui.button("About").clicked() {
                        open_tab = Some(Tab::About);
                    }
                    if ui.button("Modules").clicked() {
                        open_tab = Some(Tab::Modules);
                    }
                    // "Update available" pill, next to Modules.
                    if self.update.is_some() {
                        ui.add_space(6.0);
                        let pill = egui::Button::new(
                            egui::RichText::new("Update available").small().color(ui::color::ON_ACCENT),
                        )
                        .fill(egui::Color32::from_rgb(0xe6, 0x9a, 0x5c))
                        .rounding(egui::Rounding::same(10.0_f32));
                        if ui.add(pill).on_hover_text("A newer version is available").clicked() {
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
                        if ui.button("🛠").on_hover_text("Developer").clicked() {
                            open_tab = Some(Tab::Developer);
                        }
                        if ui.button("⚙").on_hover_text("Settings").clicked() {
                            open_tab = Some(Tab::Settings);
                        }
                    });
                });
            });

        // Tab strip: open tabs with close buttons; plus "frequent" quick-open
        // chips on the right.
        let frequent = self.frequent_modules(4);
        egui::TopBottomPanel::top("tabstrip")
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(0x18, 0x1a, 0x1f)).inner_margin(egui::Margin::symmetric(8.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let font_id = egui::TextStyle::Button.resolve(ui.style());
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let selected = i == self.active;
                        let text = tab.title();

                        // Zed-style tab: stable width (the close slot is always
                        // reserved), the × only shows on hover or when active.
                        let pad = 10.0;
                        let close_w = 16.0;
                        let gap = 6.0;
                        let galley =
                            ui.painter().layout_no_wrap(text, font_id.clone(), ui::color::TEXT);
                        let w = pad + galley.size().x + gap + close_w + pad;
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
                        // Use the pointer position, not `resp.hovered()`: the close
                        // button below is drawn on top and would otherwise steal the
                        // hover, making the tab flicker as the × shows/hides.
                        let hovered = ui.rect_contains_pointer(rect);

                        // Smoothly fade the hover fill in, and grow the active
                        // underline out from the tab's centre toward its edges.
                        let anim = ui::animations_enabled();
                        let hover_t = if anim {
                            ui.ctx().animate_bool_with_time(
                                resp.id.with("hover"),
                                hovered && !selected,
                                0.14,
                            )
                        } else {
                            (hovered && !selected) as u8 as f32
                        };
                        let active_t = if anim {
                            ui.ctx().animate_bool_with_time(resp.id.with("active"), selected, 0.05)
                        } else {
                            selected as u8 as f32
                        };

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
                            egui::Rounding { nw: 5.0, ne: 5.0, sw: 0.0, se: 0.0 },
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
                            let cc = egui::pos2(rect.right() - pad - close_w / 2.0, rect.center().y);
                            let crect = egui::Rect::from_center_size(
                                cc,
                                egui::vec2(close_w, close_w),
                            );
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
                                if cresp.hovered() { ui::color::TEXT } else { ui::color::TEXT_MUTED },
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
                                if ui
                                    .add(egui::Button::new(egui::RichText::new(format!("↗ {name}")).small()).frame(false))
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
        let mut action: Option<ui::Action> = None;
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
        let about_reveal = self.about_revealed_at.unwrap_or(now_t);
        {
            let LimenApp {
                modules, git_installed, git_meta, available_updates, pinned, view,
                view_error, inputs, output, busy_action, fatal, search, filter, remote,
                remote_error, remote_loading, installing, dev_tab, logs, log_autoscroll, ui_scale,
                animations, dev_mode_on, dev_limen_path, dev_modules_path, removing,
                modules_revealed_at, shown_filter, remote_revealed_at,
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
                            ui.label(egui::RichText::new("No open tabs — use the buttons above.").weak());
                        });
                    }
                    Some(Tab::About) => {
                        if about_view(ui, about_reveal) {
                            open_tab = Some(Tab::License);
                        }
                    }
                    Some(Tab::License) => license_view(ui),
                    Some(Tab::Modules) => modules_page(
                        ui, modules, git_installed, git_meta, available_updates,
                        pinned, remote, *remote_loading, remote_error, installing, filter, search,
                        &mut open_module, &mut remove_module, &mut add_module, &mut update_module,
                        &mut toggle_pin, &mut reload, modules_revealed_at, shown_filter,
                        *remote_revealed_at, removing,
                    ),
                    Some(Tab::Module(name)) => {
                        module_view(ui, &name, view, view_error, inputs, output, busy_action.as_ref(), &mut action)
                    }
                    Some(Tab::Settings) => {
                        settings_view(ui, ui_scale, &mut scale_changed, animations, &mut anim_changed)
                    }
                    Some(Tab::Developer) => developer_view(
                        ui, dev_tab, inputs, logs, log_autoscroll,
                        dev_mode_on, dev_limen_path, dev_modules_path, &mut dev_applied,
                    ),
                    Some(Tab::Update) => {
                        update_view(ui, update_info.as_ref(), updating, &mut do_update)
                    }
                }
            });
        }

        if do_update
            && let Some(info) = self.update.clone() {
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
            if let Some(i) = self.tabs.iter().position(|t| *t == Tab::Module(name.clone())) {
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
            if self.action_needs_consent(&a) {
                self.pending_action = Some(a);
            } else {
                self.dispatch(a);
            }
        }

        // Consent dialog for a pending elevated action.
        if let Some(pending) = self.pending_action.clone() {
            let module = self.module_of(&pending.capability).cloned();
            let mut decision: Option<bool> = None; // Some(true)=grant, Some(false)=deny
            egui::Window::new("Permission required")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    let name = module.as_ref().map(|m| m.name.as_str()).unwrap_or("This module");
                    ui.label(
                        egui::RichText::new(format!(
                            "“{name}” wants to run “{}”, which needs elevated permissions.",
                            pending.method
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
                        if ui::primary_button(ui, "Grant & Run").clicked() {
                            decision = Some(true);
                        }
                        if ui.button("Deny").clicked() {
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
        egui::RichText::new(format!("A new version is available: v{} -> v{}", info.current, info.latest))
            .size(15.0),
    );
    if !info.url.is_empty() {
        ui.add_space(2.0);
        ui.hyperlink_to("Release notes ↗", &info.url);
    }
    if !info.notes.trim().is_empty() {
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            ui::markdown(ui, info.notes.trim());
        });
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let btn = egui::Button::new(egui::RichText::new("Update").color(ui::color::ON_ACCENT))
            .fill(ui::color::ACCENT);
        if ui.add_enabled(!updating, btn).clicked() {
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

/// The Settings tab.
fn settings_view(
    ui: &mut egui::Ui,
    scale: &mut f32,
    changed: &mut bool,
    animations: &mut bool,
    anim_changed: &mut bool,
) {
    ui.add_space(4.0);
    ui.heading("Settings");
    ui.separator();
    ui.add_space(6.0);
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
            if ui.selectable_label(selected, format!("{}%", pct as u32)).clicked() {
                *scale = pct;
                *changed = true;
            }
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Animations").strong());
    ui.label(
        egui::RichText::new("Smoothly animate buttons and transitions.")
            .small()
            .color(ui::color::TEXT_MUTED),
    );
    ui.add_space(6.0);
    if ui.checkbox(animations, "Enable animations").changed() {
        *anim_changed = true;
    }
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
            ui.add(
                egui::TextEdit::singleline(dev_limen_path)
                    .hint_text("dir of Limen-<ver>-<platform>.tar.gz")
                    .desired_width(320.0),
            );
            ui.end_row();
            ui.label("Module update path");
            ui.add(
                egui::TextEdit::singleline(dev_modules_path)
                    .hint_text("dir of <name>/ module folders")
                    .desired_width(320.0),
            );
            ui.end_row();
        });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if *dev_mode_on { "Update dev mode" } else { "Set dev mode" };
        if ui::primary_button(ui, label).clicked() {
            let as_dir = |s: &str| {
                let t = s.trim();
                (!t.is_empty()).then(|| std::path::PathBuf::from(t))
            };
            limen_core::set_update_dir(as_dir(dev_limen_path));
            limen_registry::set_update_modules_dir(as_dir(dev_modules_path));
            *dev_mode_on = true;
            *applied = true;
        }
        if *dev_mode_on && ui.button("Turn off").clicked() {
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

/// The Developer tab: sub-tabs for docs / UI kit / log console.
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
) {
    ui.horizontal(|ui| {
        ui.selectable_value(dev_tab, DevTab::DevMode, "Dev mode");
        ui.selectable_value(dev_tab, DevTab::UiKit, "UI Kit");
        ui.selectable_value(dev_tab, DevTab::Console, "Console");
    });
    ui.separator();
    match dev_tab {
        DevTab::DevMode => {
            dev_mode_view(ui, dev_mode_on, dev_limen_path, dev_modules_path, dev_applied)
        }
        DevTab::UiKit => ui::render_demo_ui(ui, inputs),
        DevTab::Console => dev_console(ui, logs, autoscroll),
    }
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
            ui.checkbox(autoscroll, "Autoscroll");
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
            if ui.button("Reload").clicked() {
                *reload = true;
            }
        });
    });
    ui.add_space(10.0);

    // One universal search across installed (local) and org (remote) modules.
    ui.add(
        egui::TextEdit::singleline(search)
            .hint_text("Search modules — local and in the org…")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);

    // Installed / Available filter.
    ui.horizontal(|ui| {
        for (value, label) in [
            (ModuleFilter::All, "All"),
            (ModuleFilter::Installed, "Installed"),
            (ModuleFilter::Available, "Available"),
        ] {
            if ui::filter_chip(ui, label, *filter == value).clicked() {
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
        ordered.sort_by_key(|m| pinned.iter().position(|n| n == &m.name).unwrap_or(usize::MAX));
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
                    ui, m, git_installed.contains(&m.name), git_meta.get(&m.name),
                    available_updates.get(&m.name).map(String::as_str),
                    is_pinned, installing, open, remove, update,
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
        || m.description.as_deref().unwrap_or("").to_lowercase().contains(query)
        || m.capabilities.iter().any(|c| c.to_lowercase().contains(query))
}

fn remote_matches(r: &RemoteModule, query: &str) -> bool {
    query.is_empty()
        || r.name.to_lowercase().contains(query)
        || r.description.as_deref().unwrap_or("").to_lowercase().contains(query)
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
    // Time-based entrance, eased and staggered by index. Replays whenever the
    // reveal timer resets (tab shown, or the filter changed).
    let start = reveal_at + k as f64 * 0.05;
    let raw = (((now - start) / 0.30).clamp(0.0, 1.0)) as f32;
    let inv = 1.0 - raw;
    let enter = 1.0 - inv * inv * inv; // ease-out cubic
    // Exit uses smoothstep (ease in *and* out) so the fade, slide, and — most
    // importantly — the height collapse all progress steadily; a cubic ease-in
    // held the height near full and then snapped it shut at the end.
    let exit = remove_t * remove_t * (3.0 - 2.0 * remove_t); // smoothstep

    let dx = (1.0 - enter) * 28.0 + exit * 28.0;
    if enter < 1.0 || (remove_t > 0.0 && remove_t < 1.0) {
        ui.ctx().request_repaint(); // keep animating until settled
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
    open: &mut Option<String>,
    remove: &mut Option<String>,
    update: &mut Option<String>,
    toggle_pin: &mut Option<String>,
) {
    // This card is mid-update; another install/update is running somewhere.
    let this_busy = installing.as_deref() == Some(m.name.as_str());
    let any_busy = installing.is_some();
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
                                egui::RichText::new("Some methods may require elevated permissions.")
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
                        if ui::outline_button(ui, "Open", bw).clicked() {
                            *open = Some(m.name.clone());
                        }
                        if from_git && latest.is_some() {
                            // Update only shows when a newer release exists.
                            let label = match latest {
                                Some(v) => format!("Update {v}"),
                                None => "Update".into(),
                            };
                            let btn = egui::Button::new(
                                egui::RichText::new(label).color(ui::color::ON_ACCENT),
                            )
                            .fill(ui::color::ACCENT);
                            let clicked = ui
                                .add_enabled_ui(!any_busy, |ui| ui.add_sized(bw, btn))
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
                                && ui::outline_button(ui, "GitHub ↗", bw).clicked() {
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
                        ui.label(egui::RichText::new(&r.repo).small().color(ui::color::TEXT_MUTED));
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
                            let install = egui::Button::new(
                                egui::RichText::new("Install").color(ui::color::ON_ACCENT),
                            )
                            .fill(ui::color::ACCENT);
                            // Disable while another install is in flight.
                            let clicked = ui
                                .add_enabled_ui(!any_installing, |ui| ui.add_sized(bw, install))
                                .inner
                                .clicked();
                            if clicked {
                                *add = Some(r.repo.clone());
                            }
                        }
                        if ui.add_sized(bw, egui::Button::new("GitHub ↗")).clicked() {
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
            ui.label(egui::RichText::new(text).size(11.0).color(ui::color::TEXT_MUTED));
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
    let start = reveal_at + k as f64 * 0.035;
    let raw = (((now - start) / 0.18).clamp(0.0, 1.0)) as f32;
    let inv = 1.0 - raw;
    let t = 1.0 - inv * inv * inv; // ease-out cubic
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
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
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
                if ui.button("View license").clicked() {
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
fn license_view(ui: &mut egui::Ui) {
    // Center the license in a fixed-width column.
    ui.vertical_centered(|ui| {
        ui.set_max_width(720.0);
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
}

/// Draw the Limen brand mark — a concentric indigo diamond (◈) on a dark rounded
/// tile — reproducing `packaging/make_icon.py` from the old Limen.
fn draw_brand(painter: &egui::Painter, rect: egui::Rect) {
    use egui::{Color32, Mesh, Pos2, Rounding, Shape, Stroke};

    let bg = Color32::from_rgb(0x0f, 0x14, 0x20);
    let border = Color32::from_rgb(0x2a, 0x35, 0x50);
    let top = Color32::from_rgb(0x7c, 0x6c, 0xff);
    let bottom = Color32::from_rgb(0x63, 0x54, 0xeb);
    let mid = Color32::from_rgb(0x6f, 0x60, 0xf5);
    let center = Color32::from_rgb(0x96, 0x8a, 0xff);

    let scale = rect.width().min(rect.height()) / 256.0;
    painter.rect_filled(rect, Rounding::same(56.0 * scale), bg);
    painter.rect_stroke(
        rect.shrink(3.0 * scale),
        Rounding::same(53.0 * scale),
        Stroke::new(3.0 * scale, border),
    );

    let c = rect.center();
    let diamond = |half: f32| {
        vec![
            Pos2::new(c.x, c.y - half),
            Pos2::new(c.x + half, c.y),
            Pos2::new(c.x, c.y + half),
            Pos2::new(c.x - half, c.y),
        ]
    };

    // Outer diamond with a vertical indigo gradient (via a colored mesh).
    let half = 94.0 * scale;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(Pos2::new(c.x, c.y - half), top);
    mesh.colored_vertex(Pos2::new(c.x + half, c.y), mid);
    mesh.colored_vertex(Pos2::new(c.x, c.y + half), bottom);
    mesh.colored_vertex(Pos2::new(c.x - half, c.y), mid);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));

    // Punch out the ring, then the solid core.
    painter.add(Shape::convex_polygon(diamond(56.0 * scale), bg, Stroke::NONE));
    painter.add(Shape::convex_polygon(diamond(26.0 * scale), center, Stroke::NONE));
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
    action: &mut Option<ui::Action>,
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
