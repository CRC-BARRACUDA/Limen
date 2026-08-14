//! Easing and the resize curve.

use eframe::egui;
use limen_ui::*;
/// The curve is the point: it must run *past* the target and come back. A
/// monotonic ease would satisfy "it moves and settles" while looking like
/// the thing it replaced.
#[test]
fn the_resize_curve_overshoots_and_settles() {
assert_eq!(ease_out_back(0.0), 0.0);
assert!((ease_out_back(1.0) - 1.0).abs() < 1e-5, "it lands on target");

let peak = (1..100)
    .map(|i| ease_out_back(i as f32 / 100.0))
    .fold(f32::MIN, f32::max);
assert!(peak > 1.0, "it has to overshoot, else it is not `back`");
assert!(peak < 1.2, "but not so far it reads as a glitch: {peak}");

// The overshoot is late in the curve — it arrives fast, then settles.
let quarter = ease_out_back(0.25);
assert!(quarter > 0.5, "most of the distance is covered early: {quarter}");
}
/// Re-targeting mid-flight has to continue from where the value *is*. A
/// pop-up whose size changes twice quickly would otherwise snap back to the
/// first size before starting the second move.
#[test]
fn a_resize_interrupted_by_another_continues_from_where_it_is() {
let ctx = egui::Context::default();
let id = egui::Id::new("t");
let input = egui::RawInput {
    screen_rect: Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800.0, 600.0),
    )),
    ..Default::default()
};
// Settle at 600.
let mut v = 0.0;
for _ in 0..3 {
    let _ = ctx.run(input.clone(), |ctx| v = animate_back(ctx, id, 600.0, 0.22));
}
assert!((v - 600.0).abs() < 0.5);

// Retarget to 300 and stop part-way.
let mut mid = 0.0;
for _ in 0..4 {
    let _ = ctx.run(input.clone(), |ctx| mid = animate_back(ctx, id, 300.0, 0.22));
}
assert!(mid < 600.0 && mid > 300.0, "in flight: {mid}");

// Now retarget again; the next value must be near where it was, not back
// at 600.
let mut after = 0.0;
let _ = ctx.run(input.clone(), |ctx| after = animate_back(ctx, id, 900.0, 0.22));
// It kept moving for that frame, so it is not exactly `mid` — the
// invariant is that it stayed where it had got to rather than snapping
// back to where it started or ahead to where it is going.
assert!(
    (after - mid).abs() < (after - 600.0).abs(),
    "should continue from {mid}, not jump back toward 600 (got {after})"
);
assert!(
    (after - mid).abs() < (after - 900.0).abs(),
    "and not jump ahead to the new target (got {after})"
);
}
