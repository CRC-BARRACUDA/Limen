//! A view stays inside the window: a step's label wraps, and a row's widgets
//! all fit on it.

use eframe::egui;
use limen_ui as ui;

/// Lay a view out headlessly and return every line of text it drew, with where
/// each landed.
fn drawn(view: &serde_json::Value, width: f32) -> Vec<(egui::Rect, String)> {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let parsed: ui::View = serde_json::from_value(view.clone()).expect("a view");
    let mut inputs = std::collections::HashMap::new();
    let mut out = Vec::new();
    for _ in 0..2 {
        out.clear();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            ..Default::default()
        };
        let frame = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = ui::render_view(ui, &parsed, &mut inputs, None, 0.0);
            });
        });
        for shape in frame.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                out.push((
                    t.galley.rect.translate(t.pos.to_vec2()),
                    t.galley.text().to_owned(),
                ));
            }
        }
    }
    out
}

/// A long step label wraps instead of running off the right edge.
///
/// `step` drew its label inside a plain `horizontal`, where the available width
/// is unbounded — so a note about what could not be read, or a host that failed
/// with a reason, was cut off at the window edge with no way to read the rest.
#[test]
fn a_long_step_label_wraps_rather_than_running_off() {
    const LONG: &str = "product_serial, board_serial could not be read — the DMI \
serials are root-only on most systems. An absent serial here does not mean the \
machine has none of its own.";
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "step", "label": LONG, "state": "pending" }],
    });
    let drawn = drawn(&view, 700.0);
    let lines: Vec<&(egui::Rect, String)> =
        drawn.iter().filter(|(_, s)| LONG.contains(s.as_str()) && !s.is_empty()).collect();

    assert!(!lines.is_empty(), "the step drew no text at all");
    // Every piece of it is inside the window…
    for (rect, text) in &lines {
        assert!(
            rect.max.x <= 700.0,
            "{text:?} runs to x={} past the 700px window",
            rect.max.x
        );
    }
    // …and all of it is there, across however many rows it took.
    let rendered: String = lines.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>().join(" ");
    assert!(
        rendered.contains("does not mean the machine has none"),
        "the end of the label was never drawn: {rendered:?}"
    );
}

/// A short one still sits on the icon's line rather than being pushed below it.
#[test]
fn a_short_step_stays_beside_its_icon() {
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "step", "label": "done", "state": "done" }],
    });
    let drawn = drawn(&view, 700.0);
    let label = drawn.iter().find(|(_, s)| s == "done").expect("the label is drawn");
    assert!(label.0.min.x > 12.0, "the label is not beside the icon: {:?}", label.0);
    assert!(label.0.height() < 30.0, "it wrapped when it had no need to");
}

/// Every widget drawn, with where it landed — buttons as well as text.
fn drawn_rects(view: &serde_json::Value, width: f32) -> Vec<(egui::Rect, String)> {
    drawn(view, width)
}

/// A button after a search box is still on the screen.
///
/// A text field asks for all the width there is, so one placed before a button
/// took the lot and pushed the button past the right edge of the window — where
/// it drew, took clicks nobody could aim at, and could not be scrolled to. Four
/// fleet modules ship exactly this row.
#[test]
fn a_button_after_a_field_stays_on_screen() {
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{
            "kind": "row",
            "children": [
                { "kind": "text", "id": "query", "label": "Search", "placeholder": "host…" },
                { "kind": "button", "text": "FILTER",
                  "action": { "capability": "c", "method": "filter" } },
            ],
        }],
    });
    let drawn = drawn_rects(&view, 800.0);
    let button = drawn
        .iter()
        .find(|(_, s)| s == "FILTER")
        .expect("the button drew nothing at all");
    assert!(
        button.0.max.x <= 800.0,
        "the button runs to x={} past the 800px window",
        button.0.max.x
    );
}

/// The same for a row that is mostly buttons with one field among them.
#[test]
fn a_toolbar_with_a_field_in_it_fits() {
    let button = |text: &str| {
        serde_json::json!({ "kind": "button", "text": text,
                            "action": { "capability": "c", "method": "m" } })
    };
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{
            "kind": "row",
            "children": [
                button("SCAN AGAIN"),
                { "kind": "text", "id": "q", "label": "Search", "placeholder": "serial…" },
                button("MAKE REPORT"),
                button("AS ADMINISTRATOR"),
            ],
        }],
    });
    let drawn = drawn_rects(&view, 1200.0);
    for label in ["SCAN AGAIN", "MAKE REPORT", "AS ADMINISTRATOR"] {
        let b = drawn
            .iter()
            .find(|(_, s)| s == label)
            .unwrap_or_else(|| panic!("{label} was not drawn"));
        assert!(
            b.0.max.x <= 1200.0,
            "{label} runs to x={} past the 1200px window",
            b.0.max.x
        );
    }
}

/// A row of buttons alone is untouched — nothing is shared out, and they sit
/// where they always have.
#[test]
fn a_row_without_a_field_keeps_its_natural_layout() {
    let button = |text: &str| {
        serde_json::json!({ "kind": "button", "text": text,
                            "action": { "capability": "c", "method": "m" } })
    };
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "row", "children": [button("ONE"), button("TWO")] }],
    });
    let drawn = drawn_rects(&view, 1200.0);
    let one = drawn.iter().find(|(_, s)| s == "ONE").expect("ONE");
    let two = drawn.iter().find(|(_, s)| s == "TWO").expect("TWO");
    // Side by side and adjacent: the second starts within a button's width of
    // where the first ended, not a quarter of the window away.
    assert!(two.0.min.x - one.0.max.x < 80.0, "{:?} {:?}", one.0, two.0);
}

/// A strip of many buttons wraps rather than running off the right edge.
///
/// The autoruns module offers a button per category — eleven of them — and the
/// page scrolls only downwards, so whatever overran the window could not be
/// reached at all. Worse, it widened the page for everything below it, putting
/// a table's last columns out of reach too. That is sharpest at a UI scale
/// above 100%, where there are fewer points to fit the same buttons into.
#[test]
fn a_long_strip_of_buttons_wraps_instead_of_running_off() {
    let button = |text: &str| {
        serde_json::json!({ "kind": "button", "text": text,
                            "action": { "capability": "c", "method": "m" } })
    };
    let labels = [
        "EVERYTHING", "LOGON", "EXPLORER", "INTERNET EXPLORER", "SCHEDULED",
        "SERVICES AND DRIVERS", "BOOT AND KNOWN DLLS", "LSA PROVIDERS",
        "NETWORK PROVIDERS", "PRINT MONITORS", "OFFICE ADD-INS AND CODECS",
    ];
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{
            "kind": "row",
            "children": labels.iter().map(|l| button(l)).collect::<Vec<_>>(),
        }],
    });

    // A window narrow enough that eleven buttons cannot share one line —
    // roughly what 125% scale leaves of an ordinary window.
    let drawn = drawn_rects(&view, 1000.0);
    for label in labels {
        let b = drawn
            .iter()
            .find(|(_, s)| s == label)
            .unwrap_or_else(|| panic!("{label} was not drawn"));
        assert!(
            b.0.max.x <= 1000.0,
            "{label} runs to x={} past the 1000px window",
            b.0.max.x
        );
    }
    // Which can only be true if they went onto more than one line.
    let tops: std::collections::BTreeSet<i64> = drawn
        .iter()
        .filter(|(_, s)| labels.contains(&s.as_str()))
        .map(|(r, _)| r.min.y as i64)
        .collect();
    assert!(tops.len() > 1, "they all stayed on one line: {tops:?}");
}
