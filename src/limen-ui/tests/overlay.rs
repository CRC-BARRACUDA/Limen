//! Pop-up windows and the dialogs built on them.

use serde_json::Value;
use std::collections::HashMap;
use eframe::egui;
use limen_ui::*;
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
