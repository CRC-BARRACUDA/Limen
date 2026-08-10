//! Easing, hover and press factors, and the entrance timings. Every moving thing
//! in the toolkit gets its numbers from here.

use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether UI animations are enabled — toggled from Settings, on by default.
pub static ANIMATIONS: AtomicBool = AtomicBool::new(true);

/// Enable/disable UI animations globally (Settings checkbox + startup config).
pub fn set_animations(on: bool) {
    ANIMATIONS.store(on, Ordering::Relaxed);
}

/// Whether animations are currently on.
pub fn animations_enabled() -> bool {
    ANIMATIONS.load(Ordering::Relaxed)
}

// ---- easing ---------------------------------------------------------------- //

/// Ease-out cubic: fast start, gentle finish — for entrances.
pub fn ease_out(t: f32) -> f32 {
    let i = 1.0 - t;
    1.0 - i * i * i
}

/// Smoothstep: gentle at both ends — for reversible motion / collapses.
pub fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

// ---- colour ---------------------------------------------------------------- //

/// Eased 0→1 for a boolean (hover, selection, …) keyed by `id`. Snaps to the
/// target when animations are off. The building block for the rest.
pub fn anim_bool(ui: &egui::Ui, id: egui::Id, on: bool, time: f32) -> f32 {
    if animations_enabled() {
        ui.ctx().animate_bool_with_time(id, on, time)
    } else {
        on as u8 as f32
    }
}

/// Animated hover + press factors (0..1) for an interactive response — the base
/// for any custom animated widget: allocate a rect, then paint from these.
pub struct Interact {
    pub hover: f32,
    pub press: f32,
}

/// Compute the [`Interact`] factors for `resp`.
pub fn interact(ui: &egui::Ui, resp: &egui::Response) -> Interact {
    Interact {
        hover: anim_bool(ui, resp.id.with("hover"), resp.hovered(), 0.16),
        press: anim_bool(
            ui,
            resp.id.with("press"),
            resp.is_pointer_button_down_on(),
            0.06,
        ),
    }
}

// ---- entrance -------------------------------------------------------------- //

/// Staggered entrance factor (eased 0→1) for the `index`-th item revealed at
/// `start_at`, evaluated at `now`, with `stagger`/`dur` seconds. Requests
/// repaints until it settles; returns 1 instantly when animations are off. Apply
/// it however you like (opacity, slide, …).
pub fn reveal_t(
    ui: &egui::Ui,
    index: usize,
    start_at: f64,
    now: f64,
    stagger: f64,
    dur: f64,
) -> f32 {
    if !animations_enabled() {
        return 1.0;
    }
    let start = start_at + index as f64 * stagger;
    let t = ease_out((((now - start) / dur).clamp(0.0, 1.0)) as f32);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    t
}

// ---- typing ---------------------------------------------------------------- //

/// The overshoot curve: runs past the target and settles back onto it.
///
/// The constant is the usual one — a ~10% overshoot, enough to read as weight
/// without looking like a bug.
pub fn ease_out_back(t: f32) -> f32 {
    const C1: f32 = 1.70158;
    const C3: f32 = C1 + 1.0;
    let u = t - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// Where an eased value is in its journey.
#[derive(Clone, Copy)]
pub struct Eased {
    from: f32,
    to: f32,
    start: f64,
}

/// Animate a value toward `target` along [`ease_out_back`].
///
/// egui's own `animate_value_with_time` interpolates monotonically and so cannot
/// overshoot; this keeps its own from/to/start instead. Re-targeting mid-flight
/// starts from wherever the value currently *is*, so a size that changes twice
/// in quick succession does not jump back to the old one first.
pub fn animate_back(ctx: &egui::Context, id: egui::Id, target: f32, duration: f32) -> f32 {
    if !animations_enabled() {
        return target;
    }
    // Read the clock before taking the data lock — both lock the same Context.
    let now = ctx.input(|i| i.time);
    let state = ctx.data_mut(|d| d.get_temp::<Eased>(id));
    let (value, running) = match state {
        Some(a) => {
            let p = (((now - a.start) as f32) / duration.max(0.001)).clamp(0.0, 1.0);
            (a.from + (a.to - a.from) * ease_out_back(p), p < 1.0)
        }
        // First sight of this value: start where it is asked to be, so opening
        // does not fly in from zero.
        None => (target, false),
    };
    let retarget = state.is_none_or(|a| (a.to - target).abs() > 0.5);
    if retarget {
        ctx.data_mut(|d| {
            d.insert_temp(
                id,
                Eased {
                    from: value,
                    to: target,
                    start: now,
                },
            )
        });
    }
    if running || retarget {
        // Nothing else is driving the clock between frames, so ask.
        ctx.request_repaint();
    }
    value
}

/// Scale an overlay about its own centre as it arrives and leaves.
///
/// Applied to the layer at paint time rather than by changing the box's width,
/// so nothing inside reflows while it animates — text that re-wraps mid-fade
/// reads as a glitch. `t` is the same 0→1 the opacity uses, so the scale, the
/// fade and the rise are one motion.
pub fn pop_layer(ctx: &egui::Context, layer: egui::LayerId, rect: egui::Rect, t: f32) {
    if !animations_enabled() {
        return;
    }
    let s = 0.92 + 0.08 * ease_out_back(t);
    if (s - 1.0).abs() < 0.0005 {
        return;
    }
    // Scaling about a point: p' = s·p + c·(1 − s) keeps `c` where it is.
    let c = rect.center().to_vec2();
    ctx.set_transform_layer(layer, egui::emath::TSTransform::new(c * (1.0 - s), s));
}
