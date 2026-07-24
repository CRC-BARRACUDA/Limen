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
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0xf5, 0xf8, 0xff);
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
#[derive(Debug, Clone, Deserialize)]
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
    },
    Separator,
    Row {
        children: Vec<Widget>,
    },
}

/// Render a view; returns the action of a clicked button, if any.
pub fn render_view(
    ui: &mut egui::Ui,
    view: &View,
    inputs: &mut HashMap<String, String>,
) -> Option<Action> {
    let mut clicked = None;
    render_widgets(ui, &view.widgets, inputs, &mut clicked);
    clicked
}

fn render_widgets(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    inputs: &mut HashMap<String, String>,
    clicked: &mut Option<Action>,
) {
    for w in widgets {
        render_widget(ui, w, inputs, clicked);
    }
}

fn render_widget(
    ui: &mut egui::Ui,
    widget: &Widget,
    inputs: &mut HashMap<String, String>,
    clicked: &mut Option<Action>,
) {
    match widget {
        Widget::Label { text, style } => {
            ui.label(styled(text, *style));
        }
        Widget::Text { id, label, placeholder, multiline, default } => {
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
        Widget::Button { text, action, style } => {
            let button = match style {
                ButtonStyle::Primary => egui::Button::new(
                    egui::RichText::new(text).color(color::ON_ACCENT),
                )
                .fill(color::ACCENT),
                ButtonStyle::Default => egui::Button::new(text),
            };
            if ui.add(button).clicked() {
                *clicked = Some(action.clone());
            }
        }
        Widget::Separator => {
            ui.separator();
        }
        Widget::Row { children } => {
            ui.horizontal(|ui| render_widgets(ui, children, inputs, clicked));
        }
    }
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
