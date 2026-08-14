//! The painted controls: buttons, toggles, chips, dropdowns, the window chrome.

use crate::*;

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
pub fn dropdown_option(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
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
pub fn chamfer_pts(rect: egui::Rect, ch: f32) -> [egui::Pos2; 6] {
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
pub type Ramp = [egui::Color32; 4];

pub const RAMP_PRIMARY: Ramp = [
    egui::Color32::from_rgb(0xc2, 0x41, 0x0c),
    egui::Color32::from_rgb(0xea, 0x58, 0x0c),
    color::ORANGE,
    color::ORANGE_BRIGHT,
];

pub fn filled_button(ui: &mut egui::Ui, text: &str, min: egui::Vec2, ramp: Ramp) -> egui::Response {
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
pub struct OutlineTint {
    border: egui::Color32,
    label: egui::Color32,
    hot: egui::Color32,
    hot_label: egui::Color32,
}

/// A transparent chamfered outline that eases from its resting colours to its
/// hover ones.
pub fn outlined_button(
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

/// A step's status icon: a faint hollow circle (pending), a rotating spinner
/// (loading), or a *settled* mark — a green check (`done`), red ✕ (`error`), or
/// amber ⚠ (`warning`). The spinner smoothly morphs into the settled mark. Keyed
/// by `key` (the step label) so the morph persists across the view updates that
/// advance a multi-step flow.
pub fn step_icon(ui: &mut egui::Ui, key: &str, state: &str) {
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
