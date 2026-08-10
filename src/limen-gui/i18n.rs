//! Host UI localization.
//!
//! Limen's own chrome (nav, About, Settings, Modules, …) is translated through a
//! tiny key/value lookup: strings are keys like `"nav.about"`, resolved against
//! per-locale catalogs embedded from `resources/locales/<code>.toml`. Lookup
//! falls back **active language → English → the key itself**, so a missing
//! translation degrades to English (and, worst case, shows the key — an obvious
//! "please translate me" marker) rather than crashing or blanking.
//!
//! Only the *host's* strings live here. Module-supplied text (the widgets a
//! module returns) is localized by the module itself — the host passes it the
//! active locale over the protocol (a later phase).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// A supported UI language.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    En,
    Uk,
}

impl Lang {
    /// Every language, in display order (used to build the Settings picker).
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Uk];

    /// The BCP-47-ish code stored in settings.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Uk => "uk",
        }
    }

    /// The endonym shown in the language picker.
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Uk => "Українська",
        }
    }

    /// Parse a code (or a fuller locale like `uk_UA.UTF-8`) into a language.
    pub fn from_code(code: &str) -> Option<Lang> {
        match code.get(0..2).unwrap_or("").to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "uk" => Some(Lang::Uk),
            _ => None,
        }
    }
}

static EN_TOML: &str = include_str!("../../resources/locales/en.toml");
static UK_TOML: &str = include_str!("../../resources/locales/uk.toml");

/// The active UI language (English until set from config/detection at startup).
static LOCALE: RwLock<Lang> = RwLock::new(Lang::En);

/// The parsed catalogs, flattened to dotted keys, built once on first use.
fn catalogs() -> &'static HashMap<Lang, HashMap<String, String>> {
    static C: OnceLock<HashMap<Lang, HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert(Lang::En, flatten(EN_TOML));
        m.insert(Lang::Uk, flatten(UK_TOML));
        m
    })
}

/// Flatten a TOML document into `a.b.c` → value for every string leaf.
fn flatten(src: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Ok(val) = src.parse::<toml::Value>() {
        walk(&mut out, String::new(), &val);
    }
    out
}

fn walk(out: &mut HashMap<String, String>, prefix: String, val: &toml::Value) {
    match val {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk(out, key, v);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix, s.clone());
        }
        _ => {}
    }
}

/// Set the active UI language.
pub fn set_locale(lang: Lang) {
    *LOCALE.write().unwrap() = lang;
}

/// The active UI language.
pub fn locale() -> Lang {
    *LOCALE.read().unwrap()
}

/// Translate `key` in the active language, falling back to English then the key.
pub fn t(key: &str) -> String {
    let lang = locale();
    let c = catalogs();
    c.get(&lang)
        .and_then(|m| m.get(key))
        .or_else(|| c.get(&Lang::En).and_then(|m| m.get(key)))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Best-effort default language from the OS locale env vars, else English.
/// (Deliberately dependency-free: on Linux/macOS `LANG`/`LC_*` carry it; on
/// Windows they're usually unset, so it defaults to English and the user picks.)
pub fn detect() -> Lang {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var)
            && let Some(lang) = Lang::from_code(&v)
        {
            return lang;
        }
    }
    Lang::En
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_parse_and_fall_back() {
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
}
