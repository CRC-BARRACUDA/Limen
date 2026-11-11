//! A language change with animations turned off.
//!
//! Its own test binary: the animations switch is global, and flipping it where
//! other tests expect a retype would make them flake.

use limen_gui::i18n::{self, Lang};

/// Animations off means the new language is simply there — and, as much to the
/// point, that the frame loop is not asked to draw a change that is not
/// playing.
#[test]
fn with_animations_off_the_language_simply_changes() {
    limen_gui::ui::set_animations(false);
    i18n::set_locale(Lang::En);

    i18n::switch_to(Lang::Uk);
    assert_eq!(i18n::locale(), Lang::Uk);
    assert!(!i18n::switching(), "nothing to play out");
    assert_eq!(i18n::t("nav.about"), "Про програму", "whole, immediately");
}
