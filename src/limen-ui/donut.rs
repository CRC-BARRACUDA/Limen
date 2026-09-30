//! A donut: the shares of one whole.
//!
//! The third of the picture widgets. A [`chart`](crate::render_chart) compares
//! magnitudes — which is biggest, by how much. A [`diagram`](crate::diagram)
//! answers what is joined to what. A donut answers only *how much of the whole*
//! each part is, which is the one question bars answer badly and the one a
//! reader asks of a severity breakdown or a disk.
//!
//! It is the wrong widget for more than a handful of parts, or for parts of
//! nearly equal size — two slices a degree apart cannot be told from each other
//! by eye, where two bars can. A module with eight categories wants bars.
//!
//! Drawn as a thick stroked arc per slice rather than as a filled ring sector:
//! a sector is concave, and a concave fill triangulates badly, while a stroked
//! path is exactly what a donut is — a line of a certain width bent round.

use crate::*;

/// One part of the whole.
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct DonutSlice {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: f64,
    /// What this part *is*, which decides its colour: `critical`, `high`,
    /// `medium`, `low`, `ok`, `info`, or anything else to be given one from the
    /// palette in the order the slices arrived.
    #[serde(default)]
    pub kind: String,
}

/// How thick the ring is, as a share of its radius. A ring much thinner than
/// this stops reading as a quantity and starts reading as an outline.
const THICKNESS: f32 = 0.34;
/// How far the slice under the pointer swells, and how long it takes: far
/// enough to be plainly the one being asked about, quick enough that running
/// the pointer round the ring does not feel like wading.
const SWELL: f32 = 6.0;
const SWELL_OUT: f32 = 3.0;
const SWELL_TIME: f32 = 0.12;
/// How far each slice runs past where the next one begins, in pixels of arc.
///
/// Every stroke is drawn with a feathered edge — a pixel or so of fade, which is
/// what stops it looking jagged. At a junction two of those fades meet and leave
/// a dark hairline: a seam nobody asked for, in a ring that is meant to be
/// continuous. So each slice runs a little way into the next, which is drawn
/// afterwards and covers it.
const OVERLAP: f32 = 2.0;

/// The colour a slice is drawn in.
///
/// A named kind keeps the meaning it has everywhere else in Limen — a critical
/// count is the same red in a donut as in a table — and anything unnamed is
/// given a colour by its position, so the same data always comes out the same.
fn slice_colour(kind: &str, index: usize) -> egui::Color32 {
    const PALETTE: [egui::Color32; 6] = [
        color::ACCENT,
        egui::Color32::from_rgb(0x5b, 0x9d, 0xd9),
        color::WARNING,
        color::SUCCESS,
        egui::Color32::from_rgb(0xa5, 0x7f, 0xd6),
        color::TEXT_MUTED,
    ];
    match kind {
        "critical" | "error" => color::ERROR,
        "high" => color::ORANGE,
        "medium" | "warning" => color::WARNING,
        "low" | "ok" | "success" => color::SUCCESS,
        "info" => PALETTE[1],
        _ => PALETTE[index % PALETTE.len()],
    }
}

