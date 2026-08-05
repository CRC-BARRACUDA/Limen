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
// Barracuda "EVE-Frontier" amber-HUD palette
//
// Mirrors the platform ui-kit design tokens (Libraries/ui-kit `src/tokens.ts` +
// `theme.css`): a warm amber accent over deep warm-black backgrounds, orange CTA,
// and warm brown-grey neutrals (not a cool slate). Keep these in sync with the
// kit so Limen and the web platform read as one product.
// --------------------------------------------------------------------------- //
pub(crate) mod color {
    use eframe::egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x08, 0x06, 0x04); // deep warm-black page
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x0d, 0x0a, 0x06); // inputs/code, near-black
    pub const BG_WIDGET: Color32 = Color32::from_rgb(0x18, 0x11, 0x09); // buttons/rows
    pub const BG_HOVER: Color32 = Color32::from_rgb(0x2a, 0x1d, 0x0f); // amber-tinted hover
    pub const BORDER: Color32 = Color32::from_rgb(0x3a, 0x2c, 0x1c); // warm amber-dim border
    pub const TEXT: Color32 = Color32::from_rgb(0xc4, 0xb4, 0x98); // warm body text
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8a, 0x78, 0x68); // warm grey
    pub const ACCENT: Color32 = Color32::from_rgb(0xc8, 0x78, 0x30); // amber — primary accent
    pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0xf0, 0xa8, 0x50); // amber, hovered
    pub const ORANGE: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16); // orange — CTA fill
    pub const ORANGE_BRIGHT: Color32 = Color32::from_rgb(0xfb, 0x92, 0x3c); // orange, hovered
    pub const ON_ACCENT: Color32 = Color32::from_rgb(0xff, 0xf5, 0xe8); // text on amber/orange
    pub const SUCCESS: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80); // done / OK green
    pub const WARNING: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24); // warning amber-yellow
    pub const ERROR: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71); // error red
    // The destructive outline's accent. Same red the error text already uses, so
    // "this destroys something" reads the same wherever it appears.
    pub const DANGER_BRIGHT: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71); // red — destructive
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
    let c = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
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
        press: anim_bool(
            ui,
            resp.id.with("press"),
            resp.is_pointer_button_down_on(),
            0.06,
        ),
    }
}

// ---- entrance -------------------------------------------------------------- //

/// Staggered entrance factor (eased 0→1) for the `index`-th item revealed at
/// `start_at`, evaluated at `now`, with `stagger`/`dur` seconds. Requests
/// repaints until it settles; returns 1 instantly when animations are off. Apply
/// it however you like (opacity, slide, …).
pub fn reveal_t(
    ui: &egui::Ui,
    index: usize,
    start_at: f64,
    now: f64,
    stagger: f64,
    dur: f64,
) -> f32 {
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
/// `password` masks the input (for secrets) while keeping the same animation.
pub fn text_field(
    ui: &mut egui::Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    password: bool,
) -> egui::Response {
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
                    .password(password)
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
        egui::Rounding::ZERO,
        egui::Stroke::new(1.0_f32, lerp_color(color::BORDER, color::ACCENT, t)),
    );
    resp
}

/// An animated dropdown — the module `select` widget. A framed field whose
/// border eases grey→`ACCENT` on hover/open, with a chevron that flips as it
/// opens, over a popup whose options highlight on hover and accent the current
/// value. Replaces egui's default `ComboBox` so selects animate like the rest.
pub fn dropdown(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    value: &mut String,
    options: &[String],
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let measure = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    let text_w = options
        .iter()
        .map(|o| measure(o))
        .fold(measure(value), f32::max);
    let pad = egui::vec2(10.0, 7.0);
    let chevron = 16.0;
    let size = egui::vec2(text_w + pad.x * 2.0 + chevron, font.size + pad.y * 2.0);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let popup_id = ui.make_persistent_id(id_source).with("dd_popup");
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    let open = ui.memory(|m| m.is_popup_open(popup_id));
    let a = interact(ui, &resp);
    let open_t = anim_bool(ui, resp.id.with("open"), open, 0.14);
    let border_t = open_t.max(a.hover * 0.6);

    {
        let rounding = egui::Rounding::ZERO;
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            rounding,
            lerp_color(color::BG_ELEVATED, color::BG_WIDGET, a.hover * 0.4),
        );
        painter.rect_stroke(
            rect,
            rounding,
            egui::Stroke::new(1.5_f32, lerp_color(color::BORDER, color::ACCENT, border_t)),
        );
        let galley = painter.layout_no_wrap(value.clone(), font.clone(), color::TEXT);
        painter.galley(
            egui::pos2(rect.left() + pad.x, rect.center().y - galley.size().y * 0.5),
            galley,
            color::TEXT,
        );
        // Chevron: points down when closed, eases to point up as it opens.
        let cx = rect.right() - pad.x - chevron * 0.5;
        let cy = rect.center().y;
        let half = 4.0;
        let dir = 1.0 - 2.0 * open_t; // +1 down → -1 up
        let stroke = egui::Stroke::new(
            1.6_f32,
            lerp_color(color::TEXT_MUTED, color::ACCENT, border_t),
        );
        let l = egui::pos2(cx - half, cy - half * 0.5 * dir);
        let m = egui::pos2(cx, cy + half * 0.5 * dir);
        let r = egui::pos2(cx + half, cy - half * 0.5 * dir);
        painter.line_segment([l, m], stroke);
        painter.line_segment([m, r], stroke);
    }

    egui::popup_below_widget(
        ui,
        popup_id,
        &resp,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui: &mut egui::Ui| {
            ui.set_min_width(rect.width());
            // The menu animates in as it opens: each option fades in, staggered,
            // driven by the same eased open factor.
            let ot = smoothstep(open_t);
            let n = options.len().max(1) as f32;
            for (i, opt) in options.iter().enumerate() {
                let t = ((ot * 1.5) - i as f32 * (0.6 / n)).clamp(0.0, 1.0);
                let selected = value.as_str() == opt.as_str();
                let picked = ui
                    .scope(|ui| {
                        ui.set_opacity(t);
                        dropdown_option(ui, opt, selected)
                    })
                    .inner;
                if picked.clicked() {
                    *value = opt.clone();
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        },
    );
    resp
}

/// One option row inside a [`dropdown`] popup: highlights on hover, accents the
/// current value.
fn dropdown_option(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, color::TEXT);
    let gs = galley.size();
    let pad = egui::vec2(8.0, 5.0);
    let w = ui.available_width().max(gs.x + pad.x * 2.0);
    let (rect, resp) =
        ui.allocate_at_least(egui::vec2(w, gs.y + pad.y * 2.0), egui::Sense::click());
    let a = interact(ui, &resp);
    let painter = ui.painter();
    let rounding = egui::Rounding::ZERO;
    // The current value is marked with a steady blue fill and does not react to
    // hover; every other option highlights on hover.
    if selected {
        painter.rect_filled(rect, rounding, with_alpha(color::ACCENT, 0.30));
    } else if a.hover > 0.0 {
        painter.rect_filled(rect, rounding, with_alpha(color::BG_HOVER, a.hover));
    }
    let col = color::TEXT;
    painter.galley(
        egui::pos2(rect.left() + pad.x, rect.center().y - gs.y * 0.5),
        galley,
        col,
    );
    resp
}

/// The six corner points of a chamfered rectangle — the ui-kit clip-path shape
/// with the **top-right** and **bottom-left** corners cut by `ch`. Shared by the
/// primary and secondary buttons so both read as the same beveled family.
fn chamfer_pts(rect: egui::Rect, ch: f32) -> [egui::Pos2; 6] {
    [
        rect.left_top(),                             // TL (square)
        egui::pos2(rect.right() - ch, rect.top()),   // top edge, before TR cut
        egui::pos2(rect.right(), rect.top() + ch),   // right edge, after TR cut
        rect.right_bottom(),                         // BR (square)
        egui::pos2(rect.left() + ch, rect.bottom()), // bottom edge, before BL cut
        egui::pos2(rect.left(), rect.bottom() - ch), // left edge, after BL cut
    ]
}

/// The ui-kit's primary CTA: a chamfered bar (top-right + bottom-left corners
/// cut) filled with a diagonal deep-orange→bright gradient, a lit top bevel and
/// a thin orange border, its label upper-cased. Brightens on hover, dips on
/// press. `min` is a minimum size (`Vec2::ZERO` to size to the text).
pub fn primary_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    filled_button(ui, text, min, RAMP_PRIMARY)
}

/// A filled CTA's four gradient stops, dark to light.
type Ramp = [egui::Color32; 4];

const RAMP_PRIMARY: Ramp = [
    egui::Color32::from_rgb(0xc2, 0x41, 0x0c),
    egui::Color32::from_rgb(0xea, 0x58, 0x0c),
    color::ORANGE,
    color::ORANGE_BRIGHT,
];

fn filled_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2, ramp: Ramp) -> egui::Response {
    use egui::{Color32, Mesh, Pos2, Shape};

    let font = egui::TextStyle::Button.resolve(ui.style());
    let label = text.to_uppercase();
    let galley = ui
        .painter()
        .layout_no_wrap(label, font, color::ON_ACCENT);
    let padding = egui::vec2(20.0, 9.0);
    let size = (galley.size() + padding * 2.0).max(min);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let a = interact(ui, &resp);
    let draw = rect.expand(1.5 * a.hover - 2.5 * a.press);
    // Chamfer size — cuts the top-right and bottom-left corners.
    let ch = (draw.height() * 0.5).min(draw.width() * 0.5).min(18.0);
    let pts = chamfer_pts(draw, ch);

    // Four-stop gradient (the orange kit ramp, or the red one for danger),
    // shifted brighter on hover and darker on press.
    let shift = 0.18 * a.hover - 0.12 * a.press;
    let stops = ramp;
    let grad = |t: f32| -> Color32 {
        let t = (t + shift).clamp(0.0, 1.0) * 3.0; // into 0..3 across 4 stops
        let i = (t.floor() as usize).min(2);
        lerp_color(stops[i], stops[i + 1], t - i as f32)
    };
    // Diagonal position (mostly left→right, a touch top→bottom) → 135° feel.
    let span = draw.width() + draw.height() * 0.35;
    let shade = |p: Pos2| grad(((p.x - draw.left()) + (p.y - draw.top()) * 0.35) / span);

    let painter = ui.painter();
    // Faint orange bloom behind — expand the *chamfered* outline (not a square
    // rect, which would poke square nubs out past the cut corners).
    let expand_poly = |e: f32| -> Vec<Pos2> {
        let c = draw.center();
        pts.iter()
            .map(|p| {
                let v = *p - c;
                c + v + v.normalized() * e
            })
            .collect()
    };
    for i in 1..=3u8 {
        let e = i as f32 * 2.0;
        let alpha = (0.03 + 0.06 * a.hover) / i as f32;
        painter.add(Shape::convex_polygon(
            expand_poly(e),
            with_alpha(stops[2], alpha),
            egui::Stroke::NONE,
        ));
    }
    // Gradient fill — fan-triangulate the (convex) chamfered hexagon from its
    // centre, colouring every vertex by its diagonal position.
    let mut mesh = Mesh::default();
    mesh.colored_vertex(draw.center(), shade(draw.center()));
    for p in pts {
        mesh.colored_vertex(p, shade(p));
    }
    let n = pts.len() as u32;
    for k in 0..n {
        mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
    }
    painter.add(Shape::mesh(mesh));
    // Thin orange border + a brighter lit top edge.
    painter.add(Shape::closed_line(
        pts.to_vec(),
        egui::Stroke::new(1.0_f32, with_alpha(stops[2], 0.55)),
    ));
    painter.line_segment(
        [pts[0], pts[1]],
        egui::Stroke::new(1.0_f32, with_alpha(lerp_color(stops[3], Color32::WHITE, 0.35), 0.7)),
    );
    painter.galley(
        draw.center() - galley.size() * 0.5,
        galley,
        color::ON_ACCENT,
    );
    resp
}

/// The ui-kit's secondary button: a transparent chamfered outline (top-right +
/// bottom-left corners cut) with a thin amber border and upper-cased muted-amber
/// text. On hover the border and text brighten to `ACCENT` and a faint amber
/// wash fades in; it dips on press. `min` is a minimum size.
pub fn outline_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    outlined_button(
        ui,
        text,
        min,
        OutlineTint {
            border: color::BORDER,
            label: color::TEXT_MUTED,
            hot: color::ACCENT,
            hot_label: color::TEXT,
        },
    )
}

