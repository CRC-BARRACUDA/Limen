//! The Limen desktop shell (egui/eframe).
//!
//! The shell is deliberately thin: a sidebar of modules, and a central panel
//! that renders whatever UI the selected module describes for itself (via the
//! GUI core in [`crate::ui`]). There are no CrowdStrike-shaped built-in views —
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
    Modules,
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

    // Developer tab
    dev_tab: DevTab,
    logs: std::collections::VecDeque<String>,
    log_autoscroll: bool,

    /// Module names pinned to the tab bar, in order (persisted in settings).
    pinned: Vec<String>,

    /// Global UI scale as a percentage (persisted in settings).
    ui_scale: f32,
}

impl LimenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Vec<std::path::PathBuf>) -> Self {
        ui::apply_theme(&cc.egui_ctx);
        Self {
            worker: Worker::spawn(dirs),
            status: "starting modules…".to_string(),
            fatal: None,
            modules: Vec::new(),
            git_installed: HashSet::new(),
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
            dev_tab: DevTab::Modules,
            logs: std::collections::VecDeque::new(),
            log_autoscroll: true,
            pinned: limen_core::Config::load().map(|c| c.pinned_modules).unwrap_or_default(),
            ui_scale: {
                let pct = limen_core::Config::load().map(|c| c.ui_scale_percent).unwrap_or(0);
                if pct == 0 { 100.0 } else { pct as f32 }
            },
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

    /// Close the tab at `index`. Closing a pinned module also unpins it.
    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if let Tab::Module(name) = &self.tabs[index] {
            if self.pinned.iter().any(|n| n == name) {
                let name = name.clone();
                self.toggle_pin(&name); // unpin
            }
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

    /// Ensure every pinned+installed module has an open tab (pinned tabs persist).
    fn sync_pinned_tabs(&mut self) {
        for name in self.pinned.clone() {
            if self.modules.iter().any(|m| m.name == name)
                && !self.tabs.iter().any(|t| *t == Tab::Module(name.clone()))
            {
                self.tabs.push(Tab::Module(name));
            }
        }
    }

    /// Persist the current UI scale to settings.json (without clobbering others).
    fn save_ui_scale(&self) {
        if let Ok(mut cfg) = limen_core::Config::load() {
            cfg.ui_scale_percent = self.ui_scale.round() as u32;
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
            if let Ok(digest) = limen_registry::digest_dir(&m.cwd) {
                if trust.is_trusted(&m.name, &digest) {
                    self.trusted.insert(m.name.clone());
                }
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

    fn drain_events(&mut self) {
        while let Ok(evt) = self.worker.rx.try_recv() {
            match evt {
                Event::Ready(snap) | Event::Modules(snap) => {
                    self.modules = snap.specs;
                    self.git_installed = snap.git_installed.into_iter().collect();
                    self.refresh_trust();
                    self.status = format!("{} module(s) loaded", self.modules.len());
                }
                Event::RemoteModules(result) => {
                    self.remote_loading = false;
                    match result {
                        Ok(list) => {
                            self.remote = list;
                            self.remote_error = None;
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
                    self.refresh_trust();
                    self.push_log(format!("[status] {msg}"));
                    self.status = msg;
                }
                Event::Log(line) => self.push_log(line),
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
        self.drain_events();

        // Apply the global UI scale (set_zoom_factor no-ops if unchanged).
        ctx.set_zoom_factor(self.ui_scale / 100.0);

        // Reload is requested from the Modules page (set during the central panel).
        let mut reload = false;

        // Keep pinned modules present as tabs.
        self.sync_pinned_tabs();

        // Intents collected while rendering, applied after.
        let mut open_tab: Option<Tab> = None;
        let mut switch_to: Option<usize> = None;
        let mut close_idx: Option<usize> = None;
        let mut scale_changed = false;

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

        // Tab strip: open tabs (pinned modules marked), with close buttons; plus
        // "frequent" quick-open chips on the right.
        let pinned = self.pinned.clone();
        let frequent = self.frequent_modules(4);
        egui::TopBottomPanel::top("tabstrip")
            .frame(egui::Frame::none().fill(egui::Color32::from_rgb(0x18, 0x1a, 0x1f)).inner_margin(egui::Margin::symmetric(8.0, 4.0)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let selected = i == self.active;
                        let pinned = matches!(tab, Tab::Module(n) if pinned.iter().any(|p| p == n));
                        let title = if pinned {
                            format!("📌 {}", tab.title())
                        } else {
                            tab.title()
                        };
                        if ui.selectable_label(selected, title).clicked() {
                            switch_to = Some(i);
                        }
                        if ui
                            .add(egui::Button::new(egui::RichText::new("×").weak()).frame(false))
                            .on_hover_text("Close")
                            .clicked()
                        {
                            close_idx = Some(i);
                        }
                        ui.add_space(6.0);
                    }
                    // Frequently-visited modules not already open.
                    if !frequent.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
        let mut toggle_pin: Option<String> = None;
        let active_tab = self.active_tab();
        {
            let LimenApp {
                modules, git_installed, pinned, view, view_error, inputs, output, busy_action,
                fatal, search, filter, remote, remote_error, remote_loading, dev_tab, logs,
                log_autoscroll, ui_scale, ..
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
                        if about_view(ui) {
                            open_tab = Some(Tab::License);
                        }
                    }
                    Some(Tab::License) => license_view(ui),
                    Some(Tab::Modules) => modules_page(
                        ui, modules, git_installed, pinned, remote, *remote_loading, remote_error,
                        filter, search, &mut open_module, &mut remove_module, &mut add_module,
                        &mut toggle_pin, &mut reload,
                    ),
                    Some(Tab::Module(name)) => {
                        module_view(ui, &name, view, view_error, inputs, output, busy_action.as_ref(), &mut action)
                    }
                    Some(Tab::Settings) => settings_view(ui, ui_scale, &mut scale_changed),
                    Some(Tab::Developer) => {
                        developer_view(ui, dev_tab, inputs, logs, log_autoscroll)
                    }
                }
            });
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

        if let Some(name) = open_module {
            self.select_module(name);
        }
        if let Some(name) = remove_module {
            self.busy = true;
            self.status = format!("removing {name}…");
            self.worker.send(Command::RemoveModule(name));
        }
        if let Some(reference) = add_module {
            self.busy = true;
            self.status = format!("installing {reference}…");
            self.worker.send(Command::AddModule(reference));
        }
        if let Some(name) = toggle_pin {
            self.toggle_pin(&name);
        }
        if reload {
            self.worker.send(Command::Refresh);
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
                        let grant = egui::Button::new(
                            egui::RichText::new("Grant & Run").color(ui::color::ON_ACCENT),
                        )
                        .fill(ui::color::ACCENT);
                        if ui.add(grant).clicked() {
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

/// The Settings tab.
fn settings_view(ui: &mut egui::Ui, scale: &mut f32, changed: &mut bool) {
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
}

/// The Developer tab: sub-tabs for docs / UI kit / log console.
fn developer_view(
    ui: &mut egui::Ui,
    dev_tab: &mut DevTab,
    inputs: &mut HashMap<String, String>,
    logs: &std::collections::VecDeque<String>,
    autoscroll: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(dev_tab, DevTab::Modules, "Modules");
        ui.selectable_value(dev_tab, DevTab::UiKit, "UI Kit");
        ui.selectable_value(dev_tab, DevTab::Console, "Console");
    });
    ui.separator();
    match dev_tab {
        DevTab::Modules => dev_modules_docs(ui),
        DevTab::UiKit => ui::render_demo_ui(ui, inputs),
        DevTab::Console => dev_console(ui, logs, autoscroll),
    }
}

/// The Developer window's "Modules" tab — concise module-authoring docs.
fn dev_modules_docs(ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let h = |ui: &mut egui::Ui, t: &str| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t).strong().size(15.0));
        };
        let p = |ui: &mut egui::Ui, t: &str| {
            ui.label(egui::RichText::new(t).color(ui::color::TEXT_MUTED));
        };

        ui.heading("Writing a module");
        p(ui, "A module is an independent package loaded from ~/.limen/modules. \
                It speaks JSON-RPC and addresses other modules by capability.");

        h(ui, "Manifest — limen.toml");
        ui.monospace(
            "[module]\n\
             name = \"usb\"\n\
             version = \"0.1.0\"\n\
             language = \"python\"   # python | lua | js | native\n\
             entry = \"main.py\"\n\
             description = \"…\"\n\
             \n\
             [provides]\n\
             capabilities = [\"ops.usb\"]\n\
             \n\
             [requires.capabilities]\n\
             \"crowdstrike.rtr\" = \">=0.1\"",
        );

        h(ui, "Python SDK (host-injected — just import it)");
        ui.monospace(
            "from limen_sdk import Module, Window, Label, Text, Button\n\
             m = Module(\"usb\", capabilities=[\"ops.usb\"])\n\
             \n\
             @m.method(\"list\")\n\
             def list_(params, host):\n\
             \x20   out = host.call(\"crowdstrike.rtr\", \"runscript\", {...})\n\
             \x20   return {\"devices\": out}\n\
             \n\
             @m.on(\"demo.tick\")          # event callback\n\
             def on_tick(payload, host): ...\n\
             \n\
             @m.ui\n\
             def ui():\n\
             \x20   return Window(\"USB\", [Button(\"List\", calls=\"list\", primary=True)])\n\
             \n\
             m.run()",
        );

        h(ui, "Talking to other modules");
        p(ui, "host.call(capability, method, params) — synchronous, routed by the broker.\n\
                host.emit(topic, payload) — fire an event to subscribers.\n\
                @m.on(topic) — receive events (callback).\n\
                host.log(msg) — appears in the Console tab.");

        h(ui, "Publish");
        p(ui, "Repo CRC-BARRACUDA/limen-<name>, topic 'limen-module'. \
                Install with `limen add <name>`.");
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
    pinned: &[String],
    remote: &[RemoteModule],
    remote_loading: bool,
    remote_error: &Option<String>,
    filter: &mut ModuleFilter,
    search: &mut String,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    add: &mut Option<String>,
    toggle_pin: &mut Option<String>,
    reload: &mut bool,
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
            let text = if *filter == value {
                egui::RichText::new(label).color(ui::color::ACCENT)
            } else {
                egui::RichText::new(label)
            };
            if ui.selectable_label(*filter == value, text).clicked() {
                *filter = value;
            }
        }
        if remote_loading {
            ui.add_space(8.0);
            ui.spinner();
        }
    });
    ui.add_space(6.0);
    ui.separator();

    let query = search.to_lowercase();
    let installed_names: HashSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);
        let mut shown = 0;

        // Installed modules.
        if *filter != ModuleFilter::Available {
            for m in modules {
                if !module_matches(m, &query) {
                    continue;
                }
                let is_pinned = pinned.iter().any(|n| n == &m.name);
                module_card(
                    ui, m, git_installed.contains(&m.name), is_pinned, open, remove, toggle_pin,
                );
                ui.add_space(10.0);
                shown += 1;
            }
        }

        // Available in the org (not already installed).
        if *filter != ModuleFilter::Installed {
            for r in remote {
                if installed_names.contains(r.name.as_str()) || !remote_matches(r, &query) {
                    continue;
                }
                available_card(ui, r, add);
                ui.add_space(10.0);
                shown += 1;
            }
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
fn module_card(
    ui: &mut egui::Ui,
    m: &ModuleSpec,
    from_git: bool,
    pinned: bool,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    toggle_pin: &mut Option<String>,
) {
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
                        if !m.authors.is_empty() {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!("by {}", m.authors.join(", ")))
                                    .small()
                                    .color(ui::color::TEXT_MUTED),
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
                        if ui.add_sized(bw, egui::Button::new("Open")).clicked() {
                            *open = Some(m.name.clone());
                        }
                        let pin_label = if pinned { "📌 Unpin" } else { "📌 Pin" };
                        if ui.add_sized(bw, egui::Button::new(pin_label)).clicked() {
                            *toggle_pin = Some(m.name.clone());
                        }
                        if ui.add_sized(bw, egui::Button::new("Remove")).clicked() {
                            *remove = Some(m.name.clone());
                        }
                        // GitHub only for git-installed modules.
                        if from_git {
                            if let Some(repo) = &m.repo {
                                if ui.add_sized(bw, egui::Button::new("GitHub ↗")).clicked() {
                                    ui.output_mut(|o| {
                                        o.open_url = Some(egui::OpenUrl::new_tab(repo_url(repo)));
                                    });
                                }
                            }
                        }
                    },
                );
            });
        });
}