/// Draw a donut with its legend beside it.
///
/// `centre` is what goes in the hole; empty means the total. The hole is the
/// donut's one advantage over a pie and it should not be wasted — what a reader
/// wants there is the figure the slices are shares of.
pub fn render_donut(ui: &mut egui::Ui, title: &str, slices: &[DonutSlice], centre: &str) {
    if !title.is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }
    // A slice of nothing is not a slice; a donut of nothing is not a donut.
    let shown: Vec<(usize, &DonutSlice)> = slices
        .iter()
        .enumerate()
        .filter(|(_, s)| s.value > 0.0)
        .collect();
    let total: f64 = shown.iter().map(|(_, s)| s.value).sum();
    if shown.is_empty() || total <= 0.0 {
        ui.label(egui::RichText::new("—").color(color::TEXT_MUTED));
        return;
    }

    let font = egui::TextStyle::Body.resolve(ui.style());
    let row_h = font.size + 8.0;
    // The block is capped: on a wide window the ring would otherwise sit on the
    // left with its figures flung against the far edge, and a legend you have to
    // travel to read is not a legend.
    let avail = ui.available_width().clamp(160.0, 520.0);
    let size = (avail * 0.36).clamp(110.0, 190.0);
    let legend_h = row_h * shown.len() as f32;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(avail, size.max(legend_h) + 4.0),
        egui::Sense::hover(),
    );

    let middle = egui::pos2(rect.left() + size / 2.0, rect.top() + size / 2.0);
    // Room left for the swell, so the slice under the pointer rises inside the
    // box the donut was given rather than over whatever is beside it.
    let radius = size / 2.0 - 2.0 - (SWELL_OUT + SWELL / 2.0);
    let width = radius * THICKNESS;
    // The stroke is centred on the path, so the path runs down the middle of
    // the ring rather than along its outer edge.
    let path_r = radius - width / 2.0;

    let hovered = response
        .hover_pos()
        .and_then(|pos| hit(&shown, total, middle, path_r, width, pos));

    // How far each slice has swollen towards being the one under the pointer.
    // Held per slice rather than read off `hovered`, so the one being left
    // settles back while the one being arrived at grows: the ring moves the way
    // a hand moves across it.
    let id = ui.id().with(("donut", title));
    let swell: Vec<f32> = (0..shown.len())
        .map(|n| smoothstep(anim_bool(ui, id.with(n), hovered == Some(n), SWELL_TIME)))
        .collect();

    // From twelve o'clock, clockwise: where a reader starts, and the direction
    // every clock face has taught them to go.
    let mut angle = -std::f32::consts::FRAC_PI_2;
    let painter = ui.painter();
    for (n, (i, slice)) in shown.iter().enumerate() {
        let span = (slice.value / total) as f32 * std::f32::consts::TAU;
        let t = swell[n];
        painter.add(egui::Shape::Path(egui::epaint::PathShape {
            // Wider, and further out: the slice rises off the ring rather than
            // only thickening into the hole.
            points: arc(
                middle,
                path_r + SWELL_OUT * t,
                angle,
                angle + span + (OVERLAP / path_r).min(span),
            ),
            closed: false,
            fill: egui::Color32::TRANSPARENT,
            stroke: egui::Stroke::new(
                width + SWELL * t,
                lerp_color(
                    slice_colour(&slice.kind, *i),
                    color::ACCENT_BRIGHT,
                    t * 0.18,
                ),
            )
            .into(),
        }));
        angle += span;
    }

    // The hole says what the slices are shares of.
    let hole = if centre.is_empty() {
        fmt_num(total)
    } else {
        centre.to_owned()
    };
    painter.text(
        middle,
        egui::Align2::CENTER_CENTER,
        ellipsis(ui, &hole, &font, radius * 1.5),
        font.clone(),
        color::TEXT,
    );

    // The legend, which is where the numbers actually are: a slice can be read
    // for its share, never for its value.
    let x = rect.left() + size + 18.0;
    let mut y = rect.top() + (rect.height() - legend_h).max(0.0) / 2.0;
    let small = egui::FontId::proportional(font.size - 1.0);
    for (n, (i, slice)) in shown.iter().enumerate() {
        let colour = slice_colour(&slice.kind, *i);
        let t = swell[n];
        let mid = y + row_h / 2.0;
        painter.rect_filled(
            egui::Rect::from_center_size(egui::pos2(x + 5.0, mid), egui::Vec2::splat(9.0)),
            egui::Rounding::same(2.0),
            colour,
        );
        let share = format!("{:.0}%", slice.value / total * 100.0);
        let figures = format!("{}  {}", fmt_num(slice.value), share);
        let figures_w = ui.fonts(|f| {
            f.layout_no_wrap(figures.clone(), small.clone(), color::TEXT)
                .size()
                .x
        });
        let room = (rect.right() - (x + 18.0) - figures_w - 12.0).max(40.0);
        painter.text(
            egui::pos2(x + 18.0, mid),
            egui::Align2::LEFT_CENTER,
            ellipsis(ui, &slice.label, &font, room),
            font.clone(),
            lerp_color(color::TEXT_MUTED, color::TEXT, t),
        );
        painter.text(
            egui::pos2(rect.right(), mid),
            egui::Align2::RIGHT_CENTER,
            figures,
            small.clone(),
            lerp_color(color::TEXT, color::ACCENT_BRIGHT, t),
        );
        y += row_h;
    }

    // The exact figure for the slice under the pointer, drawn by the app in its
    // own colours — a slice is read for its share, so its value has to be asked
    // for somewhere.
    if let Some(n) = hovered {
        let (_, slice) = shown[n];
        let share = slice.value / total * 100.0;
        egui::show_tooltip_at_pointer(
            &ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::new("donut.slice"),
            |ui| card(ui, &slice.label, &format!("{}  ·  {:.1}%", fmt_num(slice.value), share)),
        );
    }
}