/// The same outline button in red, for an action that destroys something.
///
/// An outline rather than a fill: a destructive action should not be the most
/// eye-catching thing on the screen, and a filled red button competes with the
/// primary one for attention it does not deserve. It says "careful", not "start
/// here".
///
/// Red at rest, not only on hover — the warning is the whole point, and a button
/// that only reveals what it is once the pointer is over it has told you too
/// late. Muted at rest and full red on hover, so it still has somewhere to go.
pub fn danger_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2) -> egui::Response {
    outlined_button(
        ui,
        text,
        min,
        OutlineTint {
            border: lerp_color(color::BORDER, color::DANGER_BRIGHT, 0.55),
            label: lerp_color(color::TEXT_MUTED, color::DANGER_BRIGHT, 0.70),
            hot: color::DANGER_BRIGHT,
            hot_label: color::DANGER_BRIGHT,
        },
    )
}

/// An outline button's four colours: border and label at rest, and the pair they
/// ease to on hover.
struct OutlineTint {
    border: egui::Color32,
    label: egui::Color32,
    hot: egui::Color32,
    hot_label: egui::Color32,
}

/// A transparent chamfered outline that eases from its resting colours to its
/// hover ones.
fn outlined_button(
    ui: &mut egui::Ui,
    text: &str,
    min: egui::Vec2,
    tint: OutlineTint,
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label = text.to_uppercase();
    let galley = ui
        .painter()
        .layout_no_wrap(label, font, tint.label);
    // Vertical padding matches `primary_button` so the two never differ in height
    // when placed side by side.
    let padding = egui::vec2(16.0, 9.0);
    let size = (galley.size() + padding * 2.0).max(min);
    let (rect, resp) = ui.allocate_at_least(size, egui::Sense::click());

    let a = interact(ui, &resp);
    let draw = rect.shrink(2.0 * a.press);
    let ch = (draw.height() * 0.5).min(draw.width() * 0.5).min(12.0);
    let pts = chamfer_pts(draw, ch);

    let painter = ui.painter();
    // Transparent at rest; a faint amber wash fades in on hover.
    painter.add(egui::Shape::convex_polygon(
        pts.to_vec(),
        with_alpha(tint.hot, 0.08 * a.hover),
        egui::Stroke::NONE,
    ));
    // Border eases amber-dim → bright amber on hover.
    painter.add(egui::Shape::closed_line(
        pts.to_vec(),
        egui::Stroke::new(1.0_f32, lerp_color(tint.border, tint.hot, a.hover)),
    ));
    painter.galley(
        draw.center() - galley.size() * 0.5,
        galley,
        lerp_color(tint.label, tint.hot_label, a.hover),
    );
    resp
}

/// A small filled pill (rounded, `small` text) that brightens on hover and dips
/// on press — like `primary_button` but with a caller-chosen fill, for badges
/// like "Update available".
pub fn pill(ui: &mut egui::Ui, text: &str, fill: egui::Color32) -> egui::Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, color::ON_ACCENT);
    let padding = egui::vec2(10.0, 4.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let a = interact(ui, &resp);
    let hi = lerp_color(fill, egui::Color32::WHITE, 0.18);
    let c = lerp_color(lerp_color(fill, hi, a.hover), color::BG, 0.2 * a.press);
    let draw = rect.expand(1.5 * a.hover - 2.0 * a.press);

    let painter = ui.painter();
    painter.rect_filled(draw, egui::Rounding::same(draw.height() * 0.5), c);
    painter.galley(
        draw.center() - galley.size() * 0.5,
        galley,
        color::ON_ACCENT,
    );
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
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, color::TEXT);
    let padding = egui::vec2(12.0, 5.0);
    let (rect, resp) = ui.allocate_at_least(galley.size() + padding * 2.0, egui::Sense::click());

    let a = interact(ui, &resp);
    let sel = anim_bool(ui, resp.id.with("sel"), selected, 0.14);

    let rounding = egui::Rounding::ZERO;
    let painter = ui.painter();
    // Use BG_WIDGET (not BG_ELEVATED, which matches the title-bar fill and would
    // make the hover invisible there) so the chip lifts on any background.
    painter.rect_filled(
        rect,
        rounding,
        with_alpha(color::BG_WIDGET, a.hover.max(sel * 0.6)),
    );
    painter.rect_stroke(
        rect,
        rounding,
        egui::Stroke::new(1.0_f32, with_alpha(color::ACCENT, sel)),
    );
    painter.galley(
        rect.center() - galley.size() * 0.5,
        galley,
        lerp_color(color::TEXT, color::ACCENT, sel),
    );
    resp
}

// --------------------------------------------------------------------------- //
// Custom window chrome (client-side decorations)
//
// The OS titlebar is turned off (`with_decorations(false)`), so the app draws its
// own controls and must also drive move/resize itself. These helpers render the
// minimize/maximize/close glyphs and the edge/corner resize grips.
// --------------------------------------------------------------------------- //

/// A small square button with a painted icon.
///
/// Icons are painted, not typed: the app ships a single font and JetBrains Mono
/// has no trash glyph, so a character would be an empty box. `danger` tints it
/// red on hover, for the ones that destroy something.
pub fn icon_button(ui: &mut egui::Ui, icon: &str, danger: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 26.0), egui::Sense::click());
    let a = interact(ui, &resp);
    let tint = if danger {
        color::DANGER_BRIGHT
    } else {
        color::ACCENT
    };
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::Rounding::same(3.0),
        with_alpha(tint, 0.14 * a.hover),
    );
    let c = lerp_color(color::TEXT_MUTED, tint, a.hover);
    let stroke = egui::Stroke::new(1.3_f32, c);
    let m = rect.center();
    match icon {
        // A lid with a handle, a tapering body, and two slots.
        "trash" => {
            let (w, h) = (5.5_f32, 6.0_f32);
            let lid_y = m.y - h + 1.0;
            painter.hline((m.x - w)..=(m.x + w), lid_y, stroke);
            // Handle.
            painter.hline((m.x - 2.0)..=(m.x + 2.0), lid_y - 2.5, stroke);
            painter.line_segment(
                [egui::pos2(m.x - 2.0, lid_y - 2.5), egui::pos2(m.x - 2.0, lid_y)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(m.x + 2.0, lid_y - 2.5), egui::pos2(m.x + 2.0, lid_y)],
                stroke,
            );
            // Body, narrowing toward the base.
            let bot = m.y + h;
            painter.line_segment(
                [egui::pos2(m.x - w + 0.8, lid_y), egui::pos2(m.x - w + 2.0, bot)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(m.x + w - 0.8, lid_y), egui::pos2(m.x + w - 2.0, bot)],
                stroke,
            );
            painter.hline((m.x - w + 2.0)..=(m.x + w - 2.0), bot, stroke);
            for dx in [-2.0_f32, 2.0] {
                painter.line_segment(
                    [
                        egui::pos2(m.x + dx, lid_y + 2.5),
                        egui::pos2(m.x + dx, bot - 1.5),
                    ],
                    stroke,
                );
            }
        }
        _ => {
            // Unknown icon: a dot, rather than nothing at all.
            painter.circle_filled(m, 2.0, c);
        }
    }
    resp
}

/// A window-control button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WinBtn {
    /// Return to the step that raised this one. Drawn as a left arrow, in the
    /// window-control family so it belongs to the frame rather than the form.
    Back,
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Draw one window-control button (minimize / maximize / restore / close) with a
/// painted glyph and a hover fill — red for close, neutral otherwise.
pub fn window_button(ui: &mut egui::Ui, kind: WinBtn) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(34.0, 26.0), egui::Sense::click());
    let a = interact(ui, &resp);
    let is_close = kind == WinBtn::Close;

    let painter = ui.painter();
    let fill = if is_close { color::ERROR } else { color::BG_HOVER };
    painter.rect_filled(
        rect,
        egui::Rounding::ZERO,
        with_alpha(fill, if is_close { a.hover } else { a.hover * 0.9 }),
    );
    let glyph = if is_close {
        lerp_color(color::TEXT_MUTED, color::ON_ACCENT, a.hover)
    } else {
        lerp_color(color::TEXT_MUTED, color::TEXT, a.hover)
    };
    let stroke = egui::Stroke::new(1.3_f32, glyph);
    let c = rect.center();
    let s = 5.0_f32;
    match kind {
        WinBtn::Minimize => {
            painter.hline((c.x - s)..=(c.x + s), c.y + s, stroke);
        }
        WinBtn::Maximize => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(2.0 * s, 2.0 * s)),
                egui::Rounding::ZERO,
                stroke,
            );
        }
        WinBtn::Restore => {
            // Two overlapping squares — the standard "restore down" glyph.
            let sq = egui::vec2(2.0 * s - 2.0, 2.0 * s - 2.0);
            let front = egui::Rect::from_min_size(egui::pos2(c.x - s, c.y - s + 2.0), sq);
            let back = egui::Rect::from_min_size(egui::pos2(c.x - s + 2.0, c.y - s), sq);
            painter.rect_stroke(back, egui::Rounding::ZERO, stroke);
            painter.rect_filled(front, egui::Rounding::ZERO, color::BG_ELEVATED);
            painter.rect_stroke(front, egui::Rounding::ZERO, stroke);
        }
        WinBtn::Close => {
            painter.line_segment([c + egui::vec2(-s, -s), c + egui::vec2(s, s)], stroke);
            painter.line_segment([c + egui::vec2(s, -s), c + egui::vec2(-s, s)], stroke);
        }
        WinBtn::Back => {
            // A left arrow: shaft plus two barbs.
            painter.line_segment([c + egui::vec2(s, 0.0), c + egui::vec2(-s, 0.0)], stroke);
            painter.line_segment(
                [c + egui::vec2(-s, 0.0), c + egui::vec2(-s + 4.0, -4.0)],
                stroke,
            );
            painter.line_segment(
                [c + egui::vec2(-s, 0.0), c + egui::vec2(-s + 4.0, 4.0)],
                stroke,
            );
        }
    }
    resp
}

/// Invisible edge/corner drag strips that resize the window, since turning off OS
/// decorations also removes the native resize borders. No-op while maximized.
pub fn window_resize_grips(ctx: &egui::Context) {
    use egui::{CursorIcon as Cur, ResizeDirection as Dir};

    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let rect = ctx.screen_rect();
    let b = 6.0_f32; // edge thickness
    let c = 14.0_f32; // corner arm length

    // Only put the grips up when the pointer is actually near an edge. They are
    // foreground areas, so while they exist they sit above the central panel and
    // take its pointer input — which cost the module list its wheel scrolling in
    // a windowed frame, while a maximized one (grips disabled, above) scrolled
    // fine. Nothing is lost by skipping them: they can only be grabbed at an
    // edge anyway, and once a drag starts the OS owns the resize.
    let Some(p) = ctx.input(|i| i.pointer.latest_pos()) else {
        return; // no pointer on screen — nothing could grab a grip
    };
    let near_edge = p.x <= rect.left() + c
        || p.x >= rect.right() - c
        || p.y <= rect.top() + c
        || p.y >= rect.bottom() - c;
    if !near_edge {
        return;
    }
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| {
        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    };
    // Corners last so they take precedence over the edges they overlap.
    let grips = [
        ("rz_n", r(rect.left() + c, rect.top(), rect.right() - c, rect.top() + b), Dir::North, Cur::ResizeNorth),
        ("rz_s", r(rect.left() + c, rect.bottom() - b, rect.right() - c, rect.bottom()), Dir::South, Cur::ResizeSouth),
        ("rz_w", r(rect.left(), rect.top() + c, rect.left() + b, rect.bottom() - c), Dir::West, Cur::ResizeWest),
        ("rz_e", r(rect.right() - b, rect.top() + c, rect.right(), rect.bottom() - c), Dir::East, Cur::ResizeEast),
        ("rz_nw", r(rect.left(), rect.top(), rect.left() + c, rect.top() + c), Dir::NorthWest, Cur::ResizeNorthWest),
        ("rz_ne", r(rect.right() - c, rect.top(), rect.right(), rect.top() + c), Dir::NorthEast, Cur::ResizeNorthEast),
        ("rz_sw", r(rect.left(), rect.bottom() - c, rect.left() + c, rect.bottom()), Dir::SouthWest, Cur::ResizeSouthWest),
        ("rz_se", r(rect.right() - c, rect.bottom() - c, rect.right(), rect.bottom()), Dir::SouthEast, Cur::ResizeSouthEast),
    ];
    for (id, rect, dir, cursor) in grips {
        egui::Area::new(egui::Id::new(id))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let resp = ui.allocate_rect(rect, egui::Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if resp.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                }
            });
    }
}

/// Install **JetBrains Mono** as the app's single face, for both the
/// proportional and the monospace family.
///
/// It carries Latin *and* Cyrillic in one file (1363 codepoints), so English and
/// Ukrainian render in the same typeface. The previous brand pair — Orbitron and
/// Share Tech Mono — had **zero** Cyrillic glyphs between them, so every
/// Ukrainian string silently dropped through to egui's bundled Ubuntu-Light,
/// leaving mixed typefaces on any line that mixed the two alphabets.
///
/// It's inserted at the *front* of each family, with egui's bundled faces kept
/// behind it as fallback — emoji (and the `⚙` it lacks) still resolve there.
fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "JetBrainsMono".to_owned(),
        FontData::from_static(include_bytes!(
            "../../resources/fonts/JetBrainsMono-Regular.ttf"
        )),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "JetBrainsMono".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Apply the Barracuda amber-HUD theme (palette + fonts + sharp chrome) to the