/// An "available in the org, not installed" card, with an Install action.
fn available_card(ui: &mut egui::Ui, r: &RemoteModule, add: &mut Option<String>) {
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
                            badge(ui, "not installed");
                        });
                        if let Some(desc) = &r.description {
                            ui.add_space(6.0);
                            ui.label(desc);
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
                        let install = egui::Button::new(
                            egui::RichText::new("Install").color(ui::color::ON_ACCENT),
                        )
                        .fill(ui::color::ACCENT);
                        if ui.add_sized(bw, install).clicked() {
                            *add = Some(r.repo.clone());
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

/// The old Limen tagline + disclaimer (see `gui/about.py`).
const TAGLINE: &str = "On-demand ops console for a Windows fleet, over CrowdStrike Falcon RTR.";
const DISCLAIMER: &str = "Development-branch software — the developer is not liable for your actions, \
for bugs, or for a copy obtained without consent.";

/// The About page. Returns `true` if the "License" button was clicked.
fn about_view(ui: &mut egui::Ui) -> bool {
    let muted = egui::Color32::from_rgb(0x7a, 0x82, 0x8e);
    let mut license_clicked = false;

    // Push the block toward the vertical middle.
    let top = (ui.available_height() * 0.14).clamp(24.0, 140.0);
    ui.add_space(top);

    ui.vertical_centered(|ui| {
        ui.set_max_width(460.0);

        let (rect, _) = ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::hover());
        draw_brand(ui.painter(), rect);

        ui.add_space(14.0);
        ui.label(egui::RichText::new("LIMEN").size(30.0).strong());
        ui.label(
            egui::RichText::new(format!("v{} · development branch", env!("CARGO_PKG_VERSION")))
                .monospace()
                .color(muted),
        );

        ui.add_space(20.0);
        ui.label(egui::RichText::new(TAGLINE).size(15.0));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new(DISCLAIMER).small().color(muted));

        ui.add_space(14.0);
        ui.label(
            egui::RichText::new("Free software under the GNU GPL v3.")
                .small()
                .color(muted),
        );
        ui.add_space(4.0);
        if ui.button("License").clicked() {
            license_clicked = true;
        }
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
                ui.label(err.as_str());
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("loading…");
                });
            }
        }
    }
}
