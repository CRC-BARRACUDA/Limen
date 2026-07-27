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
// Animation toolkit — reusable primitives + widgets built on them.
//
// Every animated widget is assembled from a few pieces: the global on/off switch
// ([`animations_enabled`]), the easing/colour helpers, and [`interact`] — which
// turns a [`Response`](egui::Response) into eased hover/press factors. A new
// animated widget just allocates a rect and paints from those factors; entrance
// animations use [`reveal_t`]. Nothing here is Modules-specific, so it can be
// reused anywhere.
// --------------------------------------------------------------------------- //

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether UI animations are enabled — toggled from Settings, on by default.
static ANIMATIONS: AtomicBool = AtomicBool::new(true);

/// Enable/disable UI animations globally (Settings checkbox + startup config).
pub fn set_animations(on: bool) {
    ANIMATIONS.store(on, Ordering::Relaxed);
}

/// Whether animations are currently on.
pub fn animations_enabled() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}

// ---- easing ---------------------------------------------------------------- //

/// Ease-out cubic: fast start, gentle finish — for entrances.
pub fn ease_out(t: f32) -> f32 {
    let i = 1.0 - t;
    1.0 - i * i * i
}

/// Smoothstep: gentle at both ends — for reversible motion / collapses.
pub fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

// ---- colour ---------------------------------------------------------------- //

/// Channel-wise linear interpolation between two opaque colours.
pub fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let c = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(c(a.r(), b.r()), c(a.g(), b.g()), c(a.b(), b.b()))
}

/// `c` with its alpha scaled by `t` (0 = transparent, 1 = opaque) — to fade a
/// fill or stroke in.
pub fn with_alpha(c: egui::Color32, t: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (255.0 * t.clamp(0.0, 1.0)) as u8)
}

// ---- interaction ----------------------------------------------------------- //

/// Eased 0→1 for a boolean (hover, selection, …) keyed by `id`. Snaps to the
/// target when animations are off. The building block for the rest.
pub fn anim_bool(ui: &egui::Ui, id: egui::Id, on: bool, time: f32) -> f32 {
    if animations_enabled() {
        ui.ctx().animate_bool_with_time(id, on, time)
    } else {
        on as u8 as f32
    }
}

/// Animated hover + press factors (0..1) for an interactive response — the base
/// for any custom animated widget: allocate a rect, then paint from these.
pub struct Interact {
    pub hover: f32,
    pub press: f32,
}

/// Compute the [`Interact`] factors for `resp`.
pub fn interact(ui: &egui::Ui, resp: &egui::Response) -> Interact {
    Interact {
        hover: anim_bool(ui, resp.id.with("hover"), resp.hovered(), 0.16),
        press: anim_bool(ui, resp.id.with("press"), resp.is_pointer_button_down_on(), 0.06),
    }
}

// ---- entrance -------------------------------------------------------------- //

/// Staggered entrance factor (eased 0→1) for the `index`-th item revealed at
/// `start_at`, evaluated at `now`, with `stagger`/`dur` seconds. Requests
/// repaints until it settles; returns 1 instantly when animations are off. Apply
/// it however you like (opacity, slide, …).
pub fn reveal_t(ui: &egui::Ui, index: usize, start_at: f64, now: f64, stagger: f64, dur: f64) -> f32 {
    if !animations_enabled() {
        return 1.0;
    }
    let start = start_at + index as f64 * stagger;
    let t = ease_out((((now - start) / dur).clamp(0.0, 1.0)) as f32);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    t
}

// ---- widgets --------------------------------------------------------------- //

