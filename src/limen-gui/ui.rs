//! The GUI core: a declarative view protocol + a single standardized renderer.
//!
//! Modules can't call egui directly (they're separate processes, in any
//! language), so instead each module *describes* the UI it wants by answering a
//! `ui` request with a [`View`] — a small widget tree in JSON. This core renders
//! that tree with one consistent (Zed One Dark) style and routes button clicks
//! back to the module's capabilities. So every module "draws its own window"
//! while the look-and-feel stays uniform and is defined in exactly one place.
//!
//! The same primitives are showcased in the debug-only `demo-ui` gallery, which
//! is where the styles are standardized.

use std::collections::HashMap;

use eframe::egui;
use serde::Deserialize;
use serde_json::Value;

// --------------------------------------------------------------------------- //
// Zed One Dark palette
// --------------------------------------------------------------------------- //
pub(crate) mod color {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x1c, 0x1f, 0x24); // deepest panel
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x21, 0x25, 0x2b); // inputs/code
    pub const BG_WIDGET: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3b); // buttons/rows
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x34, 0x3a, 0x46);
    pub const BORDER: Color32 = Color32::from_rgb(0x3b, 0x41, 0x4d);
    pub const TEXT: Color32 = Color32::from_rgb(0xc8, 0xcc, 0xd4);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x7a, 0x82, 0x8e);
    pub const ACCENT: Color32 = Color32::from_rgb(0x5c, 0x9c, 0xf5); // soft blue
    pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0x82, 0xb6, 0xff); // accent, hovered
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0xf5, 0xf8, 0xff);
}

// --------------------------------------------------------------------------- //
// Animation toggle + animated primary button
// --------------------------------------------------------------------------- //

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether UI animations are enabled — toggled from Settings, on by default.
static ANIMATIONS: AtomicBool = AtomicBool::new(true);

/// Enable/disable UI animations globally (set from the Settings checkbox and at
/// startup from the saved config).
pub fn set_animations(on: bool) {
    ANIMATIONS.store(on, Ordering::Relaxed);
}

/// Whether animations are currently on.
pub fn animations_enabled() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}

/// `c` with its alpha scaled by `t` (0 = transparent, 1 = opaque).
fn with_alpha(c: egui::Color32, t: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (255.0 * t.clamp(0.0, 1.0)) as u8)
}

/// Channel-wise linear interpolation between two (opaque) colors.
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let c = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// An accent-filled primary button that, when animations are on, smoothly
/// brightens on hover and dips while pressed. When off it's a plain accent
/// button. Returns the click response like any button.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::ON_ACCENT);
    let padding = egui::vec2(14.0, 7.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let anim = animations_enabled();
    // `animate_bool_with_time` eases 0→1 and keeps requesting repaints until it
    // settles; a longer time makes the motion clearly visible. Off → snap.
    let hover_t = if anim {
        ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.18)
    } else {
        resp.hovered() as u8 as f32
    };
    let press_t = if anim {
        ui.ctx()
            .animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), 0.07)
    } else {
        resp.is_pointer_button_down_on() as u8 as f32
    };

    // Visible motion: grow ~2px on hover, dip inward on press (springs back).
    let draw = rect.expand(2.0 * hover_t - 3.0 * press_t);
    let mut fill = lerp_color(color::ACCENT, color::ACCENT_BRIGHT, hover_t);
    fill = lerp_color(fill, color::BG, 0.25 * press_t);

    let painter = ui.painter();
    painter.rect_filled(draw, egui::Rounding::same(6.0), fill);
    painter.galley(draw.center() - galley.size() * 0.5, galley, color::ON_ACCENT);
    resp
}