/// whole context.
pub fn apply_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, Rounding, Stroke, TextStyle};

    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
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
    v.window_rounding = Rounding::ZERO;
    v.hyperlink_color = color::ACCENT_BRIGHT;
    v.selection.bg_fill = color::ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, color::ACCENT);

    // Sharp, zero-radius HUD chrome — the ui-kit uses `border-radius: 0` throughout.
    let rounding = Rounding::ZERO;
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

/// Draw L-shaped amber corner brackets just inside `rect` — the ui-kit's framed
/// "panel"/"dialog" accent. `len` is the arm length, `inset` pulls them off the
/// edge, `alpha` sets the amber intensity.
pub fn corner_brackets(
    painter: &egui::Painter,
    rect: egui::Rect,
    len: f32,
    inset: f32,
    alpha: f32,
) {
    let s = egui::Stroke::new(1.5_f32, with_alpha(color::ACCENT, alpha));
    let r = rect.shrink(inset);
    let (tl, tr, bl, br) = (
        r.left_top(),
        r.right_top(),
        r.left_bottom(),
        r.right_bottom(),
    );
    let seg = |painter: &egui::Painter, a, b| painter.line_segment([a, b], s);
    // top-left
    seg(painter, tl, tl + egui::vec2(len, 0.0));
    seg(painter, tl, tl + egui::vec2(0.0, len));
    // top-right
    seg(painter, tr, tr + egui::vec2(-len, 0.0));
    seg(painter, tr, tr + egui::vec2(0.0, len));
    // bottom-left
    seg(painter, bl, bl + egui::vec2(len, 0.0));
    seg(painter, bl, bl + egui::vec2(0.0, -len));
    // bottom-right
    seg(painter, br, br + egui::vec2(-len, 0.0));
    seg(painter, br, br + egui::vec2(0.0, -len));
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
    /// When present, the GUI auto-invokes this action once, right after the view
    /// is shown — with no user click. Lets a module chain a multi-step flow
    /// (e.g. a progress checklist that advances on its own): each step returns a
    /// view carrying the next step's `auto`, and the chain ends at a view without
    /// one.
    #[serde(default)]
    pub auto: Option<AutoAction>,
    /// Show this view as a pop-up over whatever is already on screen, instead of
    /// replacing it. The value is the pop-up's identity.
    ///
    /// The view underneath stays visible and stops responding, so a module can
    /// put settings or a sub-form in front of the user without losing the screen
    /// they came from. Pop-ups stack: one can open another, and the last one
    /// opened is the live one.
    ///
    /// The identity is what lets a pop-up redraw itself. A view whose id is
    /// already open replaces that pop-up in place; a new id opens over it. Without
    /// it, a form that refreshes after every edit — adding a file to a list, say —
    /// would pile up a new pop-up per keystroke.
    #[serde(default)]
    pub modal: Option<String>,
    /// How wide this pop-up wants to be, in points. The host clamps it to the
    /// window — a module cannot know how much room there is, and a text field
    /// asking for infinite width must never be what decides.
    #[serde(default)]
    pub modal_width: Option<f32>,
    /// How tall it may grow before its contents scroll, in points. Also clamped.
    #[serde(default)]
    pub modal_height: Option<f32>,
}

/// The capability + method a button invokes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub capability: String,
    pub method: String,
}

/// An action a [`View`] asks the GUI to invoke automatically once it renders.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoAction {
    pub capability: String,
    pub method: String,
    /// Extra params merged into the call.
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
}

impl AutoAction {
    /// The [`Invoke`] this auto-action dispatches (always in the same tab).
    pub fn into_invoke(self) -> Invoke {
        Invoke {
            dismiss: false,
            confirm: None,
            action: Action {
                capability: self.capability,
                method: self.method,
            },
            args: self.args,
            open_in_tab: false,
        }
    }
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
    /// Ask before running this, exactly as a button can. A destructive action
    /// is no less destructive for being in a menu.
    #[serde(default)]
    pub confirm: Option<Confirm>,
    /// Submenu entries; when present, `action` is ignored and this is a submenu.
    #[serde(default)]
    pub children: Vec<MenuItem>,
}

/// A question the host asks before running an action.
///
/// Carried by the button rather than handled by the module: the module never
/// sees the click unless the answer was yes, so it cannot forget to ask, and
/// every confirmation in the app looks and behaves the same — including the
/// module manager's own "remove this module?".
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Confirm {
    #[serde(default)]
    pub title: String,
    /// What the action happens *to*, drawn in its own frame. The thing at stake
    /// should be unmistakable rather than something to skim past in a sentence.
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub confirm_label: String,
    #[serde(default)]
    pub cancel_label: String,
}

/// A dispatched interaction: an [`Action`] plus any extra params (e.g. the
/// activated row's id) and whether its result view opens in a new tab. Produced
/// by a clicked button, a row context-menu item, or a row double-click.
#[derive(Debug, Clone)]
pub struct Invoke {
    pub action: Action,
    pub args: serde_json::Map<String, Value>,
    pub open_in_tab: bool,
    /// Close the top pop-up instead of calling the module. Set by a button the
    /// module marked `dismiss` — a Cancel that has nothing to cancel remotely,
    /// so it is answered here rather than by a round trip.
    pub dismiss: bool,
    /// Ask this before running it. The host handles the asking.
    pub confirm: Option<Confirm>,
}

/// One bar in a [`Widget::Chart`].
#[derive(Debug, Clone, Deserialize)]
pub struct ChartBar {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: f64,
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
    /// Red: the action destroys something. Available to modules too — a module
    /// that deletes, revokes or wipes should be able to say so in the same
    /// language the host uses for its own destructive actions.
    Danger,
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
    /// A filesystem path input. The user can type a path, drop a file or folder
    /// onto it, or press Browse for the OS picker. Its `id` keys the chosen path
    /// in params, exactly like [`Widget::Text`].
    File {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        default: String,
        /// What this field takes: `"file"`, `"dir"`, or `"file|dir"` for either.
        /// Empty falls back to `directory`, so a module built before this field
        /// existed keeps behaving as it did.
        #[serde(default)]
        accepts: String,
        /// Superseded by `accepts`. Kept so a module compiled against an earlier
        /// SDK still gets a directory picker rather than silently becoming a
        /// file picker.
        #[serde(default)]
        directory: bool,
        /// Label for the Browse button. Supplied by the module so it can be
        /// localized alongside the rest of its view; `ui` stays i18n-free.
        #[serde(default)]
        browse: String,
        /// Label for the *folder* button on a field that takes either kind.
        /// The OS has no "pick a file or a folder" dialog — GTK, the XDG portal
        /// and Windows all treat them as separate — so a dual field offers both.
        /// Left empty, only the file button shows and folders arrive by drag.
        #[serde(default)]
        browse_dir: String,
    },
    /// An animated on/off checkbox; its boolean state is returned in params.
    Checkbox {
        id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        default: bool,
    },
    /// A progress step with an animated status icon — a loading spinner that
    /// morphs into a check when done. `state`: "pending" | "loading" | "done".
    Step {
        #[serde(default)]
        label: String,
        #[serde(default)]
        state: String,
    },
    Button {
        text: String,
        action: Action,
        #[serde(default)]
        style: ButtonStyle,
        /// Whether the button is clickable (default true).
        #[serde(default = "default_true")]
        enabled: bool,
        /// Extra params merged into the call (e.g. a device id/path).
        #[serde(default)]
        args: serde_json::Map<String, Value>,
        /// Open the result view in a new tab instead of replacing this one.
        #[serde(default)]
        open_in_tab: bool,
        /// Close the pop-up this button is in, without calling the module.
        #[serde(default)]
        dismiss: bool,
        /// Ask before running this. The host shows the question and only calls
        /// the module if the answer is yes.
        #[serde(default)]
        confirm: Option<Confirm>,
        /// Draw a painted icon instead of the label, which becomes its tooltip.
        ///
        /// Painted rather than a glyph because the app ships one font and
        /// JetBrains Mono has no trash character — a module asking for "🗑"
        /// would get an empty box on every machine.
        #[serde(default)]
        icon: String,
    },
    Separator,
    /// Inside a [`Widget::Row`], pushes everything after it to the right edge.
    ///
    /// What makes a column of trailing buttons line up when the text before them
    /// does not: without it, each button sits wherever its own row's text ended.
    Spacer,
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
        /// Right-click context-menu items shared by every row.
        #[serde(default)]
        menu: Vec<MenuItem>,
        /// Optional per-row menus (parallel to `rows`); a non-empty entry here
        /// overrides the shared `menu` for that row — so rows can offer
        /// different actions (or none).
        #[serde(default)]
        row_menus: Vec<Vec<MenuItem>>,
        /// What double-clicking a row does.
        #[serde(default)]
        on_activate: Option<RowAction>,
    },
    /// A horizontal bar chart (a value per labelled bar).
    Chart {
        #[serde(default)]
        title: String,
        #[serde(default)]
        data: Vec<ChartBar>,
    },
}

