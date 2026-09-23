//! The Modules page's filter row: the chips and the categories dropdown share it,
//! so they have to sit on the same line.

use eframe::egui;
use limen_gui::ui;

/// Lay the row out headlessly and return each control's rect.
fn row() -> (egui::Rect, egui::Rect, egui::Rect) {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let opts: Vec<String> = vec!["All categories".into(), "CrowdStrike  (5)".into()];
    let (mut chip, mut ours, mut theirs) =
        (egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING);
    // Two frames: egui settles sizes it had to measure on the first.
    for _ in 0..2 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 400.0),
            )),
            ..Default::default()
        };
        // The frame output is not needed here; only where the widgets landed.
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    chip = ui::chip(ui, "All", true).rect;
                    let mut v = "All categories".to_string();
                    ours = ui::dropdown(ui, "dd", &mut v, &opts).rect;
                    theirs = egui::ComboBox::from_id_source("cb")
                        .selected_text("All categories")
                        .show_ui(ui, |_| {})
                        .response
                        .rect;
                });
            });
        });
    }
    (chip, ours, theirs)
}

/// The categories dropdown must sit on the chips' line.
///
/// It first shipped as an `egui::ComboBox`, which places its own frame from the
/// cursor rather than being centred with its neighbours, and sat five pixels low
/// — visible immediately, and invisible in the source. `ui::dropdown` is the
/// house widget and is allocated like the chips, so it lines up.
#[test]
fn the_category_dropdown_sits_on_the_chip_line() {
    let (chip, ours, _) = row();
    assert_eq!(
        ours.height(),
        chip.height(),
        "the dropdown is {}px tall against the chips' {}px",
        ours.height(),
        chip.height()
    );
    let off = (ours.center().y - chip.center().y).abs();
    assert!(off < 1.0, "the dropdown's centre is {off}px off the chips'");
}

/// And the widget it replaced really was the one that did not line up.
///
/// Kept so the reason is a measurement rather than a memory: if some future egui
/// centres its `ComboBox` after all, this fails and the comment above can go.
#[test]
fn egui_combo_box_is_the_one_that_does_not_line_up() {
    let (chip, _, theirs) = row();
    let off = (theirs.center().y - chip.center().y).abs();
    assert!(
        off > 1.0,
        "egui's ComboBox now lines up ({off}px off) — the house dropdown is no \
         longer needed for alignment, only for the animation"
    );
}