/// A secondary button that fades a blue outline in on hover (animated), rather
/// than the outline snapping on. `min` is a minimum size (e.g. a fixed column
/// width); the button grows to fit its text. With animations off the outline
/// snaps.
pub fn outline_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::TEXT);
    let padding = egui::vec2(12.0, 6.0);
    let size = (galley.size() + padding * 2.0).max(min);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let anim = animations_enabled();
    let t = if anim {
        ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.16)
    } else {
        resp.hovered() as u8 as f32
    };
    // Press eases in fast while held and springs back on release.
    let press_t = if anim {
        ui.ctx()
            .animate_bool_with_time(resp.id.with("press"), resp.is_pointer_button_down_on(), 0.06)
    } else {
        resp.is_pointer_button_down_on() as u8 as f32
    };

    // Dip inward while pressed, pop back out on release.
    let draw = rect.shrink(2.0 * press_t);
    let rounding = egui::Rounding::same(6.0);
    let painter = ui.painter();
    let base = lerp_color(color::BG_WIDGET, color::BG_HOVER, t);
    painter.rect_filled(draw, rounding, lerp_color(base, color::BG, 0.35 * press_t));
    // Blue outline whose opacity eases in with hover.
    let outline = egui::Color32::from_rgba_unmultiplied(
        color::ACCENT.r(),
        color::ACCENT.g(),
        color::ACCENT.b(),
        (255.0 * t) as u8,
    );
    painter.rect_stroke(draw, rounding, egui::Stroke::new(1.5_f32, outline));
    painter.galley(draw.center() - galley.size() * 0.5, galley, color::TEXT);
    resp
}

/// A pill filter chip (All / Installed / …). Its hover fill fades in, and its
/// accent outline + text tint ease in when it becomes selected — so switching
/// filters slides the highlight from one chip to the next instead of snapping.
pub fn filter_chip(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::TEXT);
    let padding = egui::vec2(12.0, 5.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let anim = animations_enabled();
    let hover_t = if anim {
        ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.14)
    } else {
        resp.hovered() as u8 as f32
    };
    let sel_t = if anim {
        ui.ctx().animate_bool_with_time(resp.id.with("sel"), selected, 0.14)
    } else {
        selected as u8 as f32
    };

    let rounding = egui::Rounding::same(7.0);
    let painter = ui.painter();
    // Background: hover fills; selected half-fills.
    painter.rect_filled(rect, rounding, with_alpha(color::BG_ELEVATED, hover_t.max(sel_t * 0.6)));
    // Accent outline eases in with selection.
    painter.rect_stroke(rect, rounding, egui::Stroke::new(1.0_f32, with_alpha(color::ACCENT, sel_t)));
    // Text tints white → accent as it becomes selected.
    painter.galley(
        rect.center() - galley.size() * 0.5,
        galley,
        lerp_color(color::TEXT, color::ACCENT, sel_t),
    );
    resp
}

/// Apply the Zed One Dark theme to the whole context.
pub fn apply_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, Rounding, Stroke, TextStyle};

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(12.0);

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(color::TEXT);
    v.panel_fill = color::BG;
    v.window_fill = color::BG;
    v.extreme_bg_color = color::BG_ELEVATED;
    v.faint_bg_color = color::BG_ELEVATED;
    v.window_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.window_rounding = Rounding::same(6.0_f32);
    v.hyperlink_color = color::ACCENT;
    v.selection.bg_fill = color::ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, color::ACCENT);

    let rounding = Rounding::same(5.0_f32);
    v.widgets.noninteractive.bg_fill = color::BG;
    v.widgets.noninteractive.weak_bg_fill = color::BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.noninteractive.rounding = rounding;

    v.widgets.inactive.bg_fill = color::BG_WIDGET;
    v.widgets.inactive.weak_bg_fill = color::BG_WIDGET;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.inactive.rounding = rounding;

    v.widgets.hovered.bg_fill = color::BG_HOVER;
    v.widgets.hovered.weak_bg_fill = color::BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, color::ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, color::TEXT);
    v.widgets.hovered.rounding = rounding;

    v.widgets.active.bg_fill = color::ACCENT;
    v.widgets.active.weak_bg_fill = color::ACCENT;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, color::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, color::ON_ACCENT);
    v.widgets.active.rounding = rounding;

    v.widgets.open.bg_fill = color::BG_ELEVATED;
    v.widgets.open.weak_bg_fill = color::BG_ELEVATED;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, color::BORDER);
    v.widgets.open.rounding = rounding;

    ctx.set_style(style);
}

// --------------------------------------------------------------------------- //
// The declarative view spec (what a module returns from `ui`)
// --------------------------------------------------------------------------- //