/// Render a view; returns the action of a clicked button, if any. `busy` is the
/// action currently in flight (its button shows a spinner).
/// Draw a module's view.
///
/// `reveal_at` is when this view's entrance begins: top-level widgets fade in
/// one after another from that moment, so switching to a module plays the same
/// staggered entrance the module cards use. Pass `0.0` for "already revealed" —
/// the elapsed time is then effectively infinite and everything draws at full
/// opacity immediately.
pub fn render_view(
    ui: &mut egui::Ui,
    view: &View,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    reveal_at: f64,
) -> Option<Invoke> {
    let mut clicked = None;
    let now = ui.input(|i| i.time);

    // A module that swaps one screen for another — Basic to Advanced, settings
    // to results — gets the entrance again, so the change reads as a change
    // rather than the view blinking into a different arrangement. Keyed on the
    // shape, so a view that merely refreshes in place keeps its clock.
    let shape = view_shape(view);
    let id = ui.id().with("limen_view_shape");
    let changed_at = ui.data_mut(|d| {
        let seen = d.get_temp::<(u64, f64)>(id);
        match seen {
            Some((s, at)) if s == shape => at,
            _ => {
                d.insert_temp(id, (shape, now));
                now
            }
        }
    });
    // Whichever entrance is newer: the tab's, or this screen's own.
    let reveal_at = reveal_at.max(changed_at);

    // Publish this entrance so widgets nested deeper — tables, which clock
    // themselves — can join it instead of snapping in.
    ui.data_mut(|d| d.insert_temp(view_reveal_id(), reveal_at));
    for (i, w) in view.widgets.iter().enumerate() {
        let t = reveal_t(ui, i, reveal_at, now, 0.02, 0.13);
        // Slide as well as fade. A fade alone is easy to miss on a screen that
        // swaps for a similar one — Basic to Advanced keeps the same header and
        // target field — so the motion is what reads as "this changed".
        //
        // The offset is a frame margin, not a `horizontal` wrapper: wrapping
        // turns the child into a *row*, which stands separators on end and puts
        // every label beside its field instead of above it.
        let dx = (1.0 - t) * 18.0;
        egui::Frame::none()
            .outer_margin(egui::Margin {
                left: dx,
                ..Default::default()
            })
            .show(ui, |ui| {
                ui.set_opacity(t);
                render_widget(ui, w, inputs, busy, &mut clicked);
            });
    }
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

/// A step's status icon: a faint hollow circle (pending), a rotating spinner
/// (loading), or a *settled* mark — a green check (`done`), red ✕ (`error`), or
/// amber ⚠ (`warning`). The spinner smoothly morphs into the settled mark. Keyed
/// by `key` (the step label) so the morph persists across the view updates that
/// advance a multi-step flow.
fn step_icon(ui: &mut egui::Ui, key: &str, state: &str) {
    let sz = 18.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(sz, sz), egui::Sense::hover());
    let settled = matches!(state, "done" | "error" | "warning");
    let loading = state == "loading";
    // 0 while loading, ramps to 1 once settled — drives the spinner→mark morph.
    let t = anim_bool(ui, egui::Id::new(("step", key)), settled, 0.35);
    let time = ui.input(|i| i.time) as f32;
    let painter = ui.painter().clone();
    let c = rect.center();
    let r = sz * 0.42;

    // Pending: a faint hollow circle.
    if !loading && !settled {
        painter.circle_stroke(
            c,
            r,
            egui::Stroke::new(1.6f32, with_alpha(color::TEXT_MUTED, 0.45)),
        );
        return;
    }
    // Spinner: a rotating arc, fading out as the settled mark takes over.
    if t < 1.0 {
        let a = 1.0 - t;
        let base = time * 4.5;
        let sweep = std::f32::consts::PI * 1.4;
        let n = 20;
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let ang = base + sweep * (i as f32 / n as f32);
                c + r * egui::vec2(ang.cos(), ang.sin())
            })
            .collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(2.2f32, with_alpha(color::ACCENT, a)),
        ));
        ui.ctx().request_repaint();
    }
    // Settled mark: fades + scales in as `t` rises.
    if t > 0.0 {
        let s = ease_out(t);
        let p = |dx: f32, dy: f32| c + r * egui::vec2(dx, dy) * s;
        match state {
            // Red ✕ in a red ring.
            "error" => {
                let col = with_alpha(color::ERROR, t);
                painter.circle_stroke(c, r, egui::Stroke::new(2.0f32, col));
                painter.line_segment(
                    [p(-0.32, -0.32), p(0.32, 0.32)],
                    egui::Stroke::new(2.4f32, col),
                );
                painter.line_segment(
                    [p(-0.32, 0.32), p(0.32, -0.32)],
                    egui::Stroke::new(2.4f32, col),
                );
            }
            // Amber ⚠ — a triangle with an exclamation.
            "warning" => {
                let col = with_alpha(color::WARNING, t);
                let tri = vec![p(0.0, -0.62), p(-0.58, 0.42), p(0.58, 0.42), p(0.0, -0.62)];
                painter.add(egui::Shape::line(tri, egui::Stroke::new(2.0f32, col)));
                painter.line_segment(
                    [p(0.0, -0.18), p(0.0, 0.14)],
                    egui::Stroke::new(2.2f32, col),
                );
                painter.circle_filled(p(0.0, 0.30), 1.3f32 * s.max(0.4), col);
            }
            // Green check in a green ring.
            _ => {
                let col = with_alpha(color::SUCCESS, t);
                painter.circle_stroke(c, r, egui::Stroke::new(2.0f32, col));
                painter.line_segment(
                    [p(-0.42, 0.02), p(-0.12, 0.34)],
                    egui::Stroke::new(2.4f32, col),
                );
                painter.line_segment(
                    [p(-0.12, 0.34), p(0.46, -0.34)],
                    egui::Stroke::new(2.4f32, col),
                );
            }
        }
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
        Widget::Text {
            id,
            label,
            placeholder,
            multiline,
            default,
            password,
        } => {
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
            } else {
                // Single-line fields — including password/secret ones — get the
                // animated focus border.
                text_field(ui, value, placeholder.as_str(), f32::INFINITY, *password);
            }
        }
        Widget::Select {
            id,
            label,
            options,
            default,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let initial = if default.is_empty() {
                options.first().cloned().unwrap_or_default()
            } else {
                default.clone()
            };
            let value = inputs.entry(id.clone()).or_insert(initial);
            dropdown(ui, id.clone(), value, options);
        }
        Widget::File {
            id,
            label,
            placeholder,
            default,
            accepts,
            directory,
            browse,
            browse_dir,
        } => {
            if !label.is_empty() {
                ui.label(styled(label, LabelStyle::Weak));
            }
            let value = inputs.entry(id.clone()).or_insert_with(|| default.clone());
            let mut path = value.clone();
            let browse_label = if browse.is_empty() {
                "Browse…"
            } else {
                browse
            };
            if file_field(
                ui,
                id,
                &mut path,
                placeholder,
                browse_label,
                browse_dir,
                Accepts::of(accepts, *directory),
            ) {
                *value = path;
            }
        }
        Widget::Checkbox { id, label, default } => {
            let entry = inputs
                .entry(id.clone())
                .or_insert_with(|| default.to_string());
            let mut on = entry.as_str() == "true";
            toggle(ui, &mut on, label);
            *entry = on.to_string();
        }
        Widget::Step { label, state } => {
            ui.horizontal(|ui| {
                step_icon(ui, label, state);
                ui.add_space(8.0);
                let s = match state.as_str() {
                    "loading" => LabelStyle::Strong,
                    "done" => LabelStyle::Normal,
                    _ => LabelStyle::Weak,
                };
                ui.label(styled(label, s));
            });
        }
        Widget::Button {
            text,
            action,
            style,
            enabled,
            args,
            open_in_tab,
            dismiss,
            confirm,
            icon,
        } => {
            let running = busy == Some(action);
            ui.horizontal(|ui| {
                // Module buttons use the shared animated widgets, so a module's UI
                // animates just like the host's chrome.
                let resp = ui
                    .add_enabled_ui(*enabled, |ui| {
                        if !icon.is_empty() {
                            // The label becomes the tooltip: an icon with no
                            // name is a guess, and this one deletes things.
                            return icon_button(ui, icon, matches!(style, ButtonStyle::Danger))
                                .on_hover_text(text);
                        }
                        match style {
                            ButtonStyle::Primary => primary_button(ui, text, egui::Vec2::ZERO),
                            ButtonStyle::Danger => danger_button(ui, text, egui::Vec2::ZERO),
                            ButtonStyle::Default => outline_button(ui, text, egui::Vec2::ZERO),
                        }
                    })
                    .inner;
                if resp.clicked() {
                    *clicked = Some(Invoke {
                        action: action.clone(),
                        args: args.clone(),
                        open_in_tab: *open_in_tab,
                        dismiss: *dismiss,
                        confirm: confirm.clone(),
                    });
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
        // Only meaningful inside a Row, which handles it; on its own it is
        // nothing rather than an error.
        Widget::Spacer => {}
        Widget::Row { children } => {
            // Top-align so a row of mixed-height widgets (e.g. buttons) lines up
            // by their tops instead of being vertically centered.
            ui.horizontal_top(|ui| {
                // Anything after a spacer hugs the right edge. Laid out
                // right-to-left, so the tail is rendered in reverse to come out
                // in the order it was written.
                if let Some(at) = children.iter().position(|c| matches!(c, Widget::Spacer)) {
                    let (head, tail) = children.split_at(at);
                    render_widgets(ui, head, inputs, busy, clicked);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for c in tail[1..].iter().rev() {
                            render_widget(ui, c, inputs, busy, clicked);
                        }
                    });
                    return;
                }
                // Path and text fields ask for all the width there is, so the
                // first one in a row takes it and the rest are left as stubs —
                // three thresholds side by side rendered as one wide box and two
                // small ones. Share the row between them instead.
                let greedy = children
                    .iter()
                    .filter(|c| matches!(c, Widget::Text { .. } | Widget::File { .. }))
                    .count();
                if greedy < 2 {
                    render_widgets(ui, children, inputs, busy, clicked);
                    return;
                }
                let gap = ui.spacing().item_spacing.x;
                // What the fields have left once the labels and buttons beside
                // them have taken their share.
                let fixed: f32 = gap * (children.len().saturating_sub(1)) as f32;
                let share = ((ui.available_width() - fixed) / greedy as f32).max(72.0);
                for c in children {
                    if matches!(c, Widget::Text { .. } | Widget::File { .. }) {
                        // A field's label sits *beside* its box here, so it is
                        // centred against it rather than left on its top edge.
                        // The zero desired height is what keeps that honest:
                        // centring inside a region that was given the pop-up's
                        // whole remaining height puts the pair in the middle of
                        // a band of empty space. The row itself stays
                        // top-aligned, which is what a row of buttons wants.
                        ui.allocate_ui_with_layout(
                            egui::vec2(share, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| render_widget(ui, c, inputs, busy, clicked),
                        );
                    } else {
                        render_widget(ui, c, inputs, busy, clicked);
                    }
                }
            });
        }
        Widget::Table {
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate,
        } => render_table(
            ui,
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate.as_ref(),
            clicked,
        ),
        Widget::Chart { title, data } => render_chart(ui, title, data),
    }
}

/// Render a [`Widget::Chart`] as labelled horizontal bars scaled to the largest
/// value, with the value shown at the end of each row.
fn render_chart(ui: &mut egui::Ui, title: &str, data: &[ChartBar]) {
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(2.0);
    }
    if data.is_empty() {
        return;
    }
    let max = data
        .iter()
        .map(|b| b.value)
        .fold(0.0_f64, f64::max)
        .max(f64::MIN_POSITIVE);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    let label_w = data
        .iter()
        .map(|b| measure(&b.label))
        .fold(0.0_f32, f32::max)
        .clamp(24.0, 200.0);
    let gap = 8.0;
    let val_w = 52.0;
    let row_h = font.size + 8.0;

    for b in data {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_h),
            egui::Sense::hover(),
        );
        // Label.
        ui.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            &b.label,
            font.clone(),
            color::TEXT,
        );
        // Bar track + filled portion.
        let bx = rect.left() + label_w + gap;
        let bar_area = (rect.right() - val_w - gap - bx).max(24.0);
        let bh = row_h * 0.58;
        let track = egui::Rect::from_min_size(
            egui::pos2(bx, rect.center().y - bh / 2.0),
            egui::vec2(bar_area, bh),
        );
        ui.painter().rect_filled(
            track,
            egui::Rounding::ZERO,
            with_alpha(color::BG_WIDGET, 0.55),
        );
        let frac = (b.value / max).clamp(0.0, 1.0) as f32;
        let fill = egui::Rect::from_min_size(track.min, egui::vec2(bar_area * frac, bh));
        ui.painter()
            .rect_filled(fill, egui::Rounding::ZERO, color::ACCENT);
        // Value.
        ui.painter().text(
            egui::pos2(rect.right(), rect.center().y),
            egui::Align2::RIGHT_CENTER,
            fmt_num(b.value),
            font.clone(),
            color::TEXT_MUTED,
        );
    }
}

/// Format a chart value: integers without a decimal, otherwise two places.
fn fmt_num(v: f64) -> String {
    if v.fract().abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Where a [`Widget::File`] parks a pending Browse request: `(widget id, wants
/// a directory)`. The app picks it up after rendering and runs the OS dialog on
/// its own thread — the dialog blocks, and the UI thread only draws.
pub fn browse_request_id() -> egui::Id {
    egui::Id::new("limen_browse_request")
}

/// Marks the frame on which a dropped path was already taken by a field, so a
/// single drop cannot land in two of them at once.
fn drop_claim_id() -> egui::Id {
    egui::Id::new("limen_drop_claimed")
}

/// Marks the frame on which a field already offered itself as the drop target,
/// so only one opens even when several could accept what is being dragged.
fn invite_claim_id() -> egui::Id {
    egui::Id::new("limen_drop_invited")
}

/// The field that was armed on the last frame a drag hovered.
///
/// A drop arrives one frame *after* the hover ends — `dropped_files` is set on
/// the same frame `hovered_files` empties — so by then there is no drag left to
/// test against. This remembers the answer from while it was still visible.
fn armed_field_id() -> egui::Id {
    egui::Id::new("limen_drop_armed")
}

/// The path field the caret is in: `(widget id, wants a directory)`.
///
/// winit reports *that* files are hovering but never *where* — on Wayland at
/// all, and on X11 the XDND coordinates are read and then discarded. With no
/// cursor to test against, a view holding several fields of the same kind would
/// always drop into the first. Focus is the tie-break: click the field you mean,
/// then drop.
fn focused_field_id() -> egui::Id {
    egui::Id::new("limen_path_focus")
}

/// What a [`Widget::File`] will take.
#[derive(Clone, Copy, PartialEq)]
enum Accepts {
    File,
    Dir,
    Either,
}

impl Accepts {
    /// Read the spec, falling back to the older `directory` flag when `accepts`
    /// is absent. Anything unrecognised means a file — the narrower reading, so
    /// a typo cannot quietly widen what a field will swallow.
    fn of(accepts: &str, directory: bool) -> Self {
        let a = accepts.trim().to_lowercase();
        if a.is_empty() {
            return if directory {
                Accepts::Dir
            } else {
                Accepts::File
            };
        }
        match (a.contains("file"), a.contains("dir")) {
            (true, true) => Accepts::Either,
            (false, true) => Accepts::Dir,
            _ => Accepts::File,
        }
    }

    /// Whether a dropped path of this kind belongs here.
    fn takes(self, is_dir: bool) -> bool {
        match self {
            Accepts::Either => true,
            Accepts::Dir => is_dir,
            Accepts::File => !is_dir,
        }
    }
}

/// The overshoot curve: runs past the target and settles back onto it.
///
/// The constant is the usual one — a ~10% overshoot, enough to read as weight
/// without looking like a bug.
fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    let u = t - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// Where an eased value is in its journey.
#[derive(Clone, Copy)]
struct Eased {
    from: f32,
    to: f32,
    start: f64,
}

/// Animate a value toward `target` along [`ease_out_back`].
///
/// egui's own `animate_value_with_time` interpolates monotonically and so cannot
/// overshoot; this keeps its own from/to/start instead. Re-targeting mid-flight
/// starts from wherever the value currently *is*, so a size that changes twice
/// in quick succession does not jump back to the old one first.
fn animate_back(ctx: &egui::Context, id: egui::Id, target: f32, duration: f32) -> f32 {
    if !animations_enabled() {
        return target;
    }
    // Read the clock before taking the data lock — both lock the same Context.
    let now = ctx.input(|i| i.time);
    let state = ctx.data_mut(|d| d.get_temp::<Eased>(id));
    let (value, running) = match state {
        Some(a) => {
            let p = (((now - a.start) as f32) / duration.max(0.001)).clamp(0.0, 1.0);
            (a.from + (a.to - a.from) * ease_out_back(p), p < 1.0)
        }
        // First sight of this value: start where it is asked to be, so opening
        // does not fly in from zero.
        None => (target, false),
    };
    let retarget = state.is_none_or(|a| (a.to - target).abs() > 0.5);
    if retarget {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Eased {
                    from: value,
                    to: target,
                    start: now,
                },
            )
        });
    }
    if running || retarget {
        // Nothing else is driving the clock between frames, so ask.
        ctx.request_repaint();
    }
    value
}

