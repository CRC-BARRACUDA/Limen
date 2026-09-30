//! A donut: that the ring is the data, and the legend is the numbers.

use eframe::egui;
use limen_ui as ui;
use limen_ui::DonutSlice;

fn slice(label: &str, value: f64, kind: &str) -> DonutSlice {
    DonutSlice {
        label: label.into(),
        value,
        kind: kind.into(),
    }
}

fn four() -> Vec<DonutSlice> {
    vec![
        slice("critical", 3.0, "critical"),
        slice("high", 11.0, "high"),
        slice("medium", 27.0, "medium"),
        slice("low", 8.0, "low"),
    ]
}

struct Drawn {
    texts: Vec<String>,
    /// Every line of every galley: a card squeezed into a column shows up here
    /// as one word broken across several.
    rows: Vec<(String, usize)>,
    paths: Vec<egui::epaint::PathShape>,
}

fn draw(slices: &[DonutSlice], centre: &str, events: Vec<egui::Event>) -> Drawn {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let mut out = Drawn { texts: Vec::new(), rows: Vec::new(), paths: Vec::new() };
    // Three frames: the pointer arrives in the first, is hovering by the
    // second, and the card it raises has settled by the third.
    for time in [1.0_f64, 1.05, 1.1] {
        out.texts.clear();
        out.rows.clear();
        out.paths.clear();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 400.0),
            )),
            events: events.clone(),
            time: Some(time),
            ..Default::default()
        };
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui::render_donut(ui, "Findings", slices, centre);
            });
        });
        for shape in full.shapes {
            match shape.shape {
                egui::epaint::Shape::Text(t) => {
                    out.texts.push(t.galley.text().to_owned());
                    out.rows.push((t.galley.text().to_owned(), t.galley.rows.len()));
                }
                egui::epaint::Shape::Path(p) => out.paths.push(p),
                _ => {}
            }
        }
    }
    out
}

/// The ring is one ring: every slice reaches the next, and a little past it.
///
/// Meeting exactly is not enough. Each stroke is drawn with a feathered edge,
/// and two of those meeting at a junction leave a dark hairline — so a slice has
/// to run into its neighbour, which is drawn afterwards and covers it.
#[test]
fn the_slices_overlap_rather_than_merely_meet() {
    let d = draw(&four(), "", vec![]);
    let ring: Vec<egui::Pos2> = d.paths.iter().flat_map(|p| p.points.iter().copied()).collect();
    let c = egui::pos2(
        ring.iter().map(|p| p.x).sum::<f32>() / ring.len() as f32,
        ring.iter().map(|p| p.y).sum::<f32>() / ring.len() as f32,
    );
    let angle = |p: &egui::Pos2| (p.y - c.y).atan2(p.x - c.x);
    // How far past its neighbour's first point each slice's last point reaches,
    // measured the short way round.
    let past = |from: &egui::Pos2, to: &egui::Pos2| {
        let mut d = angle(to) - angle(from);
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d < -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        d
    };

    let radius = (d.paths[0].points[0] - c).length();
    for n in 0..d.paths.len() {
        let end = d.paths[n].points.last().unwrap();
        let start = d.paths[(n + 1) % d.paths.len()].points[0];
        let over = past(&start, end) * radius;
        assert!(
            over > 0.5,
            "slice {n} stops {over}px short of the next — that is the hairline"
        );
        assert!(over < 4.0, "slice {n} runs {over}px into the next, which will show");
    }
}

/// The slice under the pointer swells — wider, and further out — and it does it
/// over time rather than snapping, so running the pointer round the ring reads
/// as one movement.
#[test]
fn the_slice_under_the_pointer_swells_into_it() {
    let base = draw(&four(), "", vec![]);
    let cold = base.paths[0].stroke.width;

    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let at = on_the_first_slice(&base);
    let mut widths = Vec::new();
    // Frame by frame with the pointer held still on one slice.
    for time in [1.0_f64, 1.02, 1.05, 1.1, 1.3] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(700.0, 400.0),
            )),
            events: vec![egui::Event::PointerMoved(at)],
            time: Some(time),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui::render_donut(ui, "Findings", &four(), "");
            });
        });
        for shape in out.shapes {
            if let egui::epaint::Shape::Path(p) = shape.shape {
                widths.push(p.stroke.width);
                break;
            }
        }
    }

    assert!(
        widths.last().unwrap() > &(cold + 3.0),
        "it never swelled: {widths:?} against a resting {cold}"
    );
    // Somewhere on the way it was part-grown: that is what makes it a movement
    // rather than a jump.
    assert!(
        widths.iter().any(|w| *w > cold + 0.2 && *w < cold + 5.0),
        "it snapped straight to its full size: {widths:?}"
    );
    assert!(
        widths.windows(2).all(|w| w[1] >= w[0] - 0.01),
        "it grew and shrank on the way: {widths:?}"
    );
}

/// Where to put the pointer to be on the first slice, which starts at twelve
/// o'clock.
fn on_the_first_slice(d: &Drawn) -> egui::Pos2 {
    let ring: Vec<egui::Pos2> = d.paths.iter().flat_map(|p| p.points.iter().copied()).collect();
    let c = egui::pos2(
        ring.iter().map(|p| p.x).sum::<f32>() / ring.len() as f32,
        ring.iter().map(|p| p.y).sum::<f32>() / ring.len() as f32,
    );
    let top = ring.iter().min_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    egui::pos2(c.x + (top.x - c.x) * 0.98, c.y + (top.y - c.y) * 0.98)
}

