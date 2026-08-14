//! Fonts and the style everything is drawn against.

use eframe::egui;
use limen_ui::*;
/// The UI font must actually parse and lay out — egui panics on a malformed
/// face when it builds the atlas, so one headless frame covering both
/// alphabets and both families proves JetBrains Mono loads and serves them.
/// Unlike the Latin-only faces it replaced, the Cyrillic here comes from the
/// font itself rather than egui's bundled fallback.
#[test]
fn ui_font_loads_and_lays_out_both_alphabets() {
let ctx = egui::Context::default();
install_fonts(&ctx);
let _ = ctx.run(egui::RawInput::default(), |ctx| {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.label("Limen HUD 0123");
        ui.monospace("path/to/mod.rs");
        ui.label("Заборонене ПЗ — Встановлені модулі");
    });
});
}