/// A single-line text field whose border eases from grey (`BORDER`) to `ACCENT`
/// on focus (and part-way on hover) — instead of snapping. It owns the border:
/// egui's own frame stroke is suppressed so there's no double outline.
pub fn text_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    // Draw the field with no frame stroke (the fill stays), so ours is the only
    // border. `scope` keeps the visuals tweak local to this widget.
    let resp = ui
        .scope(|ui| {
            let none = egui::Stroke::NONE;
            let v = ui.visuals_mut();
            v.widgets.inactive.bg_stroke = none;
            v.widgets.hovered.bg_stroke = none;
            v.widgets.active.bg_stroke = none;
            // The TextEdit draws its *focus* border from `selection.stroke`, not
            // the widget states — null it too (the cursor uses `text_cursor`).
            v.selection.stroke = none;
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    // Taller box: more vertical padding so the caret sits inside
                    // it with room, rather than looking taller than the field.
                    .margin(egui::Margin::symmetric(8.0, 7.0))
                    .desired_width(width),
            )
        })
        .inner;

    let focus = anim_bool(ui, resp.id.with("focus"), resp.has_focus(), 0.14);
    let hover = anim_bool(ui, resp.id.with("ring-hover"), resp.hovered(), 0.14);
    let t = focus.max(hover * 0.5);
    // egui reports `resp.rect` as the *inner* rect (outer − margin) but draws the
    // frame at the outer rect, so expand by the margin to hug the box edge.
    let frame_rect = resp.rect.expand2(egui::vec2(8.0, 7.0));
    ui.painter().rect_stroke(
        frame_rect,
        egui::Rounding::same(5.0),
        egui::Stroke::new(1.0_f32, lerp_color(color::BORDER, color::ACCENT, t)),
    );
    resp
}

/// An accent-filled primary button: brightens + grows on hover, dips on press.
/// `min` is a minimum size (`Vec2::ZERO` to size to the text; a fixed width for a
/// button column).
pub fn primary_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::ON_ACCENT);
    let padding = egui::vec2(14.0, 7.0);
    let size = (galley.size() + padding * 2.0).max(min);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let a = interact(ui, &resp);
    let draw = rect.expand(2.0 * a.hover - 3.0 * a.press);
    let fill = lerp_color(
        lerp_color(color::ACCENT, color::ACCENT_BRIGHT, a.hover),
        color::BG,
        0.25 * a.press,
    );

    let painter = ui.painter();
    painter.rect_filled(draw, egui::Rounding::same(6.0), fill);
    painter.galley(draw.center() - galley.size() * 0.5, galley, color::ON_ACCENT);
    resp
}

/// A secondary button: a blue outline fades in on hover, dips on press. `min` is
/// a minimum size (e.g. a fixed column width); the button grows to fit its text.
pub fn outline_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::TEXT);
    let padding = egui::vec2(12.0, 6.0);
    let size = (galley.size() + padding * 2.0).max(min);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let a = interact(ui, &resp);
    let draw = rect.shrink(2.0 * a.press);
    let rounding = egui::Rounding::same(6.0);
    let painter = ui.painter();
    let base = lerp_color(color::BG_WIDGET, color::BG_HOVER, a.hover);
    painter.rect_filled(draw, rounding, lerp_color(base, color::BG, 0.35 * a.press));
    painter.rect_stroke(draw, rounding, egui::Stroke::new(1.5_f32, with_alpha(color::ACCENT, a.hover)));
    painter.galley(draw.center() - galley.size() * 0.5, galley, color::TEXT);
    resp
}

/// A small filled pill (rounded, `small` text) that brightens on hover and dips
/// on press — like `primary_button` but with a caller-chosen fill, for badges
/// like "Update available".
pub fn pill(ui: &mut egui::Ui, text: &str, fill: egui::Color32) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::ON_ACCENT);
    let padding = egui::vec2(10.0, 4.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let a = interact(ui, &resp);
    let hi = lerp_color(fill, egui::Color32::WHITE, 0.18);
    let c = lerp_color(lerp_color(fill, hi, a.hover), color::BG, 0.2 * a.press);
    let draw = rect.expand(1.5 * a.hover - 2.0 * a.press);

    let painter = ui.painter();
    painter.rect_filled(draw, egui::Rounding::same(draw.height() * 0.5), c);
    painter.galley(draw.center() - galley.size() * 0.5, galley, color::ON_ACCENT);
    resp
}