/// One arc per slice, each drawn as a stroked path rather than a filled sector:
/// a sector is concave, and a concave fill triangulates badly.
#[test]
fn every_slice_is_one_stroked_arc() {
    let d = draw(&four(), "", vec![]);
    assert_eq!(d.paths.len(), 4, "one path per slice");
    for p in &d.paths {
        assert!(!p.closed, "a donut slice is a bent line, not a closed shape");
        assert!(p.stroke.width > 1.0, "it is the stroke that makes the ring");
        assert!(p.points.len() >= 3, "an arc of two points is a straight line");
    }
}

/// The angle a slice takes is its share of the whole — that is the entire claim
/// a donut makes, and the only one it can be read for.
#[test]
fn a_slice_takes_its_share_of_the_ring() {
    let d = draw(&four(), "", vec![]);
    let total: f64 = four().iter().map(|s| s.value).sum();
    let points: Vec<egui::Pos2> = d.paths.iter().flat_map(|p| p.points.iter().copied()).collect();
    let c = egui::pos2(
        points.iter().map(|p| p.x).sum::<f32>() / points.len() as f32,
        points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32,
    );
    let angle = |p: &egui::Pos2| (p.y - c.y).atan2(p.x - c.x);
    for (p, s) in d.paths.iter().zip(four()) {
        // How far the arc turns, step by step: a slice of more than half the
        // ring cannot be measured from its two ends alone.
        let span: f32 = p
            .points
            .windows(2)
            .map(|w| {
                let mut d = angle(&w[1]) - angle(&w[0]);
                while d > std::f32::consts::PI {
                    d -= std::f32::consts::TAU;
                }
                while d < -std::f32::consts::PI {
                    d += std::f32::consts::TAU;
                }
                d.abs()
            })
            .sum();
        let want = (s.value / total) as f32 * std::f32::consts::TAU;
        assert!(
            (span - want).abs() < 0.12,
            "{} took {span} of the ring, not {want}",
            s.label
        );
    }
}

/// The hole is the donut's one advantage over a pie: it holds the figure the
/// slices are shares of, or whatever the module put there instead.
#[test]
fn the_hole_holds_the_total_unless_told_otherwise() {
    let d = draw(&four(), "", vec![]);
    assert!(d.texts.iter().any(|t| t == "49"), "the total: {:?}", d.texts);
    let d = draw(&four(), "49 findings", vec![]);
    assert!(d.texts.iter().any(|t| t == "49 findings"));
    assert!(!d.texts.iter().any(|t| t == "49"), "both, where one was asked for");
}

/// A slice can be read for its share and never for its value, so the legend
/// carries the numbers — every label, its figure, and its percentage.
#[test]
fn the_legend_carries_what_the_ring_cannot_say() {
    let d = draw(&four(), "", vec![]);
    for label in ["critical", "high", "medium", "low"] {
        assert!(d.texts.iter().any(|t| t == label), "{label} is not in the legend");
    }
    assert!(
        d.texts.iter().any(|t| t.starts_with("27") && t.contains("55%")),
        "the figures and shares are missing: {:?}",
        d.texts
    );
}

/// Nothing at all, and parts that are nothing, are not a donut.
#[test]
fn nothing_is_drawn_rather_than_a_ring_of_nothing() {
    for slices in [vec![], vec![slice("none", 0.0, "")]] {
        let d = draw(&slices, "", vec![]);
        assert!(d.paths.is_empty(), "a ring was drawn for {slices:?}");
        assert!(d.texts.iter().any(|t| t == "—"));
    }
    // A slice worth nothing is left out; the ones worth something are not.
    let d = draw(&[slice("gone", 0.0, ""), slice("here", 5.0, "")], "", vec![]);
    assert_eq!(d.paths.len(), 1);
    assert!(!d.texts.iter().any(|t| t == "gone"));
}

/// One part of one whole is the whole ring, with no seam taken out of it for a
/// neighbour it does not have.
#[test]
fn one_slice_is_the_whole_ring() {
    let d = draw(&[slice("all", 9.0, "ok")], "", vec![]);
    assert_eq!(d.paths.len(), 1);
    let p = &d.paths[0];
    let (a, b) = (p.points.first().unwrap(), p.points.last().unwrap());
    assert!(
        a.distance(*b) < 2.0,
        "the ring does not close on itself: {a:?} to {b:?}"
    );
}

/// Hovering a slice asks for its exact figure, and the app answers in its own
/// colours rather than leaving it to be guessed off the ring.
#[test]
fn hovering_a_slice_gives_its_figure() {
    // Straight up from the middle is the first slice, which starts at twelve
    // o'clock: the donut is drawn at the left of a 700px panel.
    let on = on_the_first_slice(&draw(&four(), "", vec![]));
    let hovered = draw(&four(), "", vec![egui::Event::PointerMoved(on)]);
    assert!(
        hovered.texts.iter().any(|t| t.contains("6.1%")),
        "critical is 3 of 49 — the card did not say so: {:?}",
        hovered.texts
    );
}

/// The card says its piece on one line each. It is two short strings; a card
/// that breaks "critical" across two lines has been squeezed into a column.
#[test]
fn the_card_is_not_squeezed_into_a_column() {
    let on = on_the_first_slice(&draw(&four(), "", vec![]));
    let d = draw(&four(), "", vec![egui::Event::PointerMoved(on)]);
    for (text, rows) in &d.rows {
        if text == "critical" || text.contains("6.1%") {
            assert_eq!(*rows, 1, "{text:?} was broken across {rows} lines");
        }
    }
    assert!(
        d.rows.iter().any(|(t, _)| t.contains("6.1%")),
        "the card was not drawn at all"
    );
}
