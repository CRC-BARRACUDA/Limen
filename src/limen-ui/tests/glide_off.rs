//! A travelling value with animations turned off.
//!
//! Its own test binary: the animations switch is global, and flipping it in a
//! file full of tests that expect motion would make them flake.

use limen_ui::*;

/// Animations off means the scale is applied at once — and, just as
/// importantly, that the caller is not asked for another frame to finish a
/// journey that never started.
#[test]
fn with_animations_off_the_scale_is_applied_at_once() {
    set_animations(false);

    let mut z = Glide::resting(100.0);
    assert!(!z.step(200.0, 0.0, 0.28), "nothing left to animate");
    assert_eq!(z.at, 200.0);
    assert_eq!(z.on_grid(2.0), 200.0, "and the lattice has nothing to thin");

    // An odd scale still arrives exactly, grid or no grid.
    assert!(!z.step(125.0, 0.1, 0.28));
    assert_eq!(z.on_grid(2.0), 125.0);
}
