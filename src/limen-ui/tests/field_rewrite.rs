//! A module changing what is in one of its own fields.

use eframe::egui;
use limen_ui::*;
use std::collections::HashMap;

/// Render a one-field view and hand back what the field drew.
fn shown(ctx: &egui::Context, inputs: &mut HashMap<String, String>, default: &str) -> Vec<String> {
    let view: View = serde_json::from_value(serde_json::json!({
        "title": "t",
        "widgets": [{ "kind": "text", "id": "paths", "default": default, "multiline": true }],
    }))
    .expect("a view");
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 400.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_view(ui, &view, inputs, None, 0.0);
            });
        },
    );
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// What the person typed is theirs: a module repeating the same default does
/// not reach over and undo it.
#[test]
fn typing_is_not_overwritten_by_a_default_that_has_not_changed() {
    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut inputs = HashMap::new();

    shown(&ctx, &mut inputs, "");
    // As typing does.
    inputs.insert("paths".into(), "/srv/data".into());
    for _ in 0..3 {
        let texts = shown(&ctx, &mut inputs, "");
        assert!(texts.iter().any(|t| t == "/srv/data"), "{texts:?}");
    }
    assert_eq!(inputs.get("paths").unwrap(), "/srv/data");
}

/// A module that says something new is heard: a folder chosen with a button
/// joins the list, a saved form comes back, a value is corrected.
///
/// This is how a screen whose fields the module fills works at all — and it did
/// not, because an input that already existed was never written to again.
#[test]
fn a_module_can_change_what_is_in_a_field() {
    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut inputs = HashMap::new();

    shown(&ctx, &mut inputs, "/srv/data");
    assert_eq!(inputs.get("paths").unwrap(), "/srv/data");

    // The module appends a folder and sends the list back.
    let texts = shown(&ctx, &mut inputs, "/srv/data\n/var/log");
    assert_eq!(inputs.get("paths").unwrap(), "/srv/data\n/var/log");
    assert!(
        texts.iter().any(|t| t.contains("/var/log")),
        "the new line is not on screen: {texts:?}"
    );

    // And it stays put while the module keeps saying the same thing.
    for _ in 0..3 {
        shown(&ctx, &mut inputs, "/srv/data\n/var/log");
    }
    assert_eq!(inputs.get("paths").unwrap(), "/srv/data\n/var/log");

    // Typing after that is still the person's.
    inputs.insert("paths".into(), "/elsewhere".into());
    shown(&ctx, &mut inputs, "/srv/data\n/var/log");
    assert_eq!(inputs.get("paths").unwrap(), "/elsewhere");
}
