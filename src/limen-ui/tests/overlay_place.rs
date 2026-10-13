//! Where a pop-up lands when the page under it is wider than the window.

use eframe::egui;
use limen_ui::*;

/// Whether this is a pop-up's own panel.
fn is_panel(r: &egui::epaint::RectShape) -> bool {
    let (ink, want) = (r.fill, color::BG_ELEVATED);
    ink.r() == want.r() && ink.g() == want.g() && ink.b() == want.b() && r.rect.width() > 300.0
}

/// The pop-up's own panel, as painted.
fn dialog_rect(ctx: &egui::Context, screen: egui::Rect, bounds: egui::Rect) -> Option<egui::Rect> {
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            time: Some(ctx.input(|i| i.time) + 0.05),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("the page");
            });
            confirm_dialog(
                ctx,
                "probe",
                true,
                "Forget this dictionary?",
                Some("the saved dictionary"),
                "Delete",
                "Cancel",
                Some(bounds),
            );
        },
    );
    out.shapes.iter().rev().find_map(|s| match &s.shape {
        // By colour, not by colour *and* alpha: a pop-up fades as it arrives,
        // so its panel is the same ink at a different strength on every frame
        // of the way in.
        egui::Shape::Rect(r) if is_panel(r) => Some(r.rect),
        _ => None,
    })
}

/// A pop-up is centred on the window, not on a page that runs off the side of
/// it.
///
/// The area a pop-up belongs to is handed in by the app — the tab's content, so
/// that the title bar and the tab strip stay live — and a panel's content rect
/// grows past the window when the page in it is wider than the window. Centred
/// on that, the pop-up walked off the right-hand side: centred on the page
/// rather than on what can be seen of it.
#[test]
fn a_pop_up_is_centred_on_the_window_not_on_a_page_wider_than_it() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1382.0, 838.0));
    // What the app passes when the page overflows: the content, not the window.
    let overflowing = egui::Rect::from_min_max(egui::pos2(0.0, 70.0), egui::pos2(2632.0, 838.0));

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut at = None;
    for _ in 0..4 {
        at = dialog_rect(&ctx, screen, overflowing);
    }
    let at = at.expect("the dialog was drawn");
    assert!(
        (at.center().x - screen.center().x).abs() < 1.0,
        "centred at {:.0}, window centre {:.0}",
        at.center().x,
        screen.center().x
    );
    assert!(at.right() <= screen.right(), "it hangs off the right: {at:?}");
}

/// And it still answers to the area it was given, when that area is inside the
/// window: a pop-up over a tab should not cover the tab strip above it.
#[test]
fn a_pop_up_still_sits_in_the_area_it_was_given() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1382.0, 838.0));
    // The content below a title bar and a tab strip.
    let content = egui::Rect::from_min_max(egui::pos2(0.0, 200.0), egui::pos2(1382.0, 838.0));

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut at = None;
    for _ in 0..4 {
        at = dialog_rect(&ctx, screen, content);
    }
    let at = at.expect("the dialog was drawn");
    assert!(
        (at.center().y - content.center().y).abs() < 1.0,
        "it should be centred on the content ({:.0}), not the window: {at:?}",
        content.center().y
    );
}

/// A module's own pop-up is placed by the same code, and has to answer the
/// same way: it is the one a person sees most.
#[test]
fn a_module_pop_up_is_centred_on_the_window_too() {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1382.0, 838.0));
    let overflowing = egui::Rect::from_min_max(egui::pos2(0.0, 70.0), egui::pos2(2632.0, 838.0));
    let view: View = serde_json::from_value(serde_json::json!({
        "title": "Report",
        "modal": "report.preview",
        "modal_width": 420.0,
        "widgets": [{ "kind": "label", "text": "the document" }],
    }))
    .expect("a pop-up");

    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut inputs = std::collections::HashMap::new();
    let mut at = None;
    for _ in 0..4 {
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(ctx.input(|i| i.time) + 0.05),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("the page");
                });
                modal_layer(ctx, Some(&view), true, 1, Some(overflowing), &mut inputs, None);
            },
        );
        at = out.shapes.iter().rev().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if is_panel(r) => Some(r.rect),
            _ => None,
        });
    }
    let at = at.expect("the pop-up was drawn");
    assert!(
        (at.center().x - screen.center().x).abs() < 1.0,
        "centred at {:.0}, window centre {:.0}",
        at.center().x,
        screen.center().x
    );
}
