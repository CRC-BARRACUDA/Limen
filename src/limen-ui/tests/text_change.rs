//! A text field that tells the module as it is typed.

use eframe::egui;
use limen_ui::*;

/// Render one view and return what it asked the host to invoke.
fn typed(view: &serde_json::Value, keys: &str) -> Option<Invoke> {
    let parsed: View = serde_json::from_value(view.clone()).expect("the view");
    let ctx = egui::Context::default();
    let mut inputs = std::collections::HashMap::new();
    let mut out = None;
    for (i, ch) in keys.chars().enumerate() {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events: vec![egui::Event::Text(ch.to_string())],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Focus the field on the first frame, so the keystrokes land.
                let got = render_view(ui, &parsed, &mut inputs, None, 0.0);
                if i > 0 && got.is_some() {
                    out = got;
                }
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(egui::Id::new("q")));
                }
            });
        });
    }
    out
}

/// A field without `on_change` asks for nothing, however much is typed — the
/// behaviour every existing form relies on.
#[test]
fn a_plain_field_does_not_invoke_anything() {
    let view = serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "text", "id": "q" }],
    });
    assert!(typed(&view, "abc").is_none());
}

/// A field *with* it deserialises and carries the action it was given.
#[test]
fn a_field_can_carry_an_on_change_action() {
    let view: View = serde_json::from_value(serde_json::json!({
        "title": "t",
        "widgets": [{
            "kind": "text",
            "id": "q",
            "on_change": { "capability": "autoruns.local", "method": "filter_touch" },
        }],
    }))
    .expect("a field may carry on_change");

    match &view.widgets[0] {
        Widget::Text { on_change, .. } => {
            let a = on_change.as_ref().expect("the action survived");
            assert_eq!(a.capability, "autoruns.local");
            assert_eq!(a.method, "filter_touch");
        }
        other => panic!("expected a text field, got {other:?}"),
    }
}

/// Every view written before this existed still deserialises: the field is
/// optional, and absent means a field that asks for nothing.
#[test]
fn a_field_without_one_is_still_a_field() {
    let view: View = serde_json::from_value(serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "text", "id": "q", "label": "Search" }],
    }))
    .expect("no on_change is fine");
    match &view.widgets[0] {
        Widget::Text { on_change, label, .. } => {
            assert!(on_change.is_none());
            assert_eq!(label, "Search");
        }
        other => panic!("expected a text field, got {other:?}"),
    }
}