/// Scale an overlay about its own centre as it arrives and leaves.
///
/// Applied to the layer at paint time rather than by changing the box's width,
/// so nothing inside reflows while it animates — text that re-wraps mid-fade
/// reads as a glitch. `t` is the same 0→1 the opacity uses, so the scale, the
/// fade and the rise are one motion.
fn pop_layer(ctx: &egui::Context, layer: egui::LayerId, rect: egui::Rect, t: f32) {
    if !animations_enabled() {
        return;
    }
    let s = 0.92 + 0.08 * ease_out_back(t);
    if (s - 1.0).abs() < 0.0005 {
        return;
    }
    // Scaling about a point: p' = s·p + c·(1 − s) keeps `c` where it is.
    let c = rect.center().to_vec2();
    ctx.set_transform_layer(layer, egui::emath::TSTransform::new(c * (1.0 - s), s));
}

/// What a pop-up window looks like, independent of what is inside it.
pub struct OverlayOpts {
    /// Requested content width in points; clamped to the window.
    pub width: f32,
    /// How tall the content may grow before it scrolls.
    pub max_height: f32,
    /// A title bar with the window controls on the right. `None` leaves the
    /// header to the content — the host's own dialogs centre their titles.
    pub title: Option<String>,
    /// Show the back arrow beside the close control.
    pub back: bool,
    /// Show the close control.
    pub close: bool,
    /// The area this pop-up belongs to — dimmed, blocked, and centred within.
    ///
    /// A tab's content rather than the whole window: the title bar, the tab
    /// strip and every other tab stay usable while it is open, so a pop-up
    /// suspends the thing that raised it and nothing else. `None` falls back to
    /// the whole window.
    pub bounds: Option<egui::Rect>,
}

impl Default for OverlayOpts {
    fn default() -> Self {
        Self {
            width: 460.0,
            max_height: f32::INFINITY,
            title: None,
            back: false,
            close: false,
            bounds: None,
        }
    }
}

/// What the chrome around a pop-up reported this frame.
#[derive(Default)]
pub struct Overlay {
    /// Esc was pressed or the back arrow clicked — go back one step.
    pub back: bool,
    /// The close control was clicked.
    pub close: bool,
    /// It has finished animating away and the caller may let go of its content.
    pub closed: bool,
}

/// A pop-up window: the veil, the frame, the entrance, and nothing about what
/// is inside.
///
/// Every overlay in the app is this plus its own content — a module's view, an
/// "are you sure?", a permission prompt. They were three near-identical copies
/// of the same thirty lines, which is how one of them ended up without the
/// entrance animation the other two had.
///
/// Call it every frame with `open`: an overlay that only exists while it is open
/// cannot animate shut.
pub fn overlay(
    ctx: &egui::Context,
    id: egui::Id,
    open: bool,
    opts: &OverlayOpts,
    add: impl FnOnce(&mut egui::Ui),
) -> Overlay {
    let mut out = Overlay::default();
    let t = if animations_enabled() {
        ctx.animate_bool_with_time(id, open, 0.13)
    } else {
        open as u8 as f32
    };
    if t <= 0.002 {
        out.closed = !open;
        return out;
    }

    // The veil dims what the pop-up belongs to and, sensing clicks across all of
    // it at a higher order than the panels, swallows them — so that area is
    // inert while the pop-up stands. Bounded rather than full-screen: everything
    // outside it, the title bar and the tab strip included, stays live.
    let screen = ctx.screen_rect();
    let bounds = opts.bounds.unwrap_or(screen);
    egui::Area::new(id.with("veil"))
        .order(egui::Order::Middle)
        .fixed_pos(bounds.min)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(bounds.size(), egui::Sense::click_and_drag());
            ui.painter()
                .rect_filled(rect, egui::Rounding::ZERO, with_alpha(color::BG, 0.72 * t));
        });

    // Esc always gets you out — a pop-up you cannot leave is a trap, and a
    // module should not be able to build one.
    if open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        out.back = true;
    }

    let area = egui::Area::new(id.with("box"))
        .order(egui::Order::Foreground)
        // Rises the last few pixels as it arrives, and sinks as it leaves.
        // Centred on its own area, not on the window: anchoring is relative to
        // the screen, so the offset is how far that area's middle sits from it.
        .anchor(
            egui::Align2::CENTER_CENTER,
            [
                bounds.center().x - screen.center().x,
                bounds.center().y - screen.center().y + (1.0 - t) * 12.0,
            ],
        )
        .show(ctx, |ui| {
            ui.set_opacity(t);
            egui::Frame::none()
                .fill(color::BG_ELEVATED)
                .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                .inner_margin(egui::Margin::symmetric(22.0, 18.0))
                .show(ui, |ui| {
                    // Pinned width, always: module text fields ask for
                    // `f32::INFINITY`, harmless in a panel because a panel is
                    // already bounded, but an Area is not — an unpinned pop-up
                    // grows straight past both edges of the window.
                    let room = (bounds.width() - 80.0).max(320.0);
                    let target_w = opts.width.clamp(320.0, 1290.0).min(room);
                    let max_h = opts.max_height.min(bounds.height() - 100.0).max(200.0);
                    let w = animate_back(ctx, id.with("w"), target_w, 0.22);
                    ui.set_width(w);

                    if let Some(title) = &opts.title {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(title).size(16.0).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Right-to-left, so close is placed first and
                                    // ends up outermost — the corner it occupies
                                    // on the window itself. Back sits just inside
                                    // it, with the window controls rather than
                                    // adrift on the other side of the title.
                                    if opts.close
                                        && window_button(ui, WinBtn::Close)
                                            .on_hover_text("Esc")
                                            .clicked()
                                    {
                                        out.close = true;
                                    }
                                    if opts.back && window_button(ui, WinBtn::Back).clicked() {
                                        out.back = true;
                                    }
                                },
                            );
                        });
                        ui.add_space(10.0);
                    }

                    // Height follows the content, measured on the previous frame
                    // and animated toward. Content that changes makes the box
                    // grow rather than appear at a new size — the two read as one
                    // thing changing.
                    let hkey = id.with("content_h");
                    let measured: f32 = ui.data(|d| d.get_temp(hkey)).unwrap_or(0.0);
                    let mut area = egui::ScrollArea::vertical();
                    if measured > 0.0 {
                        let h = animate_back(ctx, id.with("h"), measured.min(max_h), 0.22);
                        area = area.max_height(h).min_scrolled_height(h);
                    } else {
                        // First frame for this content: nothing measured yet, so
                        // let it size itself and record what it came to.
                        area = area.max_height(max_h);
                    }
                    area
                        // Width is the pop-up's decision, so it must not shrink
                        // to content; height is handled above.
                        .auto_shrink([false, measured <= 0.0])
                        .show(ui, |ui| {
                            ui.set_width(w);
                            add(ui);
                            let h = ui.min_rect().height();
                            ui.data_mut(|d| d.insert_temp(hkey, h));
                        });
                });
        });
    pop_layer(ctx, area.response.layer_id, area.response.rect, t);
    out
}

/// What a frame of the pop-up layer produced.
#[derive(Default)]
pub struct ModalOutcome {
    /// A button inside the pop-up was clicked.
    pub invoke: Option<Invoke>,
    /// Go back one step — Esc, the back arrow, or a `dismiss` button. At the
    /// first step that closes the pop-up; deeper in it returns to what raised it.
    pub dismissed: bool,
    /// Close the whole pop-up, however deep it went. The cross means "I am done
    /// here", which at three steps in should not mean "take me back two".
    pub close_all: bool,
    /// The closing animation has finished, so the caller can drop what it kept
    /// only so the pop-up had something to draw on the way out.
    pub closed: bool,
}

/// Draw the module pop-up layer: a module view shown over the screen it came
/// from.
///
/// The chrome is [`overlay`]; this adds what is particular to a module's own
/// window — its requested size, its title bar with back and close, and the
/// widgets it sent. `view` is the one on top, or, once the stack is empty, the
/// one still animating away, which is why the caller keeps it a moment longer
/// than the stack does.
///
/// `depth` shows the back arrow once there is somewhere to go back to.
#[allow(clippy::too_many_arguments)]
pub fn modal_layer(
    ctx: &egui::Context,
    view: Option<&View>,
    open: bool,
    depth: usize,
    bounds: Option<egui::Rect>,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
) -> ModalOutcome {
    let id = egui::Id::new("limen_modal");
    let mut out = ModalOutcome::default();
    let Some(view) = view else {
        // Still has to be driven so the layer can finish closing.
        out.closed = overlay(
            ctx,
            id,
            false,
            &OverlayOpts {
                bounds,
                ..Default::default()
            },
            |_| {},
        )
        .closed;
        return out;
    };

    let opts = OverlayOpts {
        width: view.modal_width.unwrap_or(560.0),
        max_height: view.modal_height.unwrap_or(f32::INFINITY),
        title: Some(view.title.clone()),
        // Back appears only once there is somewhere to go back to; at the first
        // step the cross is the only way out.
        back: depth > 1,
        close: true,
        bounds,
    };

    let chrome = overlay(ctx, id, open, &opts, |ui| {
        // Depth keys the entrance so a second pop-up over the first animates in
        // as its own arrival. Read the clock before taking the data lock —
        // `input` and `data_mut` lock the same Context, and nesting them
        // deadlocks the moment a pop-up opens.
        let now = ui.input(|i| i.time);
        let key = id.with("reveal").with(depth);
        let reveal = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
        if let Some(inv) = render_view(ui, view, inputs, busy, reveal) {
            out.invoke = Some(inv);
        }
    });
    out.dismissed = chrome.back;
    // The cross means "I am done here", which at three steps in should not mean
    // "take me back two".
    out.close_all = chrome.close;
    out.closed = chrome.closed;
    out
}

/// A modal "are you sure?" over the app.
///
/// The chrome is [`overlay`]; this is the question shape on top of it — a
/// centred title, the subject in its own frame, and two buttons.
///
/// Call it every frame with `open`; it animates itself in and out, so the caller
/// keeps whatever it is holding until the answer comes back. Returns
/// `Some(true)` on confirm, `Some(false)` on cancel, `None` while it is open,
/// closing, or absent.
///
/// `which` keys the dialog: the host's own questions and a module's are the same
/// dialog, but two of them sharing one id would fight over the screen.
///
/// `subject` is what the action will happen *to* — drawn in its own frame, so
/// the thing at stake is unmistakable rather than something to skim past inside
/// a sentence. Keep passing it while the dialog closes, or it will blink empty
/// on the way out.
#[allow(clippy::too_many_arguments)]
pub fn confirm_dialog(
    ctx: &egui::Context,
    which: &str,
    open: bool,
    title: &str,
    subject: Option<&str>,
    confirm_label: &str,
    cancel_label: &str,
    bounds: Option<egui::Rect>,
) -> Option<bool> {
    let mut answer = None;
    let opts = OverlayOpts {
        width: 420.0,
        bounds,
        ..Default::default()
    };
    overlay(
        ctx,
        egui::Id::new("limen_confirm").with(which),
        open,
        &opts,
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(title).size(17.0).strong());
                if let Some(s) = subject {
                    ui.add_space(14.0);
                    egui::Frame::none()
                        .fill(color::BG_WIDGET)
                        .stroke(egui::Stroke::new(1.0_f32, color::BORDER))
                        .inner_margin(egui::Margin::symmetric(16.0, 9.0))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(s).strong());
                        });
                }
                ui.add_space(18.0);

                // Both buttons the same width, so the pair can be centred by
                // arithmetic rather than by hoping a layout does it.
                const BW: f32 = 150.0;
                let gap = ui.spacing().item_spacing.x;
                let pair = BW * 2.0 + gap;
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - pair) / 2.0).max(0.0));
                    if danger_button(ui, confirm_label, egui::vec2(BW, 0.0)).clicked() {
                        answer = Some(true);
                    }
                    if primary_button(ui, cancel_label, egui::vec2(BW, 0.0)).clicked() {
                        answer = Some(false);
                    }
                });
            });
        },
    );
    // Only a live dialog can be answered; a closing one is just an animation.
    open.then_some(answer).flatten()
}

