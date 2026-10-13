//! Pointing at a line on a map drawn the way a module's map is drawn.
//!
//! The toolkit's own test draws the diagram by itself. A module's map goes
//! through the view renderer and asks for the whole screen, which puts it
//! inside a child of a child with its own clip and its own opacity — and that
//! is the path the card has to work in.

use eframe::egui;
use limen_ui as ui;

fn view() -> ui::View {
    serde_json::from_str(
        r#"{"title":"Map","widgets":[
            {"kind":"diagram","title":"","fill":true,
             "nodes":[
               {"id":"self","label":"PC-HOME","kind":"self","detail":"42:40:62:FC:A8:DA",
                "info":[{"name":"Hardware address","value":"42:40:62:FC:A8:DA"}]},
               {"id":"gw","label":"BC:FC:E7:94:6B:90","kind":"router"}],
             "edges":[{"from":"self","to":"gw","dashed":false,
                       "label":"PC-HOME ↔ BC:FC:E7:94:6B:90",
                       "info":[{"name":"That end","value":"192.168.0.1(BC:FC:E7:94:6B:90)"},
                               {"name":"This end","value":"192.168.0.126(42:40:62:FC:A8:DA)"}]}]},
            {"kind":"separator"},
            {"kind":"row","children":[
               {"kind":"button","text":"Back","action":{"capability":"c","method":"ui"}}]}]}"#,
    )
    .expect("a view")
}

/// Draw the view for a few frames with the pointer where it is told, and hand
/// back everything painted and every line segment drawn.
fn frames(at: Option<egui::Pos2>) -> (Vec<String>, Vec<(egui::Pos2, egui::Pos2)>) {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let view = view();
    let mut inputs = std::collections::HashMap::new();
    let (mut text, mut lines) = (Vec::new(), Vec::new());
    // Three: the pointer, then the hover, then the card it asks for.
    for t in [1.0_f64, 2.0, 3.0] {
        let out = ctx.run(
            egui::RawInput {
                time: Some(t),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events: at
                    .map(|p| vec![egui::Event::PointerMoved(p)])
                    .unwrap_or_default(),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui::render_view(ui, &view, &mut inputs, None, 0.0);
                });
            },
        );
        text.clear();
        lines.clear();
        fn walk(
            s: &egui::Shape,
            text: &mut Vec<String>,
            lines: &mut Vec<(egui::Pos2, egui::Pos2)>,
        ) {
            match s {
                egui::Shape::Text(t) => text.push(t.galley.text().to_string()),
                egui::Shape::LineSegment { points, .. } => lines.push((points[0], points[1])),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, lines)),
                _ => {}
            }
        }
        for c in &out.shapes {
            walk(&c.shape, &mut text, &mut lines);
        }
    }
    (text, lines)
}

/// The card for a line opens on a map drawn as a module's map is drawn.
#[test]
fn a_line_on_a_filling_map_still_answers() {
    let (_, lines) = frames(None);
    // The one long line on that map is the edge; the rest are rules and
    // separators, which are flat.
    let wire = lines
        .iter()
        .copied()
        .filter(|(a, b)| (a.y - b.y).abs() > 2.0)
        .max_by(|a, b| (a.0 - a.1).length().total_cmp(&(b.0 - b.1).length()))
        .expect("the map drew no line");

    let beside = wire.0.lerp(wire.1, 0.5) + egui::vec2(3.0, 0.0);
    let (said, _) = frames(Some(beside));
    let said = said.join(" | ");
    assert!(
        said.contains("This end") && said.contains("192.168.0.126(42:40:62:FC:A8:DA)"),
        "the card did not open on the line: {said}"
    );
}

/// And so does a circle, on the same map — the control for the one above.
#[test]
fn a_circle_on_a_filling_map_answers() {
    let (_, lines) = frames(None);
    let wire = lines
        .iter()
        .copied()
        .filter(|(a, b)| (a.y - b.y).abs() > 2.0)
        .max_by(|a, b| (a.0 - a.1).length().total_cmp(&(b.0 - b.1).length()))
        .expect("the map drew no line");
    // The line is trimmed to the circles' edges, so just past its end is a
    // circle.
    let along = (wire.1 - wire.0).normalized();
    let circle = wire.0 - along * 10.0;
    let (said, _) = frames(Some(circle));
    let said = said.join(" | ");
    // The fact's *name*, which is on the card and nowhere else on the map —
    // its value is also painted under the circle, so asserting on that would
    // pass with no card at all.
    assert!(
        said.contains("Hardware address"),
        "no card for the circle either: {said}"
    );
}