/// A module-described view.
#[derive(Debug, Clone, Deserialize)]
pub struct View {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

/// The capability + method a button invokes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub capability: String,
    pub method: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelStyle {
    #[default]
    Normal,
    Heading,
    Strong,
    Weak,
    Mono,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Default,
    Primary,
}

/// Serde default for `#[serde(default = "default_true")]` bool fields.
fn default_true() -> bool {
    true
}

/// One widget in a [`View`]. `kind` is the tag.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Widget {
    Label {
        text: String,
        #[serde(default)]
        style: LabelStyle,
    },
    Text {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        default: String,
        /// Mask the input (single-line only), for secrets.
        #[serde(default)]
        password: bool,
    },
    Select {
        id: String,
        #[serde(default)]
        label: String,
        options: Vec<String>,
        #[serde(default)]
        default: String,
    },
    Button {
        text: String,
        action: Action,
        #[serde(default)]
        style: ButtonStyle,
        /// Whether the button is clickable (default true).
        #[serde(default = "default_true")]
        enabled: bool,
    },
    Separator,
    Row {
        children: Vec<Widget>,
    },
    /// A table with a header row and string cells.
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
    },
}

/// Render a view; returns the action of a clicked button, if any. `busy` is the
/// action currently in flight (its button shows a spinner).
pub fn render_view(
    ui: &mut egui::Ui,
    view: &View,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
) -> Option<Action> {
    let mut clicked = None;
    render_widgets(ui, &view.widgets, inputs, busy, &mut clicked);
    clicked
}

fn render_widgets(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Action>,
) {
    for w in widgets {
        render_widget(ui, w, inputs, busy, clicked);
    }
}

fn render_widget(
    ui: &mut egui::Ui,
    widget: &Widget,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Action>,
) {
    match widget {
        Widget::Label { text, style } => {
            ui.label(styled(text, *style));
        }
        Widget::Text { id, label, placeholder, multiline, default, password } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let value = inputs.entry(id.clone()).or_insert_with(|| default.clone());
            let editor = if *multiline {
                egui::TextEdit::multiline(value)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .hint_text(placeholder.as_str())
            } else {
                egui::TextEdit::singleline(value)
                    .desired_width(f32::INFINITY)
                    .hint_text(placeholder.as_str())
                    .password(*password)
            };
            ui.add(editor);
        }
        Widget::Select { id, label, options, default } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let initial = if default.is_empty() {
                options.first().cloned().unwrap_or_default()
            } else {
                default.clone()
            };
            let value = inputs.entry(id.clone()).or_insert(initial);
            egui::ComboBox::from_id_source(id)
                .selected_text(value.clone())
                .show_ui(ui, |ui| {
                    for opt in options {
                        ui.selectable_value(value, opt.clone(), opt);
                    }
                });
        }
        Widget::Button { text, action, style, enabled } => {
            let button = match style {
                ButtonStyle::Primary => egui::Button::new(
                    egui::RichText::new(text).color(color::ON_ACCENT),
                )
                .fill(color::ACCENT),
                ButtonStyle::Default => egui::Button::new(text),
            };
            let running = busy == Some(action);
            ui.horizontal(|ui| {
                if ui.add_enabled(*enabled, button).clicked() {
                    *clicked = Some(action.clone());
                }
                if running {
                    ui.add_space(6.0);
                    ui.spinner(); // animates while this action is in flight
                }
            });
        }
        Widget::Separator => {
            ui.separator();
        }
        Widget::Row { children } => {
            ui.horizontal(|ui| render_widgets(ui, children, inputs, busy, clicked));
        }
        Widget::Table { columns, rows } => render_table(ui, columns, rows),
    }
}

