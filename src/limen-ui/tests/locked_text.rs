//! A text field locked by a checkbox: read-only while the box is ticked.

use std::collections::HashMap;

use eframe::egui;
use limen_ui::*;

fn form() -> View {
    serde_json::from_value(serde_json::json!({
        "title": "Edit Wi-Fi",
        "widgets": [
            {"kind": "checkbox", "id": "net.dhcp", "label": "Get the address from DHCP", "default": true},
            {"kind": "text", "id": "net.ipv4", "label": "IPv4", "default": "10.0.0.5/24",
             "locked_by": "net.dhcp"}
        ]
    }))
    .unwrap()
}

/// Draw the form for one frame with `events`, and say where the field's text is.
fn frame(ctx: &egui::Context, inputs: &mut HashMap<String, String>, time: f64,
         events: Vec<egui::Event>) -> Option<egui::Pos2> {
    let view = form();
    let out = ctx.run(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_view(ui, &view, inputs, None, 0.0);
            });
        },
    );
    out.shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text().starts_with("10.0.0.") => Some(t.pos),
        _ => None,
    })
}

/// Click into the field and type into it; what the field holds afterwards.
fn type_into(inputs: &mut HashMap<String, String>) -> String {
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut time = 0.0;
    let mut at = None;
    for _ in 0..30 {
        time += 1.0 / 60.0;
        at = frame(&ctx, inputs, time, vec![]).or(at);
    }
    let at = at.expect("the field's value is drawn") + egui::vec2(30.0, 6.0);
    for pressed in [true, false] {
        time += 1.0 / 60.0;
        frame(&ctx, inputs, time, vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary,
                                         pressed, modifiers: Default::default() },
        ]);
    }
    time += 1.0 / 60.0;
    frame(&ctx, inputs, time, vec![egui::Event::Text("9".into())]);
    inputs.get("net.ipv4").cloned().unwrap_or_default()
}

/// While "Get the address from DHCP" is ticked, the address cannot be typed
/// into: the box says where the address comes from, and an address typed under
/// it would be one the host never uses.
#[test]
fn a_field_locked_by_a_ticked_box_takes_no_input() {
    let mut inputs = HashMap::new();
    assert_eq!(type_into(&mut inputs), "10.0.0.5/24", "typing into it changed nothing");
}

/// Untick the box and the same field takes input again — the lock follows the
/// box, not the screen it was drawn with.
#[test]
fn unticking_the_box_frees_the_field() {
    let mut inputs = HashMap::new();
    inputs.insert("net.dhcp".to_string(), "false".to_string());
    let typed = type_into(&mut inputs);
    assert_ne!(typed, "10.0.0.5/24", "the field took the typing");
    assert!(typed.contains('9'), "and it is what was typed: {typed:?}");
}
