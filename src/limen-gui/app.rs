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

use std::collections::HashMap;
use std::time::Duration;

use eframe::egui;
use limen_core::ModuleSpec;

use crate::ui;
use crate::worker::{Command, Event, RunTag, Worker};

#[derive(Clone, PartialEq)]
enum Nav {
    About,
    Modules,
    Module(String),
    #[cfg(debug_assertions)]
    DemoUi,
}

pub struct LimenApp {
    worker: Worker,
    status: String,
    fatal: Option<String>,
    modules: Vec<ModuleSpec>,

    nav: Nav,
    view: Option<ui::View>,
    view_error: Option<String>,
    inputs: HashMap<String, String>,
    output: String,
    busy: bool,

    // Modules page state
    search: String,
    category: String,
    install_ref: String,
}

impl LimenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Vec<std::path::PathBuf>) -> Self {
        ui::apply_theme(&cc.egui_ctx);
        Self {
            worker: Worker::spawn(dirs),
            status: "starting modules…".to_string(),
            fatal: None,
            modules: Vec::new(),
            nav: Nav::About,
            view: None,
            view_error: None,
            inputs: HashMap::new(),
            output: String::new(),
            busy: false,
            search: String::new(),
            category: "All".to_string(),
            install_ref: String::new(),
        }
    }

    fn drain_events(&mut self) {
        while let Ok(evt) = self.worker.rx.try_recv() {
            match evt {
                Event::Ready(m) | Event::Modules(m) => {
                    self.modules = m;
                    self.status = format!("{} module(s) loaded", self.modules.len());
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
                    self.status = msg;
                }
                Event::Fatal(e) => {
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

        // Title bar
        let mut reload = false;
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
                        if ui.button("Reload").clicked() {
                            reload = true;
                        }
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(&self.status).weak());
                    });
                });
            });
        if reload {
            self.worker.send(Command::Refresh);
        }

        // Sidebar
        let mut nav_click: Option<Nav> = None;
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

                #[cfg(debug_assertions)]
                {
                    ui.add_space(6.0);
                    ui.separator();
                    if ui
                        .selectable_label(self.nav == Nav::DemoUi, egui::RichText::new("demo-ui").weak())
                        .clicked()
                    {
                        nav_click = Some(Nav::DemoUi);
                    }
                }
            });

        // Central content (split-borrow so the closure can mutate inputs while
        // reading the rest of the app).
        let mut action: Option<ui::Action> = None;
        let mut open_module: Option<String> = None;
        let mut remove_module: Option<String> = None;
        let mut add_module: Option<String> = None;
        let mut go_modules = false;
        {
            let LimenApp {
                nav, modules, view, view_error, inputs, output, busy, fatal, search, category,
                install_ref, ..
            } = self;
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(err) = fatal {
                    ui.colored_label(egui::Color32::LIGHT_RED, "The engine failed to start:");
                    ui.add_space(4.0);
                    ui.monospace(err.as_str());
                    return;
                }
                match nav {
                    Nav::About => about_view(ui),
                    Nav::Modules => modules_page(
                        ui, modules, search, category, install_ref,
                        &mut open_module, &mut remove_module, &mut add_module,
                    ),
                    Nav::Module(name) => {
                        if ui.link("‹ Modules").clicked() {
                            go_modules = true;
                        }
                        ui.add_space(4.0);
                        module_view(ui, name, view, view_error, inputs, output, *busy, &mut action)
                    }
                    #[cfg(debug_assertions)]
                    Nav::DemoUi => ui::render_demo_ui(ui, inputs),
                }
            });
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
        if let Some(a) = action {
            self.dispatch(a);
        }

        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

// --------------------------------------------------------------------------- //

/// The Modules page — a Zed-Extensions-style list of installed modules.
#[allow(clippy::too_many_arguments)]
fn modules_page(
    ui: &mut egui::Ui,
    modules: &[ModuleSpec],
    search: &mut String,
    category: &mut String,
    install_ref: &mut String,
    open: &mut Option<String>,
    remove: &mut Option<String>,
    add: &mut Option<String>,
) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Modules");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_add = !install_ref.trim().is_empty();
            if ui.add_enabled(can_add, egui::Button::new("Install")).clicked() {
                *add = Some(install_ref.trim().to_string());
                install_ref.clear();
            }
            ui.add(
                egui::TextEdit::singleline(install_ref)
                    .hint_text("owner/repo@version  or  ./path")
                    .desired_width(260.0),
            );
        });
    });
    ui.add_space(10.0);

    // Search box.
    ui.add(
        egui::TextEdit::singleline(search)
            .hint_text("Search modules…")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);

    // Category chips, derived from capability namespaces (the part before the dot).
    let mut categories = vec!["All".to_string()];
    for m in modules {
        for cap in &m.capabilities {
            if let Some((ns, _)) = cap.split_once('.') {
                if !categories.iter().any(|c| c == ns) {
                    categories.push(ns.to_string());
                }
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        for cat in &categories {
            let label = if *category == *cat {
                egui::RichText::new(cat).color(ui::color::ACCENT)
            } else {
                egui::RichText::new(cat)
            };
            if ui.selectable_label(*category == *cat, label).clicked() {
                *category = cat.clone();
            }
        }
    });
    ui.add_space(6.0);
    ui.separator();

    let query = search.to_lowercase();
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(4.0);
        let mut shown = 0;
        for m in modules {
            if !matches_filter(m, &query, category) {
                continue;
            }
            module_card(ui, m, open, remove);
            ui.add_space(10.0);
            shown += 1;
        }
        if shown == 0 {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("No modules match.").color(ui::color::TEXT_MUTED));
            });
        }
    });
}

fn matches_filter(m: &ModuleSpec, query: &str, category: &str) -> bool {
    let in_category = category == "All"
        || m.capabilities
            .iter()
            .any(|c| c.split_once('.').map(|(ns, _)| ns == category).unwrap_or(false));
    if !in_category {
        return false;
    }
    if query.is_empty() {
        return true;
    }
    m.name.to_lowercase().contains(query)
        || m.description.as_deref().unwrap_or("").to_lowercase().contains(query)
        || m.capabilities.iter().any(|c| c.to_lowercase().contains(query))
}

/// A single Zed-style module card, in its own rounded box.
fn module_card(
    ui: &mut egui::Ui,
    m: &ModuleSpec,
    open: &mut Option<String>,
    remove: &mut Option<String>,
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
                        if ui.add_sized(bw, egui::Button::new("Remove")).clicked() {
                            *remove = Some(m.name.clone());
                        }
                        if let Some(repo) = &m.repo {
                            if ui.add_sized(bw, egui::Button::new("GitHub ↗")).clicked() {
                                ui.output_mut(|o| {
                                    o.open_url = Some(egui::OpenUrl::new_tab(repo_url(repo)));
                                });
                            }
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

fn about_view(ui: &mut egui::Ui) {
    let muted = egui::Color32::from_rgb(0x7a, 0x82, 0x8e);

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
