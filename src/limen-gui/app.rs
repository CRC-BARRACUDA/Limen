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

#[derive(Clone, PartialEq)]
enum Nav {
    About,
    License,
    Modules,
    Module(String),
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

    nav: Nav,
    view: Option<ui::View>,
    view_error: Option<String>,
    inputs: HashMap<String, String>,
    output: String,
    busy: bool,

    // Modules page state
    search: String,
    filter: ModuleFilter,
    // Modules available in the GitHub org
    remote: Vec<RemoteModule>,
    remote_error: Option<String>,
    remote_loading: bool,
    remote_fetched: bool,

    // Developer window (its own OS window / viewport)
    dev_open: bool,
    dev_tab: DevTab,
    logs: std::collections::VecDeque<String>,
    log_autoscroll: bool,

    /// Module names pinned to the sidebar, in order (persisted in settings).
    pinned: Vec<String>,

    // Settings window
    settings_open: bool,
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
            nav: Nav::About,
            view: None,
            view_error: None,
            inputs: HashMap::new(),
            output: String::new(),
            busy: false,
            search: String::new(),
            filter: ModuleFilter::All,
            remote: Vec::new(),
            remote_error: None,
            remote_loading: false,
            remote_fetched: false,
            dev_open: false,
            dev_tab: DevTab::Modules,
            logs: std::collections::VecDeque::new(),
            log_autoscroll: true,
            pinned: limen_core::Config::load().map(|c| c.pinned_modules).unwrap_or_default(),
            settings_open: false,
            ui_scale: {
                let pct = limen_core::Config::load().map(|c| c.ui_scale_percent).unwrap_or(0);
                if pct == 0 { 100.0 } else { pct as f32 }
            },
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
                        // Ignore late results for a module we've navigated away from.
                        if self.nav == Nav::Module(module) {
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
                        self.output = match result {
                            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|e| e.to_string()),
                            Err(e) => format!("error: {e}"),
                        };
                        self.status = "done".to_string();
                    }
                },
                Event::Status(msg) => {
                    self.busy = false;
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
        self.nav = Nav::Module(name.clone());
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
        self.output = "running…".to_string();
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

        // Title bar
        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(0x21, 0x25, 0x2b))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("Limen").color(egui::Color32::from_rgb(0x5c, 0x9c, 0xf5)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(&self.status).weak());
                    });
                });
            });

        // Sidebar
        let mut nav_click: Option<Nav> = None;
        let mut toggle_dev = false;
        let mut toggle_settings = false;
        let mut open_pinned: Option<String> = None;
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(168.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                if ui
                    .selectable_label(self.nav == Nav::About, egui::RichText::new("About").size(15.0))
                    .clicked()
                {
                    nav_click = Some(Nav::About);
                }
                // "Modules" is selected both on the list page and any module detail.
                let on_modules = matches!(self.nav, Nav::Modules | Nav::Module(_));
                if ui
                    .selectable_label(on_modules, egui::RichText::new("Modules").size(15.0))
                    .clicked()
                {
                    nav_click = Some(Nav::Modules);
                }

                ui.add_space(8.0);
                ui.separator();

                // Middle: pinned-module icons, scrollable if there are many. Only
                // show pins that are currently installed. Leave room at the bottom
                // for the developer tool button (drawn last, bottom-anchored).
                let pins: Vec<String> = self
                    .pinned
                    .iter()
                    .filter(|n| self.modules.iter().any(|m| &m.name == *n))
                    .cloned()
                    .collect();
                if !pins.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("PINNED").small().weak());
                    ui.add_space(2.0);
                    let scroll_h = (ui.available_height() - 58.0).max(60.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .max_height(scroll_h)
                        .show(ui, |ui| {
                            for name in &pins {
                                let selected = self.nav == Nav::Module(name.clone());
                                if pinned_icon(ui, name, selected).clicked() {
                                    open_pinned = Some(name.clone());
                                }
                                ui.add_space(4.0);
                            }
                        });
                }

                // Bottom: the developer + settings tool buttons.
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if tool_button(ui, "🛠", self.dev_open, "Developer").clicked() {
                            toggle_dev = true;
                        }
                        if tool_button(ui, "⚙", self.settings_open, "Settings").clicked() {
                            toggle_settings = true;
                        }
                    });
                });
            });
        if toggle_dev {
            self.dev_open = !self.dev_open;
        }
        if toggle_settings {
            self.settings_open = !self.settings_open;
        }
        if let Some(name) = open_pinned {
            self.select_module(name);
        }

        // Central content (split-borrow so the closure can mutate inputs while
        // reading the rest of the app).
        let mut action: Option<ui::Action> = None;
        let mut open_module: Option<String> = None;
        let mut remove_module: Option<String> = None;
        let mut add_module: Option<String> = None;
        let mut toggle_pin: Option<String> = None;
        let mut go_modules = false;
        let mut go_nav: Option<Nav> = None;
        {
            let LimenApp {
                nav, modules, git_installed, pinned, view, view_error, inputs, output, busy, fatal,
                search, filter, remote, remote_error, remote_loading, ..
            } = self;
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(err) = fatal {
                    ui.colored_label(egui::Color32::LIGHT_RED, "The engine failed to start:");
                    ui.add_space(4.0);
                    ui.monospace(err.as_str());
                    return;
                }
                match nav {
                    Nav::About => {
                        if about_view(ui) {
                            go_nav = Some(Nav::License);
                        }
                    }
                    Nav::License => {
                        if ui.link("‹ About").clicked() {
                            go_nav = Some(Nav::About);
                        }
                        ui.add_space(4.0);
                        license_view(ui);
                    }
                    Nav::Modules => modules_page(
                        ui, modules, git_installed, pinned, remote, *remote_loading, remote_error,
                        filter, search, &mut open_module, &mut remove_module, &mut add_module,
                        &mut toggle_pin, &mut reload,
                    ),
                    Nav::Module(name) => {
                        if ui.link("‹ Modules").clicked() {
                            go_modules = true;
                        }
                        ui.add_space(4.0);
                        module_view(ui, name, view, view_error, inputs, output, *busy, &mut action)
                    }
                }
            });
        }

        // Developer window — a real, separate OS window (its own viewport), with
        // native resize/maximize. Toggled by the bottom-left tool button.
        if self.dev_open {
            let LimenApp { dev_open, dev_tab, inputs, logs, log_autoscroll, .. } = self;
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("limen_developer"),
                egui::ViewportBuilder::default()
                    .with_title("Limen — Developer")
                    .with_inner_size([900.0, 620.0])
                    .with_min_inner_size([420.0, 300.0]),
                |ctx, _class| {
                    ui::apply_theme(ctx);
                    egui::TopBottomPanel::top("dev_tabs").show(ctx, |ui| {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.selectable_value(dev_tab, DevTab::Modules, "Modules");
                            ui.selectable_value(dev_tab, DevTab::UiKit, "UI Kit");
                            ui.selectable_value(dev_tab, DevTab::Console, "Console");
                        });
                        ui.add_space(4.0);
                    });
                    egui::CentralPanel::default().show(ctx, |ui| match dev_tab {
                        DevTab::Modules => dev_modules_docs(ui),
                        DevTab::UiKit => ui::render_demo_ui(ui, inputs),
                        DevTab::Console => dev_console(ui, logs, log_autoscroll),
                    });
                    // The OS window's close button asks the viewport to close.
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                *dev_open = false;
            }
        }

        // Settings — a separate OS window (its own viewport). First setting:
        // global UI scale, in discrete steps.
        if self.settings_open {
            let mut close = false;
            let mut scale_changed = false;
            let scale = &mut self.ui_scale;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("limen_settings"),
                egui::ViewportBuilder::default()
                    .with_title("Limen — Settings")
                    .with_inner_size([440.0, 300.0])
                    .with_min_inner_size([360.0, 220.0]),
                |ctx, _class| {
                    ui::apply_theme(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
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
                                if ui
                                    .selectable_label(selected, format!("{}%", pct as u32))
                                    .clicked()
                                {
                                    *scale = pct;
                                    scale_changed = true;
                                }
                            }
                        });
                    });
                    if ctx.input(|i| i.viewport().close_requested()) {
                        close = true;
                    }
                },
            );
            if close {
                self.settings_open = false;
            }
            if scale_changed {
                self.save_ui_scale();
            }
        }

        if let Some(n) = nav_click {
            match n {
                Nav::Module(name) => self.select_module(name),
                other => self.nav = other,
            }
        }
        if go_modules {
            self.nav = Nav::Modules;
        }
        if let Some(n) = go_nav {
            self.nav = n;
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
            self.dispatch(a);
        }

        // Fetch the org's module list the first time we land on the Modules page.
        if matches!(self.nav, Nav::Modules) && !self.remote_fetched {
            self.remote_fetched = true;
            self.remote_loading = true;
            self.worker.send(Command::ListRemote);
        }

        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

