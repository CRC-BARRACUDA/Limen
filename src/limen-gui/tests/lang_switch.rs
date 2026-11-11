//! A language change, played out as a retype.
//!
//! Its own test binary: the active language and the change in flight are both
//! global, and a half-erased catalogue would fail every other test that reads a
//! string.

use limen_gui::i18n::{self, Lang, Retype};

// Every one of these asks `t_at` directly, with both languages named, so no
// test here depends on — or disturbs — the active language.

/// The shape of the change: the old text comes off, then the new text goes on,
/// and then it is over. A phase that never ends would leave the screen stuck
/// mid-word.
#[test]
fn the_change_erases_then_types_then_finishes() {
    assert_eq!(i18n::retype_at(0.0), Some(Retype::Erasing(1.0)), "all there");
    match i18n::retype_at(0.08) {
        Some(Retype::Erasing(share)) => {
            assert!(share > 0.0 && share < 1.0, "part-way erased: {share}")
        }
        other => panic!("still erasing at 0.08s, not {other:?}"),
    }
    match i18n::retype_at(0.20) {
        Some(Retype::Typing(share)) => {
            assert!(share > 0.0 && share < 1.0, "part-way typed: {share}")
        }
        other => panic!("typing at 0.20s, not {other:?}"),
    }
    assert_eq!(i18n::retype_at(10.0), None, "and it ends");
    assert_eq!(i18n::retype_at(-1.0), None, "and has no business before it");
}

/// What is erased is the language being *left*. The active language is already
/// the new one by then, so reading the catalogue the obvious way would backspace
/// through text nobody ever saw — the one mistake here that looks almost right.
#[test]
fn what_is_erased_is_the_old_language_and_what_is_typed_is_the_new() {
    let (en, uk) = ("About", "Про програму");
    let at = |elapsed: f64| i18n::t_at("nav.about", Lang::En, Lang::Uk, elapsed);

    assert_eq!(at(0.0), en, "starts as it was");

    let erasing = at(0.08);
    assert!(
        en.starts_with(&erasing) && erasing != en,
        "mid-erase it is a shortened {en:?}, got {erasing:?}"
    );
    assert!(
        !erasing.is_empty() && !uk.starts_with(&erasing),
        "and not the new language yet: {erasing:?}"
    );

    let typing = at(0.20);
    assert!(
        uk.starts_with(&typing) && typing != uk,
        "mid-type it is a part of {uk:?}, got {typing:?}"
    );

    assert_eq!(at(10.0), uk, "ends as asked");
}

/// Every string on screen finishes together: the change is a share of each
/// label, not a count of characters, so a long heading does not still be
/// typing itself after the nav has settled.
#[test]
fn long_and_short_strings_finish_together() {
    let at = |elapsed: f64| {
        (
            i18n::t_at("nav.about", Lang::Uk, Lang::En, elapsed),
            i18n::t_at("settings.ui_scale_hint", Lang::Uk, Lang::En, elapsed),
        )
    };
    let (short_full, long_full) = at(10.0);
    assert!(
        long_full.chars().count() > short_full.chars().count() * 3,
        "the fixture needs a long string and a short one: {long_full:?}"
    );

    // Part-way through typing, both are part-way through — and the same part.
    let (short, long) = at(0.16 + 0.26 * 0.5);
    let shares = (
        short.chars().count() as f32 / short_full.chars().count() as f32,
        long.chars().count() as f32 / long_full.chars().count() as f32,
    );
    assert!(
        (shares.0 - shares.1).abs() < 0.1,
        "the two are the same share in: {shares:?}"
    );

    // And neither is finished while there is still a way to go. Only "still":
    // a share is rounded to whole characters, so a five-letter label is
    // complete once the share passes 0.9 — a frame or two early, which is
    // nobody's idea of the screen settling in pieces.
    let (short, long) = at(0.16 + 0.26 * 0.7);
    assert!(
        short != short_full && long != long_full,
        "nothing settles early: {short:?} / {long:?}"
    );
}