/// What a slice says when the pointer is on it: its name, and the figure the
/// ring cannot be read for.
///
/// Neither line wraps. A tooltip is placed wherever there is room for it, and
/// where there is not much it is given a narrow column — which turned one word
/// into two and a figure into two halves. Two short strings have nothing to
/// gain from being wrapped, so they are drawn at the width they are.
fn card(ui: &mut egui::Ui, label: &str, figures: &str) {
    ui.set_max_width(340.0);
    ui.add(
        egui::Label::new(egui::RichText::new(label).strong().color(color::TEXT)).extend(),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(figures)
                .monospace()
                .color(color::TEXT_MUTED),
        )
        .extend(),
    );
}

/// The points of one arc, at a step fine enough that the curve reads as a curve.
fn arc(middle: egui::Pos2, r: f32, from: f32, to: f32) -> Vec<egui::Pos2> {
    let span = (to - from).abs();
    let steps = ((span / 0.08).ceil() as usize).clamp(2, 180);
    (0..=steps)
        .map(|k| {
            let a = from + (to - from) * k as f32 / steps as f32;
            egui::pos2(middle.x + r * a.cos(), middle.y + r * a.sin())
        })
        .collect()
}

/// Which slice the pointer is on: inside the ring, at the angle it is at.
fn hit(
    shown: &[(usize, &DonutSlice)],
    total: f64,
    middle: egui::Pos2,
    path_r: f32,
    width: f32,
    pos: egui::Pos2,
) -> Option<usize> {
    let d = pos - middle;
    let r = d.length();
    if (r - path_r).abs() > width / 2.0 + 3.0 {
        return None;
    }
    // Measured the way the slices are laid: from twelve o'clock, clockwise.
    let mut a = d.y.atan2(d.x) + std::f32::consts::FRAC_PI_2;
    while a < 0.0 {
        a += std::f32::consts::TAU;
    }
    let mut from = 0.0_f32;
    for (n, (_, slice)) in shown.iter().enumerate() {
        let span = (slice.value / total) as f32 * std::f32::consts::TAU;
        if a >= from && a < from + span {
            return Some(n);
        }
        from += span;
    }
    None
}

/// `text` cut to `width` with an ellipsis, measured in the font it is drawn in.
fn ellipsis(ui: &egui::Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let measure = |s: &str| {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    if measure(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut next = out.clone();
        next.push(ch);
        if measure(&format!("{next}…")) > width {
            break;
        }
        out = next;
    }
    format!("{out}…")
}

// In this file rather than in `tests/`, because the invariant is about a piece
// the outside cannot reach: the card a hovered slice raises.
#[cfg(test)]
mod tests {
    use super::card;
    use crate::egui;

    /// The card never wraps, however little room it is given.
    ///
    /// A tooltip is placed wherever there is room for it, and where there is
    /// little it is handed a narrow column — which is how "critical" came to be
    /// drawn as "criti" over "cal". Two short strings gain nothing from being
    /// wrapped, so they are drawn at the width they are and the card grows to
    /// hold them.
    #[test]
    fn the_card_never_wraps() {
        let ctx = egui::Context::default();
        crate::apply_theme(&ctx);
        let out = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // As cramped as a tooltip pushed against the edge of a window:
                // a ui that genuinely has forty pixels, not one merely asked to
                // pretend it has.
                let narrow = egui::Rect::from_min_size(
                    ui.max_rect().min,
                    egui::vec2(40.0, 200.0),
                );
                let mut ui = ui.child_ui(
                    narrow,
                    egui::Layout::top_down(egui::Align::Min),
                    None,
                );
                card(&mut ui, "critical", "3  ·  6.1%");
            });
        });
        let mut seen = 0;
        for shape in out.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                seen += 1;
                assert_eq!(
                    t.galley.rows.len(),
                    1,
                    "{:?} was broken across {} lines in a 40px column",
                    t.galley.text(),
                    t.galley.rows.len()
                );
            }
        }
        assert_eq!(seen, 2, "the card draws its name and its figures");
    }
}
