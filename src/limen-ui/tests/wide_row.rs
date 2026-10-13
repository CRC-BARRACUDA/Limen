//! A row of fields stays inside the window.
//!
//! A box to type in has no width of its own: it is as wide as it is allowed to
//! be. Two of them side by side, in a row that is not shared out, came out as
//! one enormous box and a second one past the right edge of the window — and a
//! form scrolls downwards, so what overran sideways could not be reached at all.

use eframe::egui;
use limen_ui as ui;

const PANEL: f32 = 1000.0;

/// The rightmost edge anything was painted at, and how far the layout claimed
/// to need.
fn reach(widgets: &str) -> f32 {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let view: ui::View =
        serde_json::from_str(&format!(r#"{{"title":"T","widgets":[{widgets}]}}"#)).expect("a view");
    let mut inputs = std::collections::HashMap::new();
    let mut right = 0.0_f32;
    // Several frames: the entrance fades in, and a widget still arriving has
    // not yet been given its final place.
    for t in [0.0_f64, 1.0, 2.0] {
        let input = egui::RawInput {
            time: Some(t),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(PANEL, 700.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui::render_view(ui, &view, &mut inputs, None, 0.0);
            });
        });
        if t < 1.5 {
            continue;
        }
        right = 0.0;
        fn walk(s: &egui::Shape, right: &mut f32) {
            match s {
                egui::Shape::Rect(r) => *right = right.max(r.rect.right()),
                egui::Shape::Text(t) => {
                    *right = right.max(t.pos.x + t.galley.size().x);
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, right)),
                _ => {}
            }
        }
        for c in &out.shapes {
            walk(&c.shape, &mut right);
        }
    }
    right
}

fn date(id: &str, label: &str) -> String {
    format!(r#"{{"kind":"date","id":"{id}","label":"{label}"}}"#)
}

fn row(children: &[String]) -> String {
    format!(r#"{{"kind":"row","children":[{}]}}"#, children.join(","))
}

/// The case from the form: two date fields on one line.
#[test]
fn two_date_fields_in_a_row_both_fit() {
    let wide = reach(&row(&[
        date("a", "Created after"),
        date("b", "Created before"),
    ]));
    assert!(
        wide <= PANEL + 0.5,
        "the row reached {wide} across a {PANEL} window"
    );
}

/// And a date field beside a box to type in, which is the same problem with
/// one of each.
#[test]
fn a_date_field_beside_a_text_field_fits() {
    let wide = reach(&row(&[
        date("from", "From"),
        r#"{"kind":"text","id":"q","label":"Larger than"}"#.to_string(),
    ]));
    assert!(wide <= PANEL + 0.5, "the row reached {wide}");
}

/// Three of them, which is what the row would hold if a third threshold were
/// added tomorrow.
#[test]
fn three_date_fields_in_a_row_still_fit() {
    let wide = reach(&row(&[
        date("a", "After"),
        date("b", "Before"),
        date("c", "Seen"),
    ]));
    assert!(wide <= PANEL + 0.5, "the row reached {wide}");
}

/// A single date field on a line of its own was never the problem, and must
/// still take the width it is given rather than being squeezed.
#[test]
fn one_date_field_alone_still_fills_its_line() {
    let wide = reach(&row(&[date("a", "Created after")]));
    assert!(wide <= PANEL + 0.5, "the row reached {wide}");
    assert!(
        wide > PANEL * 0.75,
        "a lone field should still use its line: {wide}"
    );
}