/// The host's consent dialog: the same pop-up window as everything else, with
/// the permission question inside it.
///
/// Content is a closure because what needs consenting to varies — a module name,
/// the permissions it declares — while the frame, the veil and the timing should
/// not. Returns `true` once it has finished animating away, so the caller can
/// let go of whatever it was drawing.
pub fn consent_dialog(
    ctx: &egui::Context,
    open: bool,
    bounds: Option<egui::Rect>,
    contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let opts = OverlayOpts {
        width: 460.0,
        bounds,
        ..Default::default()
    };
    overlay(
        ctx,
        egui::Id::new("limen_consent"),
        open,
        &opts,
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(crate::i18n::t("perm.title"))
                        .size(17.0)
                        .strong(),
                );
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            contents(ui);
        },
    )
    .closed
}

/// A path field: type it, drop a file or folder on it, or Browse for it.
///
/// Returns `true` when `path` changed. Drops are matched against this field's
/// rect, so a view with several path inputs routes each drop to the one under
/// the cursor.
#[allow(clippy::too_many_arguments)]
fn file_field(
    ui: &mut egui::Ui,
    id: &str,
    path: &mut String,
    placeholder: &str,
    browse_label: &str,
    browse_dir_label: &str,
    accepts: Accepts,
) -> bool {
    let ctx = ui.ctx().clone();
    let frame = ctx.frame_nr();
    // What is being dragged over the window: `Some(true)` a directory,
    // `Some(false)` a file, `None` nothing (or a drag whose path the platform
    // withheld, which we cannot classify and therefore do not invite).
    let dragged_is_dir = ctx.input(|i| {
        i.raw
            .hovered_files
            .iter()
            .find_map(|f| f.path.as_ref())
            .map(|p| p.is_dir())
    });
    // Every field that could take this thing opens, so all the valid targets are
    // visible at once — dragging a file over a directory field still invites
    // nothing, and vice versa.
    let suits_me = dragged_is_dir.is_some_and(|d| accepts.takes(d));
    // Where the pointer is, asked of the OS: winit reports the drag but never
    // its position. `None` where that query is unavailable — a Wayland session,
    // macOS — and the caret becomes the tie-break instead.
    let drag_pos = suits_me.then(|| crate::cursor::drag_pos(&ctx)).flatten();
    let mut changed = false;

    let row = ui.horizontal(|ui| {
        // Lay the button out first, right-aligned, and give the field whatever
        // is left. `outline_button`'s size argument is only a *minimum*, so a
        // long label — "Choose folder…", or any translation of it — grows the
        // button; reserving a fixed width for it pushed the row off-screen.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // A field that takes either kind offers both dialogs, because the
            // OS has no single one that picks a file *or* a folder. Rendered
            // right-to-left, so the folder button sits outermost.
            if matches!(accepts, Accepts::Either)
                && !browse_dir_label.is_empty()
                && outline_button(ui, browse_dir_label, egui::Vec2::ZERO).clicked()
            {
                ctx.data_mut(|d| d.insert_temp(browse_request_id(), (id.to_string(), true)));
            }
            let wants_dir = matches!(accepts, Accepts::Dir);
            if outline_button(ui, browse_label, egui::Vec2::ZERO).clicked() {
                ctx.data_mut(|d| d.insert_temp(browse_request_id(), (id.to_string(), wants_dir)));
            }
            let field_w = ui.available_width().max(80.0);
            let before = path.clone();
            let resp = text_field(ui, path, placeholder, field_w, false);
            changed |= *path != before;

            // Remember where the caret is, so a drag with no cursor to follow
            // still knows which of several same-kind fields the user means.
            if resp.has_focus() {
                ctx.data_mut(|d| {
                    d.insert_temp(
                        focused_field_id(),
                        (id.to_string(), matches!(accepts, Accepts::Dir)),
                    )
                });
            }
        });
    });

    // A collision box around the row, tested against the cursor: enter it and
    // this field becomes the drop zone, leave and it goes back to being a text
    // box. The box follows the zone as it grows — sized from *last* frame's
    // openness — so moving down into the expanded area keeps it open instead of
    // falling straight back out of the rect that opened it.
    const GROW: f32 = 44.0;
    let r = row.response.rect;
    let was_open = ctx
        .data(|d| d.get_temp::<f32>(egui::Id::new(("dropopen", id))))
        .unwrap_or(0.0);
    let hit = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() + GROW * was_open))
        .expand(6.0);

    // Only the field the cursor is actually in opens. Without a cursor to test
    // — a Wayland session, macOS — fall back to showing every candidate and
    // arming the focused one, since otherwise nothing would open at all.
    let focused = ctx.data(|d| d.get_temp::<(String, bool)>(focused_field_id()));
    let (want_open, want_armed) = match drag_pos {
        Some(pos) => {
            let inside = suits_me && hit.contains(pos);
            (inside, inside)
        }
        None => {
            let mine = focused.as_ref().is_some_and(|(f, _)| f == id);
            let elsewhere = focused
                .as_ref()
                .is_some_and(|(f, dir)| f != id && accepts.takes(*dir));
            let taken = ctx.data(|d| d.get_temp::<u64>(invite_claim_id())) == Some(frame);
            (suits_me, suits_me && (mine || (!elsewhere && !taken)))
        }
    };
    if want_armed {
        ctx.data_mut(|d| {
            d.insert_temp(invite_claim_id(), frame);
            // The drop itself arrives a frame later, by which point
            // `hovered_files` is empty and nothing can be recomputed — so
            // remember who was armed while we still know.
            d.insert_temp(armed_field_id(), id.to_string());
        });
    }
    let open = anim_bool(ui, egui::Id::new(("dropzone", id)), want_open, 0.16);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("dropopen", id)), open));

    // While open, the field *is* the drop target: it grows downward and a cross
    // marks the middle. Painted over the row rather than swapped for it, so the
    // layout below eases apart instead of jumping.
    if open > 0.0 {
        let extra = GROW * open;
        ui.add_space(extra);
        let zone = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() + extra));
        let armed = anim_bool(ui, egui::Id::new(("dropzonelit", id)), want_armed, 0.16);

        let p = ui.painter();
        let rounding = egui::Rounding::same(3.0);
        // ACCENT is the only colour used — no tinted wash underneath it. Washing
        // one amber over another is what muddied this into brown; the zone keeps
        // the ordinary input background and speaks entirely through its outline.
        let k = (0.4 + 0.6 * armed) * open;
        // Opaque at full open, so the row underneath is covered rather than
        // showing through the zone.
        p.rect_filled(zone, rounding, with_alpha(color::BG_ELEVATED, open));
        p.rect_stroke(
            zone,
            rounding,
            egui::Stroke::new(1.0 + 0.5 * armed, with_alpha(color::ACCENT, k)),
        );
        // The cross, drawn rather than typed — no glyph to depend on.
        let c = zone.center();
        let arm = 13.0 * open;
        let stroke = egui::Stroke::new(1.5 + 0.5 * armed, with_alpha(color::ACCENT, k));
        p.line_segment([c - egui::vec2(arm, 0.0), c + egui::vec2(arm, 0.0)], stroke);
        p.line_segment([c - egui::vec2(0.0, arm), c + egui::vec2(0.0, arm)], stroke);
    }

    // The drop lands a frame after the hover ends, so it cannot be matched
    // against a live drag — the field armed on the last hovered frame takes it,
    // provided the kind still agrees.
    if let Some(dropped) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
        let armed_id = ctx.data(|d| d.get_temp::<String>(armed_field_id()));
        let taken = ctx.data(|d| d.get_temp::<u64>(drop_claim_id())) == Some(frame);
        if !taken && armed_id.as_deref() == Some(id) && accepts.takes(dropped.is_dir()) {
            *path = dropped.display().to_string();
            changed = true;
            ctx.data_mut(|d| d.insert_temp(drop_claim_id(), frame));
        }
    }
    changed
}

/// A fingerprint of a view's *shape* — its title and the kinds of widget it is
/// made of, not their contents.
///
/// This is what decides whether a view has become a different screen or merely
/// refreshed. Switching a module between Basic and Advanced adds controls and so
/// changes the shape; a progress checklist ticking through its steps swaps a
/// label's text and keeps it, which must not count, or the screen would restart
/// its entrance on every tick and strobe.
fn view_shape(view: &View) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.title.hash(&mut h);
    view.widgets.len().hash(&mut h);
    for w in &view.widgets {
        std::mem::discriminant(w).hash(&mut h);
    }
    h.finish()
}

/// Where [`render_view`] parks the current entrance's start time, for widgets
/// that clock themselves rather than taking a stagger index from the top level.
fn view_reveal_id() -> egui::Id {
    egui::Id::new("limen_view_reveal")
}

/// When this table's entrance started, and how far row `r` is into it.
///
/// The clock lives in egui memory keyed by the table's shape, so a table whose
/// row count changes (results arriving, a page turned) is a *new* id and
/// restarts — while a view that merely refreshes in place, like a progress list
/// ticking through its steps, keeps its clock and does not re-animate.
///
/// The stagger index is capped: a 500-row table staggered per row would take
/// half a minute to finish appearing. Past the cap every remaining row shares
/// the last slot, so it reads as a cascade without ever outstaying it.
fn row_reveal(ui: &mut egui::Ui, cols: &[String], nrows: usize, r: usize) -> f32 {
    const CAP: usize = 18;
    let id = egui::Id::new(("tablereveal", cols.len(), cols.first().cloned(), nrows));
    let now = ui.input(|i| i.time);
    // When this table's shape was first seen...
    let shape = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    // ...and when the view around it last began an entrance. The later of the
    // two wins, so the table replays both when its contents change *and* when
    // you switch back to a tab that was already open — but sits still while a
    // view merely refreshes in place.
    let view = ui
        .data(|d| d.get_temp::<f64>(view_reveal_id()))
        .unwrap_or(0.0);
    reveal_t(ui, r.min(CAP), shape.max(view), now, 0.010, 0.14)
}

/// Render a [`Widget::Table`]. A plain data table is a striped grid; when the
/// module attaches a `menu` or `on_activate`, rows become interactive — the
/// whole row (full width, not just its text) is clickable, highlights on hover,
/// and emits an [`Invoke`] carrying that row's id on right-click / double-click.
#[allow(clippy::too_many_arguments)]
fn render_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_ids: &[String],
    menu: &[MenuItem],
    row_menus: &[Vec<MenuItem>],
    on_activate: Option<&RowAction>,
    clicked: &mut Option<Invoke>,
) {
    let ncols = columns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if ncols == 0 {
        return;
    }
    let has_row_menus = row_menus.iter().any(|m| !m.is_empty());
    if menu.is_empty() && !has_row_menus && on_activate.is_none() {
        render_plain_table(ui, columns, rows, ncols);
    } else {
        render_interactive_table(
            ui,
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate,
            clicked,
            ncols,
        );
    }
}

/// A non-interactive table: a striped grid of labels inside a scroll area.
fn render_plain_table(ui: &mut egui::Ui, columns: &[String], rows: &[Vec<String>], ncols: usize) {
    let id = ui.make_persistent_id(("limen_table", ncols, rows.len()));
    egui::ScrollArea::horizontal().id_source(id).show(ui, |ui| {
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
                    let t = row_reveal(ui, columns, rows.len(), r);
                    for cell in row {
                        ui.scope(|ui| {
                            ui.set_opacity(t);
                            ui.label(cell);
                        });
                    }
                    ui.end_row();
                }
            });
    });
}

