//! The catalogs, and what falls back to what.

use limen_gui::i18n::*;

/// The chosen language is one global, so no two tests here may hold an opinion
/// about it at once. Every test that sets or reads it takes this first — one
/// that forgets makes the lock useless for the others, which is how this raced
/// the first time.
#[test]
fn catalogs_parse_and_fall_back() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    // A known key resolves in both languages…
    set_locale(Lang::En);
    assert_eq!(t("nav.modules"), "Modules");
    set_locale(Lang::Uk);
    assert_ne!(t("nav.modules"), "nav.modules"); // has a Ukrainian value
    // …an unknown key falls through to the key itself.
    assert_eq!(t("does.not.exist"), "does.not.exist");
    set_locale(Lang::En);
}

#[test]
fn every_english_key_has_a_ukrainian_translation() {
    let c = catalogs();
    let (en, uk) = (&c[&Lang::En], &c[&Lang::Uk]);
    let missing: Vec<&String> = en.keys().filter(|k| !uk.contains_key(*k)).collect();
    assert!(missing.is_empty(), "Ukrainian catalog missing keys: {missing:?}");
}

/// The chosen language is one global, so a test that sets it cannot run
/// beside another that reads it. They take turns.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Every string the calendar draws comes from the catalog, in both
/// languages. A month or a weekday missing renders as `cal.month_9`, in a
/// grid where there is no room to notice it is not a word.
#[test]
fn the_calendar_is_translated() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        for m in 1..=12 {
            let key = format!("cal.month_{m}");
            let name = t(&key);
            assert_ne!(name, key, "{lang:?}: month {m} is not translated");
            assert!(!name.is_empty());
        }
        for d in 1..=7 {
            let key = format!("cal.wd_{d}");
            assert_ne!(t(&key), key, "{lang:?}: weekday {d}");
        }
        for k in [
            "cal.today",
            "cal.clear",
            // The time picker's own strings: what each pair of digits is,
            // and the two ends of a day.
            "cal.hours",
            "cal.minutes",
            "cal.seconds",
            "cal.day_start",
            "cal.day_end",
        ] {
            assert_ne!(t(k), k, "{lang:?}: {k}");
        }
    }
    set_locale(Lang::En);
}

/// The weekday headings label the columns the days actually land in. Monday
/// first, and 1970-01-01 was a Thursday — get the offset wrong and every
/// date in the grid sits under the wrong heading.
#[test]
fn the_weekday_headings_match_the_grid() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    set_locale(Lang::En);
    // August 2026 opens on a Saturday, which is the sixth column.
    assert_eq!(limen_proto::date::weekday(2026, 8, 1), 5);
    assert_eq!(t("cal.wd_6"), "Sa");
    // ...and February 2024 opens on a Thursday, the fourth.
    assert_eq!(limen_proto::date::weekday(2024, 2, 1), 3);
    assert_eq!(t("cal.wd_4"), "Th");
}

/// The screen a panicked module shows is three strings and a button. A missing
/// one would leave the tab reading `module.inactive` in red.
#[test]
fn the_panic_screen_is_translated() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        for key in [
            "module.inactive",
            "module.show_panic",
            "module.panic_title",
            "module.panic_help",
            "module.failed_start",
        ] {
            assert_ne!(t(key), key, "{lang:?}: {key} is not translated");
        }
    }
    set_locale(Lang::En);
}