/// An animated on/off toggle: a knob that slides while the track eases
/// grey → accent, with a trailing label. Flips `on` when clicked; the returned
/// response reports `.changed()` on a flip.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let (w, h) = (34.0_f32, 18.0_f32);
        let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
        if resp.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        let t = anim_bool(ui, resp.id, *on, 0.12);
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            egui::Rounding::same(h * 0.5),
            lerp_color(color::BG_WIDGET, color::ACCENT, t),
        );
        let r = h * 0.5 - 2.0;
        let cx = egui::lerp((rect.left() + r + 2.0)..=(rect.right() - r - 2.0), t);
        painter.circle_filled(egui::pos2(cx, rect.center().y), r, color::ON_ACCENT);
        ui.add_space(6.0);
        ui.label(label);
        resp
    })
    .inner
}

/// A pill chip whose hover fill fades in and whose accent outline + text tint
/// ease in when `selected` — e.g. the All / Installed / Available filter, where
/// the highlight then slides from chip to chip instead of snapping.
pub fn chip(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color::TEXT);
    let padding = egui::vec2(12.0, 5.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let a = interact(ui, &resp);
    let sel = anim_bool(ui, resp.id.with("sel"), selected, 0.14);

    let rounding = egui::Rounding::same(7.0);
    let painter = ui.painter();
    // Use BG_WIDGET (not BG_ELEVATED, which matches the title-bar fill and would
    // make the hover invisible there) so the chip lifts on any background.
    painter.rect_filled(rect, rounding, with_alpha(color::BG_WIDGET, a.hover.max(sel * 0.6)));
    painter.rect_stroke(rect, rounding, egui::Stroke::new(1.0_f32, with_alpha(color::ACCENT, sel)));
    painter.galley(
        rect.center() - galley.size() * 0.5,
        galley,
        lerp_color(color::TEXT, color::ACCENT, sel),
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

/// What double-clicking a table row does.
#[derive(Debug, Clone, Deserialize)]
pub struct RowAction {
    pub action: Action,
    /// Open the returned view in a new tab rather than replacing the current one.
    #[serde(default)]
    pub open_in_tab: bool,
}

/// One right-click menu entry on a table row. A leaf carries an `action`; an
/// entry with `children` is a submenu (e.g. the Windows "Open path ▸" submenu).
#[derive(Debug, Clone, Deserialize)]
pub struct MenuItem {
    pub label: String,
    #[serde(default)]
    pub action: Option<Action>,
    /// Extra params merged into the call (e.g. `{"via":"explorer"}`).
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
    /// Open the result in a new tab instead of replacing the current view.
    #[serde(default)]
    pub open_in_tab: bool,
    /// Submenu entries; when present, `action` is ignored and this is a submenu.
    #[serde(default)]
    pub children: Vec<MenuItem>,
}

/// A dispatched interaction: an [`Action`] plus any extra params (e.g. the
/// activated row's id) and whether its result view opens in a new tab. Produced
/// by a clicked button, a row context-menu item, or a row double-click.
#[derive(Debug, Clone)]
pub struct Invoke {
    pub action: Action,
    pub args: serde_json::Map<String, Value>,
    pub open_in_tab: bool,
}

impl Invoke {
    /// A plain button click — no extra args, renders in place.
    fn button(action: Action) -> Self {
        Invoke { action, args: serde_json::Map::new(), open_in_tab: false }
    }
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
    /// A table with a header row and string cells. Rows become interactive
    /// (right-click menu + double-click) when `menu` or `on_activate` is set.
    Table {
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
        /// Per-row identity (parallel to `rows`), sent as `id` on a row action.
        #[serde(default)]
        row_ids: Vec<String>,
        /// Right-click context-menu items for a row.
        #[serde(default)]
        menu: Vec<MenuItem>,
        /// What double-clicking a row does.
        #[serde(default)]
        on_activate: Option<RowAction>,
    },
}

/// Render a view; returns the action of a clicked button, if any. `busy` is the
/// action currently in flight (its button shows a spinner).
pub fn render_view(
    ui: &mut egui::Ui,
    view: &View,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
) -> Option<Invoke> {
    let mut clicked = None;
    render_widgets(ui, &view.widgets, inputs, busy, &mut clicked);
    clicked
}

fn render_widgets(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Invoke>,
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
    clicked: &mut Option<Invoke>,
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
            if *multiline {
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text(placeholder.as_str()),
                );
            } else if *password {
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(f32::INFINITY)
                        .hint_text(placeholder.as_str())
                        .password(true),
                );
            } else {
                // Single-line fields get the animated focus border.
                text_field(ui, value, placeholder.as_str(), f32::INFINITY);
            }
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
            let running = busy == Some(action);
            ui.horizontal(|ui| {
                // Module buttons use the shared animated widgets, so a module's UI
                // animates just like the host's chrome.
                let resp = ui
                    .add_enabled_ui(*enabled, |ui| match style {
                        ButtonStyle::Primary => primary_button(ui, text, egui::Vec2::ZERO),
                        ButtonStyle::Default => outline_button(ui, text, egui::Vec2::ZERO),
                    })
                    .inner;
                if resp.clicked() {
                    *clicked = Some(Invoke::button(action.clone()));
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
        Widget::Table { columns, rows, row_ids, menu, on_activate } => {
            render_table(ui, columns, rows, row_ids, menu, on_activate.as_ref(), clicked)
        }
    }
}

/// Render a [`Widget::Table`] as a striped grid inside a scroll area. Plain data
/// tables draw as before; when the module attaches a `menu` or `on_activate`,
/// each row becomes interactive (right-click menu + double-click) and emits an
/// [`Invoke`] carrying that row's id.
#[allow(clippy::too_many_arguments)]
fn render_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_ids: &[String],
    menu: &[MenuItem],
    on_activate: Option<&RowAction>,
    clicked: &mut Option<Invoke>,
) {
    let ncols = columns.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if ncols == 0 {
        return;
    }
    let interactive = !menu.is_empty() || on_activate.is_some();
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
                    for (r, row) in rows.iter().enumerate() {
                        if !interactive {
                            for cell in row {
                                ui.label(cell);
                            }
                            ui.end_row();
                            continue;
                        }
                        // Interactive row: clickable cells unioned into one row
                        // response that carries the context menu + double-click.
                        let mut row_resp: Option<egui::Response> = None;
                        for cell in row {
                            let r = ui.add(egui::Label::new(cell).sense(egui::Sense::click()));
                            row_resp = Some(match row_resp {
                                Some(prev) => prev.union(r),
                                None => r,
                            });
                        }
                        ui.end_row();
                        let Some(row_resp) = row_resp else { continue };
                        let row_resp = row_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        let row_id = row_ids.get(r).cloned().unwrap_or_default();
                        if !menu.is_empty() {
                            let mut picked: Option<Invoke> = None;
                            row_resp.context_menu(|ui| render_row_menu(ui, menu, &row_id, &mut picked));
                            if picked.is_some() {
                                *clicked = picked;
                            }
                        }
                        if let Some(act) = on_activate
                            && row_resp.double_clicked()
                        {
                            let mut args = serde_json::Map::new();
                            args.insert("id".into(), Value::String(row_id.clone()));
                            *clicked = Some(Invoke {
                                action: act.action.clone(),
                                args,
                                open_in_tab: act.open_in_tab,
                            });
                        }
                    }
                });
        });
}

/// Build a table row's right-click menu, recursing into submenus. Writes the
/// chosen [`Invoke`] (with the row's `id` merged in) into `out`.
fn render_row_menu(ui: &mut egui::Ui, items: &[MenuItem], row_id: &str, out: &mut Option<Invoke>) {
    for item in items {
        if !item.children.is_empty() {
            let mut sub: Option<Invoke> = None;
            ui.menu_button(&item.label, |ui| render_row_menu(ui, &item.children, row_id, &mut sub));
            if sub.is_some() {
                *out = sub;
                ui.close_menu();
            }
        } else if let Some(action) = &item.action {
            if ui.button(&item.label).clicked() {
                let mut args = item.args.clone();
                args.insert("id".into(), Value::String(row_id.to_string()));
                *out = Some(Invoke { action: action.clone(), args, open_in_tab: item.open_in_tab });
                ui.close_menu();
            }
        } else {
            ui.label(&item.label);
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
