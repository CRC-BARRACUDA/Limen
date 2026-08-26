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

/// The licence pop-up in Ukrainian carries a translation the FSF has not
/// approved. The line that says so is the one string in the app that must
/// never be missing — without it the reader is looking at a draft and cannot
/// tell.
#[test]
fn the_unofficial_translation_is_declared_in_both_languages() {
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        let said = t("license.unofficial");
        assert_ne!(said, "license.unofficial", "{lang:?} does not declare it");
        assert!(said.contains("0.9.2"), "{lang:?} does not say which revision");
    }
    set_locale(Lang::En);
}

/// The Ukrainian licence is a file on disk compiled into the binary, so the
/// ways it can go wrong are silent: truncated on save, missing its provenance,
/// or quietly replaced by something that is not the GPL at all.
#[test]
fn the_ukrainian_licence_is_whole_and_says_where_it_came_from() {
    let uk = include_str!("../../../resources/licenses/LICENSE.uk.txt");
    let en = include_str!("../../../LICENSE");
    // The file is wrapped to a fixed column, so any phrase longer than a few
    // words is split across lines. Search the text as one run of words.
    let flat = uk.split_whitespace().collect::<Vec<_>>().join(" ");

    // Provenance, in both languages, before any clause is read.
    assert!(flat.contains("НЕОФІЦІЙНИЙ ПЕРЕКЛАД"), "the Ukrainian notice is gone");
    assert!(
        flat.contains("not officially approved by the Free Software Foundation"),
        "the English notice is gone"
    );
    assert!(flat.contains("Андрій Рисін"), "the translator is not credited");

    // The terms run 0 to 17; a truncated save would take the last of them.
    for section in ["0. Означення", "8. Припинення угоди", "17. Тлумачення"] {
        assert!(flat.contains(section), "missing section: {section}");
    }

    // And it is the same document, not a summary of one: the English runs
    // ~35k characters, so anything under half of that is not a licence.
    assert!(
        uk.len() > en.len() / 2,
        "the translation is {} chars against the original's {}",
        uk.len(),
        en.len()
    );
}