/// Render a [`Widget::Table`] as a striped grid inside a scroll area.
fn render_table(ui: &mut egui::Ui, columns: &[String], rows: &[Vec<String>]) {
    let ncols = columns.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if ncols == 0 {
        return;
    }
    let id = ui.make_persistent_id(("limen_table", ncols, rows.len()));
    egui::ScrollArea::horizontal()
        .id_source(id)
        .show(ui, |ui| {
            egui::Grid::new(id)
                .striped(true)
                .num_columns(ncols)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for c in columns {
                        ui.label(egui::RichText::new(c).strong());
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            ui.label(cell);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn styled(text: &str, style: LabelStyle) -> egui::RichText {
    let rt = egui::RichText::new(text);
    match style {
        LabelStyle::Normal => rt,
        LabelStyle::Heading => rt.heading(),
        LabelStyle::Strong => rt.strong(),
        LabelStyle::Weak => rt.color(color::TEXT_MUTED),
        LabelStyle::Mono => rt.monospace(),
    }
}

// --------------------------------------------------------------------------- //
// A tiny Markdown renderer — just enough for GitHub release notes (headings,
// bold, inline code, bullet lists, links). Not full CommonMark; anything it
// doesn't recognise degrades to readable text.
// --------------------------------------------------------------------------- //

/// One inline span of a Markdown line.
#[derive(Debug, PartialEq)]
enum Md {
    Text(String),
    Bold(String),
    Code(String),
    Link { label: String, url: String },
}

/// Split a line into inline spans, handling `**bold**`, `` `code` `` and
/// `[label](url)`. Unmatched markers stay literal.
fn inline_segments(line: &str) -> Vec<Md> {
    let mut out: Vec<Md> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Md>| {
        if !buf.is_empty() {
            out.push(Md::Text(std::mem::take(buf)));
        }
    };
    let mut rest = line;
    while !rest.is_empty() {
        // Index of the next markup opener, whichever comes first.
        let next = ["**", "`", "["]
            .iter()
            .filter_map(|m| rest.find(m).map(|i| (i, *m)))
            .min_by_key(|(i, _)| *i);
        let Some((i, marker)) = next else {
            buf.push_str(rest);
            break;
        };
        buf.push_str(&rest[..i]);
        let after = &rest[i + marker.len()..];
        match marker {
            "**" => match after.find("**") {
                Some(j) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Bold(after[..j].to_string()));
                    rest = &after[j + 2..];
                }
                None => {
                    buf.push_str("**");
                    rest = after;
                }
            },
            "`" => match after.find('`') {
                Some(j) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Code(after[..j].to_string()));
                    rest = &after[j + 1..];
                }
                None => {
                    buf.push('`');
                    rest = after;
                }
            },
            _ => match parse_link(&rest[i..]) {
                Some((label, url, consumed)) => {
                    flush(&mut buf, &mut out);
                    out.push(Md::Link { label, url });
                    rest = &rest[i + consumed..];
                }
                None => {
                    buf.push('[');
                    rest = after;
                }
            },
        }
    }
    flush(&mut buf, &mut out);
    out
}

/// Parse `[label](url)` at the start of `s`; returns (label, url, bytes consumed).
fn parse_link(s: &str) -> Option<(String, String, usize)> {
    let close = s.find(']')?;
    let tail = &s[close + 1..];
    if !tail.starts_with('(') {
        return None;
    }
    let end = tail.find(')')?;
    Some((s[1..close].to_string(), tail[1..end].to_string(), close + 1 + end + 1))
}

/// Render a line's inline spans into the current (wrapped) row.
fn render_spans(ui: &mut egui::Ui, spans: Vec<Md>) {
    for span in spans {
        match span {
            Md::Text(t) => {
                ui.label(egui::RichText::new(t).color(color::TEXT_MUTED));
            }
            Md::Bold(t) => {
                ui.label(egui::RichText::new(t).strong().color(color::TEXT));
            }
            Md::Code(t) => {
                ui.label(egui::RichText::new(t).monospace().color(color::ACCENT));
            }
            Md::Link { label, url } => {
                ui.hyperlink_to(label, url);
            }
        }
    }
}

/// Render a small subset of Markdown (headings, bold, inline code, bullets,
/// links) — enough for release notes.
pub fn markdown(ui: &mut egui::Ui, md: &str) {
    for raw in md.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            ui.add_space(6.0);
        } else if let Some(rest) = trimmed.strip_prefix("### ") {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(rest).strong().size(15.0).color(color::TEXT));
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(rest).strong().size(17.0).color(color::TEXT));
        } else if let Some(rest) = trimmed.strip_prefix("# ") {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(rest).heading().color(color::TEXT));
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label(egui::RichText::new("  •  ").color(color::TEXT_MUTED));
                render_spans(ui, inline_segments(rest));
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                render_spans(ui, inline_segments(line));
            });
        }
    }
}

/// Gather the current values of every input widget into a params object,
/// keyed by widget `id`.
pub fn collect_params(view: &View, inputs: &HashMap<String, String>) -> Value {
    let mut map = serde_json::Map::new();
    collect_ids(&view.widgets, inputs, &mut map);
    Value::Object(map)
}

fn collect_ids(
    widgets: &[Widget],
    inputs: &HashMap<String, String>,
    map: &mut serde_json::Map<String, Value>,
) {
    for w in widgets {
        match w {
            Widget::Text { id, .. } | Widget::Select { id, .. } => {
                if let Some(v) = inputs.get(id) {
                    map.insert(id.clone(), Value::String(v.clone()));
                }
            }
            Widget::Row { children } => collect_ids(children, inputs, map),
            _ => {}
        }
    }
}