// --------------------------------------------------------------------------- //

/// A bottom-left sidebar tool button — a glyph on a rounded tile. Highlights
/// when its window is open.
fn tool_button(ui: &mut egui::Ui, glyph: &str, active: bool, tooltip: &str) -> egui::Response {
    let size = egui::vec2(34.0, 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if active {
        ui::color::ACCENT
    } else if resp.hovered() {
        ui::color::BG_HOVER
    } else {
        ui::color::BG_WIDGET
    };
    let fg = if active { ui::color::ON_ACCENT } else { ui::color::TEXT };
    let p = ui.painter();
    p.rect_filled(rect, egui::Rounding::same(6.0_f32), bg);
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(17.0),
        fg,
    );
    resp.on_hover_text(tooltip)
}

/// A pinned-module icon for the sidebar: a rounded tile with the module's
/// initials, highlighted when it's the active module. Tooltip shows the name.
fn pinned_icon(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let size = egui::vec2(40.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if selected {
        ui::color::ACCENT
    } else if resp.hovered() {
        ui::color::BG_HOVER
    } else {
        ui::color::BG_WIDGET
    };
    let fg = if selected { ui::color::ON_ACCENT } else { ui::color::TEXT };
    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
    let p = ui.painter();
    p.rect_filled(rect, egui::Rounding::same(7.0_f32), bg);
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        &initials,
        egui::FontId::proportional(14.0),
        fg,
    );
    resp.on_hover_text(name)
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
    busy: bool,
    action: &mut Option<ui::Action>,
) {
    match view {
        Some(v) => {
            ui.heading(if v.title.is_empty() { name } else { &v.title });
            ui.separator();
            if let Some(a) = ui::render_view(ui, v, inputs) {
                *action = Some(a);
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Result").strong());
                if busy {
                    ui.spinner();
                }
            });
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let mut text = output;
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_width(f32::INFINITY)
                        .code_editor(),
                );
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
