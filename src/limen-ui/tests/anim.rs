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

/// The UI scale travels. Snapping is what this replaces, so a value that is
/// only ever its old self or its new one is the bug, not the feature.
#[test]
fn a_change_of_scale_is_travelled_not_snapped() {
    let mut z = Glide::resting(100.0);
    // The journey begins the frame the goal is set, and runs 0.28s from there.
    z.step(200.0, 0.0, 0.28);
    let seen: Vec<f32> = (1..=16)
        .map(|f| {
            z.step(200.0, f as f64 / 60.0, 0.28);
            z.at
        })
        .collect();
    assert!(
        seen.iter().any(|v| *v > 100.0 && *v < 200.0),
        "it passes through the sizes in between: {seen:?}"
    );
    assert!(
        seen.windows(3).all(|w| w[0] != w[1] || w[1] != w[2]),
        "and never sits still mid-journey: {seen:?}"
    );

    // It arrives, exactly, and then stops asking for frames.
    assert!(z.step(200.0, 0.27, 0.28), "still going just before the end");
    assert!(!z.step(200.0, 0.29, 0.28), "settled just after it");
    assert_eq!(z.at, 200.0, "and lands on the chosen scale, not near it");
}

/// The curve draws back before it sets off and runs past the target before it
/// settles. Both are the point of `easeInOutBack`: a plain ease would satisfy
/// "it moves and arrives" while looking like the thing it replaced.
#[test]
fn the_scale_gathers_itself_then_overshoots() {
    let mut z = Glide::resting(100.0);
    z.step(200.0, 0.0, 1.0);
    let at = |z: &mut Glide, t: f64| {
        z.step(200.0, t, 1.0);
        z.at
    };

    let dip = (1..50).map(|i| at(&mut z, i as f64 / 100.0)).fold(f32::MAX, f32::min);
    assert!(dip < 100.0, "it gathers itself before leaving: {dip}");
    assert!(dip > 85.0, "but not so far it reads as a glitch: {dip}");

    let peak = (50..100)
        .map(|i| at(&mut z, i as f64 / 100.0))
        .fold(f32::MIN, f32::max);
    assert!(peak > 200.0, "it runs past the chosen scale: {peak}");
    assert!(peak < 215.0, "and not by more than it came back by: {peak}");

    // Symmetric: the dip below the start matches the overshoot past the end.
    assert!(
        ((100.0 - dip) - (peak - 200.0)).abs() < 1.0,
        "the two ends mirror each other: {dip} / {peak}"
    );
}

/// Halfway through the time is halfway through the size. The curve leaves and
/// arrives the long way round, but it is symmetric about its middle — a zoom
/// that reached three-quarters of the way in half the time would feel like it
/// had stalled at the end.
#[test]
fn the_scale_is_symmetric_about_its_middle() {
    let mut z = Glide::resting(100.0);
    z.step(200.0, 0.0, 1.0);
    let at = |z: &mut Glide, t: f64| {
        z.step(200.0, t, 1.0);
        z.at
    };
    assert_eq!(at(&mut z, 0.5), 150.0, "halfway in time is halfway in size");
    let (a, b) = (at(&mut z, 0.25), at(&mut z, 0.75));
    assert!(
        ((a - 100.0) - (200.0 - b)).abs() < 0.5,
        "and the two halves mirror each other: {a} / {b}"
    );
}

/// Picking 200% while a 100→150 journey is still running must continue from
/// where the window *is*. Starting the second journey from 100 would make the
/// screen jump back to its old size before growing again.
#[test]
fn picking_another_scale_mid_journey_continues_from_where_it_is() {
    let mut z = Glide::resting(100.0);
    z.step(150.0, 0.0, 0.28);
    z.step(150.0, 0.14, 0.28);
    let interrupted = z.at;
    assert!(
        interrupted > 100.0 && interrupted < 150.0,
        "mid-flight: {interrupted}"
    );

    z.step(200.0, 0.14, 0.28);
    assert_eq!(z.at, interrupted, "the new journey starts where it stood");
    // It gathers itself for the new journey, from where it stood — not from
    // the scale the first journey started at.
    z.step(200.0, 0.16, 0.28);
    assert!(z.at < interrupted, "it draws back first: {}", z.at);
    assert!(z.at > 100.0, "but nowhere near back to the old scale: {}", z.at);
    z.step(200.0, 0.30, 0.28);
    assert!(z.at > interrupted, "then carries on upward: {}", z.at);
    z.step(200.0, 0.42, 0.28);
    assert_eq!(z.at, 200.0, "to the scale picked last");
}

/// The lattice exists to spare font atlases: egui builds one per distinct
/// scale it is handed. What it guarantees is that the count depends on the
/// distance travelled, not on how fast the screen refreshes — a 240Hz monitor
/// must not cost four times the rebuilds a 60Hz one does.
#[test]
fn the_lattice_bounds_the_atlases_by_distance_not_by_frame_rate() {
    let stops = |fps: f64| {
        let mut z = Glide::resting(100.0);
        z.step(125.0, 0.0, 0.28);
        let mut seen = std::collections::BTreeSet::new();
        for f in 1..=(0.3 * fps) as u32 {
            z.step(125.0, f as f64 / fps, 0.28);
            seen.insert(z.on_grid(2.0).to_bits());
        }
        seen
    };
    // 100→125 on a 2% lattice is 13 scales (102…124) plus the goal itself,
    // and a few more for the ground the curve covers outside the two: it dips
    // towards 98 before setting off and passes 126 before settling.
    let fast = stops(240.0);
    assert!(
        fast.len() <= 20,
        "a 25% journey visits the lattice, not every frame: {}",
        fast.len()
    );
    assert!(
        fast.len() >= stops(60.0).len(),
        "the faster screen sees at least as much of the journey"
    );
    assert!(
        fast.iter().all(|b| {
            let v = f32::from_bits(*b);
            v == 125.0 || (v / 2.0).fract() == 0.0
        }),
        "and every scale along the way is on the lattice or is the goal"
    );
}

/// The lattice must not decide where the zoom comes to rest: 125% is not on a
/// 2% grid, and 124% is the wrong size to be left at.
#[test]
fn the_lattice_never_moves_the_end_of_the_journey() {
    let mut z = Glide::resting(100.0);
    z.step(125.0, 0.0, 0.28);
    z.step(125.0, 1.0, 0.28);
    assert_eq!(z.at, 125.0);
    assert_eq!(z.on_grid(2.0), 125.0, "it rests on exactly what was asked");
}