/// The component gallery — the UI Kit shown in the Developer window, and the
/// standardized source of truth for module widget styling.
pub fn render_demo_ui(ui: &mut egui::Ui, inputs: &mut HashMap<String, String>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
    ui.heading("UI Kit");
    ui.label(styled(
        "Standardized Limen widgets — the single source of truth for module styling.",
        LabelStyle::Weak,
    ));
    ui.separator();

    ui.add_space(4.0);
    ui.label(styled("Typography", LabelStyle::Strong));
    ui.label(styled("Heading", LabelStyle::Heading));
    ui.label(styled("Body text — the default", LabelStyle::Normal));
    ui.label(styled("Strong emphasis", LabelStyle::Strong));
    ui.label(styled("Muted / secondary", LabelStyle::Weak));
    ui.label(styled("monospace / code", LabelStyle::Mono));

    ui.add_space(10.0);
    ui.label(styled("Buttons", LabelStyle::Strong));
    ui.horizontal(|ui| {
        let _ = ui.add(egui::Button::new(egui::RichText::new("Primary").color(color::ON_ACCENT)).fill(color::ACCENT));
        let _ = ui.button("Default");
    });

    ui.add_space(10.0);
    ui.label(styled("Inputs", LabelStyle::Strong));
    let text = inputs.entry("demo.text".into()).or_insert_with(|| "editable".into());
    ui.add(egui::TextEdit::singleline(text).desired_width(240.0).hint_text("single line"));
    let sel = inputs.entry("demo.select".into()).or_insert_with(|| "one".into());
    egui::ComboBox::from_id_source("demo.select")
        .selected_text(sel.clone())
        .show_ui(ui, |ui| {
            for o in ["one", "two", "three"] {
                ui.selectable_value(sel, o.to_string(), o);
            }
        });

    ui.add_space(10.0);
    ui.label(styled("Palette", LabelStyle::Strong));
    ui.horizontal(|ui| {
        for (name, c) in [
            ("bg", color::BG),
            ("elevated", color::BG_ELEVATED),
            ("widget", color::BG_WIDGET),
            ("border", color::BORDER),
            ("accent", color::ACCENT),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 34.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, egui::Rounding::same(5.0_f32), c);
            ui.painter().text(
                rect.center_bottom() + egui::vec2(0.0, -2.0),
                egui::Align2::CENTER_BOTTOM,
                name,
                egui::FontId::proportional(10.0),
                color::TEXT,
            );
        }
    });
    }); // end ScrollArea
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_inline_tokenizes_bold_code_and_links() {
        let segs = inline_segments("**UI widgets** — a `table` and a [link](http://x)");
        assert_eq!(
            segs,
            vec![
                Md::Bold("UI widgets".into()),
                Md::Text(" — a ".into()),
                Md::Code("table".into()),
                Md::Text(" and a ".into()),
                Md::Link { label: "link".into(), url: "http://x".into() },
            ]
        );
        // Unmatched markers stay literal.
        assert_eq!(
            inline_segments("a ** b `c"),
            vec![Md::Text("a ** b `c".into())]
        );
        // Plain text is a single span.
        assert_eq!(inline_segments("just text"), vec![Md::Text("just text".into())]);
    }

    #[test]
    fn parses_a_module_view_and_collects_params() {
        // The exact shape modules emit from their `ui` method.
        let json = serde_json::json!({
            "title": "PowerShell",
            "widgets": [
                {"kind": "label", "text": "hi", "style": "weak"},
                {"kind": "text", "id": "command", "label": "Command", "multiline": true},
                {"kind": "button", "text": "Run", "style": "primary",
                 "action": {"capability": "ops.powershell", "method": "run"}}
            ]
        });
        let view: View = serde_json::from_value(json).unwrap();
        assert_eq!(view.title, "PowerShell");
        assert_eq!(view.widgets.len(), 3);

        // The renderer would populate `inputs`; collecting turns them into params.
        let mut inputs = HashMap::new();
        inputs.insert("command".to_string(), "Get-Process".to_string());
        let params = collect_params(&view, &inputs);
        assert_eq!(params["command"], "Get-Process");
    }
}