/// An interactive table: each row is one full-width clickable band that zebra-
/// stripes, highlights (with an accent edge) on hover, and dims slightly on
/// press — so it reads as clickable — carrying the row context menu + double
/// click. Laid out manually (not a `Grid`) so a row spans the whole width.
#[allow(clippy::too_many_arguments)]
fn render_interactive_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_ids: &[String],
    menu: &[MenuItem],
    row_menus: &[Vec<MenuItem>],
    on_activate: Option<&RowAction>,
    clicked: &mut Option<Invoke>,
    ncols: usize,
) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let col_gap = 18.0;
    let pad_x = 8.0;
    let pad_y = 5.0;

    // Column widths = the widest of the header and every cell in that column.
    let measure = |ui: &egui::Ui, s: &str| -> f32 {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    let mut widths = vec![0f32; ncols];
    for (c, col) in columns.iter().enumerate() {
        widths[c] = widths[c].max(measure(ui, col));
    }
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(measure(ui, cell));
        }
    }
    let content_w =
        widths.iter().sum::<f32>() + col_gap * ncols.saturating_sub(1) as f32 + pad_x * 2.0;
    let row_h = font.size + pad_y * 2.0;
    let rounding = egui::Rounding::ZERO;

    let id = ui.make_persistent_id(("limen_itable", ncols, rows.len()));
    egui::ScrollArea::horizontal().id_source(id).show(ui, |ui| {
        // Stretch to the viewport so a row's highlight spans the full width.
        let table_w = content_w.max(ui.available_width());

        // Header.
        let (hrect, _) = ui.allocate_exact_size(egui::vec2(table_w, row_h), egui::Sense::hover());
        let mut hx = hrect.left() + pad_x;
        for (c, col) in columns.iter().enumerate() {
            ui.painter().text(
                egui::pos2(hx, hrect.center().y),
                egui::Align2::LEFT_CENTER,
                col,
                font.clone(),
                color::TEXT_MUTED,
            );
            hx += widths.get(c).copied().unwrap_or(0.0) + col_gap;
        }
        ui.painter().hline(
            hrect.x_range(),
            hrect.bottom(),
            egui::Stroke::new(1.0_f32, color::BORDER),
        );

        // Rows.
        for (r, row) in rows.iter().enumerate() {
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(table_w, row_h), egui::Sense::click());
            let it = interact(ui, &resp);
            // Fade this row in on its slot of the table's entrance. Set on the
            // Ui so the painted chrome (zebra, hover, separators) and the cell
            // text below both pick it up.
            let row_t = row_reveal(ui, columns, rows.len(), r);
            ui.set_opacity(row_t);

            // Zebra base, then a hover highlight faded in by the eased factor,
            // with a small accent bar on the leading edge and a press tint.
            if r % 2 == 1 {
                ui.painter()
                    .rect_filled(rect, rounding, with_alpha(color::BG_WIDGET, 0.30));
            }
            if it.hover > 0.0 {
                let tint = lerp_color(color::BG_HOVER, color::ACCENT, it.press * 0.18);
                ui.painter()
                    .rect_filled(rect, rounding, with_alpha(tint, it.hover));
                let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2.5, rect.height()));
                ui.painter().rect_filled(
                    bar,
                    egui::Rounding::ZERO,
                    with_alpha(color::ACCENT, it.hover),
                );
            }

            // Cells.
            let mut x = rect.left() + pad_x;
            for (c, cell) in row.iter().enumerate() {
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    cell,
                    font.clone(),
                    color::TEXT,
                );
                x += widths.get(c).copied().unwrap_or(0.0) + col_gap;
            }

            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let row_id = row_ids.get(r).cloned().unwrap_or_default();
            // A non-empty per-row menu overrides the shared one for this row.
            let this_menu: &[MenuItem] = match row_menus.get(r) {
                Some(m) if !m.is_empty() => m,
                _ => menu,
            };
            if !this_menu.is_empty() {
                let mut picked: Option<Invoke> = None;
                resp.context_menu(|ui| render_row_menu(ui, this_menu, &row_id, &mut picked));
                if picked.is_some() {
                    *clicked = picked;
                }
            }
            if let Some(act) = on_activate
                && resp.double_clicked()
            {
                let mut args = serde_json::Map::new();
                args.insert("id".into(), Value::String(row_id.clone()));
                *clicked = Some(Invoke {
                    dismiss: false,
                    // A double-click is not a menu entry; nothing to ask.
                    confirm: None,
                    action: act.action.clone(),
                    args,
                    open_in_tab: act.open_in_tab,
                });
            }
        }
    });
}

