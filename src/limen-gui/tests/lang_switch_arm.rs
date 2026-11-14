//! Arming a language change.
//!
//! Its own test binary: the active language and the change in flight are
//! global, so a test that sets them cannot run beside one that reads them.

use limen_gui::i18n::{self, Lang};

/// The switch arms the change, and `switching` is what keeps frames coming
/// while it plays.
#[test]
fn switching_languages_arms_the_change() {
    limen_gui::ui::set_animations(true);
    i18n::set_locale(Lang::En);
    assert!(!i18n::switching(), "nothing is playing to begin with");

    i18n::switch_to(Lang::Uk);
    assert_eq!(i18n::locale(), Lang::Uk, "the language changes at once");
    assert!(i18n::switching(), "and the retype is playing");

    // Switching to the language already active changes nothing to play.
    i18n::set_locale(Lang::Uk);
    i18n::switch_to(Lang::Uk);
    assert!(!i18n::switching());
}