/// Build a table row's right-click menu, recursing into submenus. Writes the
/// chosen [`Invoke`] (with the row's `id` merged in) into `out`.
fn render_row_menu(ui: &mut egui::Ui, items: &[MenuItem], row_id: &str, out: &mut Option<Invoke>) {
    for item in items {
        if !item.children.is_empty() {
            let mut sub: Option<Invoke> = None;
            ui.menu_button(&item.label, |ui| {
                render_row_menu(ui, &item.children, row_id, &mut sub)
            });
            if sub.is_some() {
                *out = sub;
                ui.close_menu();
            }
        } else if let Some(action) = &item.action {
            if ui.button(&item.label).clicked() {
                let mut args = item.args.clone();
                args.insert("id".into(), Value::String(row_id.to_string()));
                *out = Some(Invoke {
                    dismiss: false,
                    // A menu entry can carry a question just as a button can.
                    confirm: item.confirm.clone(),
                    action: action.clone(),
                    args,
                    open_in_tab: item.open_in_tab,
                });
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
    Some((
        s[1..close].to_string(),
        tail[1..end].to_string(),
        close + 1 + end + 1,
    ))
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
            ui.label(
                egui::RichText::new(rest)
                    .strong()
                    .size(15.0)
                    .color(color::TEXT),
            );
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(rest)
                    .strong()
                    .size(17.0)
                    .color(color::TEXT),
            );
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

/// Every input id a view declares.
///
/// A widget's `default` only seeds its entry the first time it is drawn, so a
/// pop-up whose fields are left behind when it closes would reopen showing what
/// was typed and abandoned — a Cancel that cancels nothing. The caller forgets
/// these when the pop-up goes, and the next open takes the module's values again.
pub fn widget_ids(view: &View) -> Vec<String> {
    fn walk(widgets: &[Widget], out: &mut Vec<String>) {
        for w in widgets {
            match w {
                Widget::Text { id, .. }
                | Widget::Select { id, .. }
                | Widget::File { id, .. }
                | Widget::Checkbox { id, .. } => out.push(id.clone()),
                Widget::Row { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&view.widgets, &mut out);
    out
}

fn collect_ids(
    widgets: &[Widget],
    inputs: &HashMap<String, String>,
    map: &mut serde_json::Map<String, Value>,
) {
    for w in widgets {
        match w {
            Widget::Text { id, .. } | Widget::Select { id, .. } | Widget::File { id, .. } => {
                if let Some(v) = inputs.get(id) {
                    map.insert(id.clone(), Value::String(v.clone()));
                }
            }
            Widget::Checkbox { id, default, .. } => {
                let on = inputs.get(id).map(|s| s == "true").unwrap_or(*default);
                map.insert(id.clone(), Value::Bool(on));
            }
            Widget::Row { children } => collect_ids(children, inputs, map),
            _ => {}
        }
    }
}

/// The component gallery — the UI Kit shown in the Developer window, and the
/// standardized source of truth for module widget styling.
pub fn render_demo_ui(ui: &mut egui::Ui, inputs: &mut HashMap<String, String>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
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
                let _ = ui.add(
                    egui::Button::new(egui::RichText::new("Primary").color(color::ON_ACCENT))
                        .fill(color::ACCENT),
                );
                let _ = ui.button("Default");
            });

            ui.add_space(10.0);
            ui.label(styled("Inputs", LabelStyle::Strong));
            let text = inputs
                .entry("demo.text".into())
                .or_insert_with(|| "editable".into());
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(240.0)
                    .hint_text("single line"),
            );
            let sel = inputs
                .entry("demo.select".into())
                .or_insert_with(|| "one".into());
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
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(56.0, 34.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, egui::Rounding::ZERO, c);
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

    /// The UI font must actually parse and lay out — egui panics on a malformed
    /// face when it builds the atlas, so one headless frame covering both
    /// alphabets and both families proves JetBrains Mono loads and serves them.
    /// Unlike the Latin-only faces it replaced, the Cyrillic here comes from the
    /// font itself rather than egui's bundled fallback.
    #[test]
    fn ui_font_loads_and_lays_out_both_alphabets() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Limen HUD 0123");
                ui.monospace("path/to/mod.rs");
                ui.label("Заборонене ПЗ — Встановлені модулі");
            });
        });
    }

    /// `modal` and `dismiss` are opt-in: every view built before they existed
    /// must keep replacing the screen and calling the module, as it always did.
    #[test]
    fn views_are_not_pop_ups_unless_they_say_so() {
        let plain: View = serde_json::from_str(r#"{"title":"S","widgets":[]}"#).unwrap();
        assert!(plain.modal.is_none());

        let popup: View =
            serde_json::from_str(r#"{"title":"S","widgets":[],"modal":"settings"}"#).unwrap();
        assert_eq!(popup.modal.as_deref(), Some("settings"));
        // Size is optional: a pop-up that does not ask gets the host's default.
        assert!(popup.modal_width.is_none());
        let sized: View = serde_json::from_str(
            r#"{"title":"S","widgets":[],"modal":"s","modal_width":860.0}"#,
        )
        .unwrap();
        assert_eq!(sized.modal_width, Some(860.0));

        let btn: Widget = serde_json::from_str(
            r#"{"kind":"button","text":"Cancel","action":{"capability":"c","method":"m"},"dismiss":true}"#,
        )
        .unwrap();
        match btn {
            Widget::Button { dismiss, .. } => assert!(dismiss),
            _ => panic!("expected a button"),
        }
        let old: Widget = serde_json::from_str(
            r#"{"kind":"button","text":"Go","action":{"capability":"c","method":"m"}}"#,
        )
        .unwrap();
        match old {
            Widget::Button { dismiss, .. } => assert!(!dismiss, "a plain button still calls out"),
            _ => panic!("expected a button"),
        }
    }

    /// Every overlay in the app arrives the same way — module pop-ups, the
    /// "are you sure?" and the permission prompt — and each has to survive the
    /// frames where it is part-way there, which is when a scaled layer and a
    /// half-faded frame are being drawn together.
    #[test]
    fn every_overlay_draws_through_its_entrance_and_exit() {
        let view: View = serde_json::from_str(
            r#"{"title":"Settings","modal":"s","modal_width":600.0,
                "widgets":[{"kind":"label","text":"body"},
                           {"kind":"button","text":"Go","action":{"capability":"c","method":"m"}}]}"#,
        )
        .unwrap();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };

        /// One overlay, drawn open or closed.
        type Draw = Box<dyn Fn(&egui::Context, bool)>;
        let draws: Vec<Draw> = vec![
            Box::new(move |ctx: &egui::Context, open: bool| {
                let mut inputs = HashMap::new();
                let v = if open { Some(&view) } else { None };
                let _ = modal_layer(ctx, v, open, 1, None, &mut inputs, None);
            }),
            Box::new(|ctx: &egui::Context, open: bool| {
                let _ = confirm_dialog(ctx, "t", open, "Remove?", Some("x"), "Yes", "No", None);
            }),
            Box::new(|ctx: &egui::Context, open: bool| {
                let _ = consent_dialog(ctx, open, None, |ui| {
                    ui.label("wants to run");
                });
            }),
        ];

        for draw in draws {
            let ctx = egui::Context::default();
            install_fonts(&ctx);
            // Closed first: the layer is driven every frame in the app, and
            // without those frames egui's animation would start *at* its target
            // and the entrance would never happen.
            for _ in 0..3 {
                let _ = ctx.run(input.clone(), |ctx| draw(ctx, false));
            }
            for _ in 0..20 {
                let _ = ctx.run(input.clone(), |ctx| draw(ctx, true));
            }
            for _ in 0..20 {
                let _ = ctx.run(input.clone(), |ctx| draw(ctx, false));
            }
        }
    }

    /// All three overlays are the same window with different contents, so the
    /// chrome must behave identically in each — including Esc, which is the one
    /// way out that must never depend on what a pop-up chose to draw.
    #[test]
    fn esc_leaves_every_overlay() {
        let input = |esc: bool| {
            let mut i = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            if esc {
                i.events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            i
        };
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let opts = OverlayOpts {
            title: Some("A window".into()),
            close: true,
            ..Default::default()
        };
        // Open it, then press Esc.
        for _ in 0..3 {
            let _ = ctx.run(input(false), |ctx| {
                overlay(ctx, egui::Id::new("x"), false, &opts, |_| {});
            });
        }
        let mut asked_back = false;
        for i in 0..10 {
            let _ = ctx.run(input(i == 5), |ctx| {
                let o = overlay(ctx, egui::Id::new("x"), true, &opts, |ui| {
                    ui.label("body");
                });
                if o.back {
                    asked_back = true;
                }
            });
        }
        assert!(asked_back, "Esc must always be a way out");
    }

    /// A pop-up belongs to the area that raised it, not to the window. The veil
    /// must cover exactly that area — otherwise it dims and blocks the title bar
    /// and the tab strip, and the app becomes unusable while a pop-up is open in
    /// one tab.
    #[test]
    fn a_pop_up_dims_only_the_area_it_belongs_to() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        // The content area below a title bar and a tab strip.
        let content = egui::Rect::from_min_max(egui::pos2(0.0, 120.0), egui::pos2(1280.0, 800.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let opts = OverlayOpts {
            title: Some("Scan settings".into()),
            bounds: Some(content),
            ..Default::default()
        };
        for _ in 0..3 {
            let _ = ctx.run(input.clone(), |ctx| {
                overlay(ctx, egui::Id::new("b"), false, &opts, |_| {});
            });
        }
        for _ in 0..30 {
            let _ = ctx.run(input.clone(), |ctx| {
                overlay(ctx, egui::Id::new("b"), true, &opts, |ui| {
                    ui.label("body");
                });
            });
        }

        let veil = ctx
            .memory(|m| m.area_rect(egui::Id::new("b").with("veil")))
            .expect("the veil drew");
        assert!(
            veil.min.y >= content.min.y - 0.5,
            "the veil must start below the chrome, not at the top of the window: {veil:?}"
        );
        assert!(veil.height() <= content.height() + 0.5);

        // ...and the box is centred on that area, not on the window.
        let boxed = ctx
            .memory(|m| m.area_rect(egui::Id::new("b").with("box")))
            .expect("the box drew");
        assert!(
            (boxed.center().y - content.center().y).abs() < 2.0,
            "centred on its area ({}), not the window ({}): {boxed:?}",
            content.center().y,
            screen.center().y
        );
    }

    /// The curve is the point: it must run *past* the target and come back. A
    /// monotonic ease would satisfy "it moves and settles" while looking like
    /// the thing it replaced.
    #[test]
    fn the_resize_curve_overshoots_and_settles() {
        assert_eq!(ease_out_back(0.0), 0.0);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-5, "it lands on target");

        let peak = (1..100)
            .map(|i| ease_out_back(i as f32 / 100.0))
            .fold(f32::MIN, f32::max);
        assert!(peak > 1.0, "it has to overshoot, else it is not `back`");
        assert!(peak < 1.2, "but not so far it reads as a glitch: {peak}");

        // The overshoot is late in the curve — it arrives fast, then settles.
        let quarter = ease_out_back(0.25);
        assert!(quarter > 0.5, "most of the distance is covered early: {quarter}");
    }

    /// Re-targeting mid-flight has to continue from where the value *is*. A
    /// pop-up whose size changes twice quickly would otherwise snap back to the
    /// first size before starting the second move.
    #[test]
    fn a_resize_interrupted_by_another_continues_from_where_it_is() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("t");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        // Settle at 600.
        let mut v = 0.0;
        for _ in 0..3 {
            let _ = ctx.run(input.clone(), |ctx| v = animate_back(ctx, id, 600.0, 0.22));
        }
        assert!((v - 600.0).abs() < 0.5);

        // Retarget to 300 and stop part-way.
        let mut mid = 0.0;
        for _ in 0..4 {
            let _ = ctx.run(input.clone(), |ctx| mid = animate_back(ctx, id, 300.0, 0.22));
        }
        assert!(mid < 600.0 && mid > 300.0, "in flight: {mid}");

        // Now retarget again; the next value must be near where it was, not back
        // at 600.
        let mut after = 0.0;
        let _ = ctx.run(input.clone(), |ctx| after = animate_back(ctx, id, 900.0, 0.22));
        // It kept moving for that frame, so it is not exactly `mid` — the
        // invariant is that it stayed where it had got to rather than snapping
        // back to where it started or ahead to where it is going.
        assert!(
            (after - mid).abs() < (after - 600.0).abs(),
            "should continue from {mid}, not jump back toward 600 (got {after})"
        );
        assert!(
            (after - mid).abs() < (after - 900.0).abs(),
            "and not jump ahead to the new target (got {after})"
        );
    }

    /// A button that carries a question must not reach the module until it is
    /// answered — that is the whole point of putting the question on the button.
    #[test]
    fn a_button_with_a_question_is_not_a_plain_button() {
        let plain: Widget = serde_json::from_str(
            r#"{"kind":"button","text":"Go","action":{"capability":"c","method":"m"}}"#,
        )
        .unwrap();
        match plain {
            Widget::Button { confirm, .. } => assert!(confirm.is_none(), "asking is opt-in"),
            _ => panic!("expected a button"),
        }

        let asking: Widget = serde_json::from_str(
            r#"{"kind":"button","text":"Delete","action":{"capability":"c","method":"m"},
                "confirm":{"title":"Remove these rules?","subject":"yara-rules-core.yar"}}"#,
        )
        .unwrap();
        match asking {
            Widget::Button { confirm, .. } => {
                let c = confirm.expect("the question");
                assert_eq!(c.subject, "yara-rules-core.yar");
                // Labels are optional: the host has its own words for yes and no.
                assert!(c.confirm_label.is_empty());
            }
            _ => panic!("expected a button"),
        }
    }

    /// The consent dialog is the app's only overlay that used to snap in and
    /// out. It has to animate, and — because the answer clears the pending
    /// action immediately — it has to keep drawing after it is answered, or
    /// there would be nothing left to animate away.
    #[test]
    fn the_consent_dialog_animates_out_after_it_is_answered() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut gone_while_open = None;
        let _ = ctx.run(input.clone(), |ctx| {
            gone_while_open = Some(consent_dialog(ctx, true, None, |ui| {
                ui.label("“loki” wants to run “scan”");
            }));
        });
        assert_eq!(gone_while_open, Some(false), "an open dialog is not gone");

        // Answered: it keeps drawing while it fades, and only then reports that
        // the caller may let go.
        let mut gone = false;
        for _ in 0..200 {
            let _ = ctx.run(input.clone(), |ctx| {
                gone = consent_dialog(ctx, false, None, |ui| {
                    ui.label("“loki” wants to run “scan”");
                });
            });
            if gone {
                break;
            }
        }
        assert!(gone, "it must eventually finish closing");
    }

    /// Control for the pop-up test below: the shipped confirm dialog uses the
    /// same screen-sized veil, so if this hangs too the harness is at fault
    /// rather than the layer being tested.
    #[test]
    fn control_the_shipped_confirm_dialog_renders_headlessly() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("behind");
            });
            let _ = confirm_dialog(ctx, "test", true, "Remove module?", Some("loki"), "Remove", "Cancel", None);
        });
    }

    /// The pop-up layer has to survive a real frame — it allocates a
    /// screen-sized veil, nests a scroll area and renders arbitrary module
    /// widgets, any of which can panic on a bad layout rather than fail a
    /// deserialize.
    #[test]
    fn the_pop_up_layer_renders_a_module_view() {
        let view: View = serde_json::from_str(
            r#"{"title":"Scan settings","modal":"settings","widgets":[
                {"kind":"label","text":"Scanning"},
                {"kind":"checkbox","id":"procs","label":"Process memory"},
                {"kind":"row","children":[
                    {"kind":"select","id":"cpu","options":["100","80"],"label":"CPU"},
                    {"kind":"text","id":"alert","label":"Alert"}]},
                {"kind":"separator"},
                {"kind":"button","text":"Cancel","action":{"capability":"c","method":"m"},"dismiss":true}]}"#,
        )
        .unwrap();
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut inputs = HashMap::new();
        // A real frame always has a screen rect, and the veil is allocated at
        // exactly that size — without one there is nothing bounding the layout.
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        for (open, depth) in [(true, 1), (true, 2), (false, 2)] {
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("behind");
                });
                // Open, then closing — the closing frame still has to draw.
                let _ = modal_layer(ctx, Some(&view), open, depth, None, &mut inputs, None);
            });
        }
    }

    /// Cancel has to actually cancel. A widget's default seeds its entry only
    /// on first draw, so unless the abandoned values are forgotten the pop-up
    /// reopens showing the very edits it was meant to discard.
    #[test]
    fn a_closed_pop_up_takes_its_abandoned_edits_with_it() {
        let popup: View = serde_json::from_str(
            r#"{"title":"Settings","modal":"s","widgets":[
                {"kind":"select","id":"cpu","options":["100"]},
                {"kind":"row","children":[{"kind":"text","id":"alert"}]},
                {"kind":"checkbox","id":"procs"}]}"#,
        )
        .unwrap();
        let mut ids = widget_ids(&popup);
        ids.sort();
        assert_eq!(ids, vec!["alert", "cpu", "procs"], "including nested rows");

        let mut inputs: HashMap<String, String> = [
            ("cpu".to_string(), "40".to_string()),
            ("target".to_string(), "/srv".to_string()),
        ]
        .into_iter()
        .collect();
        for id in widget_ids(&popup) {
            inputs.remove(&id);
        }
        assert!(!inputs.contains_key("cpu"), "the abandoned edit is gone");
        assert!(
            inputs.contains_key("target"),
            "the screen behind keeps its own fields"
        );
    }

    /// A pop-up's fields have to reach the module, and where the pop-up and the
    /// screen behind it name the same field, the one in front wins — that is the
    /// one the user just typed into.
    #[test]
    fn pop_up_fields_are_collected_over_the_screen_behind() {
        let base: View = serde_json::from_str(
            r#"{"title":"S","widgets":[{"kind":"text","id":"target"},{"kind":"text","id":"depth"}]}"#,
        )
        .unwrap();
        let popup: View = serde_json::from_str(
            r#"{"title":"Settings","widgets":[{"kind":"text","id":"depth"}],"modal":"settings"}"#,
        )
        .unwrap();
        let inputs: HashMap<String, String> = [
            ("target".to_string(), "/srv".to_string()),
            ("depth".to_string(), "9".to_string()),
        ]
        .into_iter()
        .collect();

        let mut m = serde_json::Map::new();
        for v in [&base, &popup] {
            if let Value::Object(o) = collect_params(v, &inputs) {
                m.extend(o);
            }
        }
        assert_eq!(m.get("target").and_then(Value::as_str), Some("/srv"));
        assert_eq!(m.get("depth").and_then(Value::as_str), Some("9"));
    }

    /// What a path field takes, and the fallback for modules built before
    /// `accepts` existed.
    #[test]
    fn accepts_reads_the_spec_and_the_legacy_flag() {
        // The new spelling.
        assert!(Accepts::of("file", false) == Accepts::File);
        assert!(Accepts::of("dir", false) == Accepts::Dir);
        assert!(Accepts::of("file|dir", false) == Accepts::Either);
        assert!(Accepts::of("dir|file", false) == Accepts::Either);
        assert!(Accepts::of(" FILE|DIR ", false) == Accepts::Either);

        // Absent: fall back to `directory`, so an older module is unchanged.
        assert!(Accepts::of("", true) == Accepts::Dir);
        assert!(Accepts::of("", false) == Accepts::File);
        // `accepts` wins when both are given.
        assert!(Accepts::of("file", true) == Accepts::File);

        // Nonsense narrows rather than widens — a typo must not make a field
        // swallow anything at all.
        assert!(Accepts::of("folder", false) == Accepts::File);
        assert!(Accepts::of("anything", false) == Accepts::File);
    }

    #[test]
    fn a_dual_field_takes_both_and_the_others_take_one() {
        for (a, file_ok, dir_ok) in [
            (Accepts::File, true, false),
            (Accepts::Dir, false, true),
            (Accepts::Either, true, true),
        ] {
            assert_eq!(a.takes(false), file_ok, "file into {a:?}", a = a as u8);
            assert_eq!(a.takes(true), dir_ok, "dir into {a:?}", a = a as u8);
        }
    }

    /// Swapping a module's screen must count as a change; refreshing one in
    /// place must not, or a progress checklist would restart its entrance on
    /// every tick.
    #[test]
    fn view_shape_tracks_the_screen_not_its_contents() {
        let parse = |s: &str| -> View { serde_json::from_str(s).unwrap() };

        let basic = parse(
            r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
        );
        // Same screen, different text — a progress step advancing.
        let ticked = parse(
            r#"{"title":"S","widgets":[{"kind":"label","text":"b"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
        );
        assert_eq!(
            view_shape(&basic),
            view_shape(&ticked),
            "text changes must not restart the entrance"
        );

        // A control appears — Basic becoming Advanced.
        let advanced = parse(
            r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"checkbox","id":"x"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
        );
        assert_ne!(view_shape(&basic), view_shape(&advanced));

        // A different screen entirely.
        let other = parse(
            r#"{"title":"Other","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
        );
        assert_ne!(view_shape(&basic), view_shape(&other));

        // Same kinds, different order still counts — the arrangement changed.
        let reordered = parse(
            r#"{"title":"S","widgets":[{"kind":"button","text":"go","action":{"capability":"c","method":"m"}},{"kind":"label","text":"a"}]}"#,
        );
        assert_ne!(view_shape(&basic), view_shape(&reordered));
    }

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
                Md::Link {
                    label: "link".into(),
                    url: "http://x".into()
                },
            ]
        );
        // Unmatched markers stay literal.
        assert_eq!(
            inline_segments("a ** b `c"),
            vec![Md::Text("a ** b `c".into())]
        );
        // Plain text is a single span.
        assert_eq!(
            inline_segments("just text"),
            vec![Md::Text("just text".into())]
        );
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
