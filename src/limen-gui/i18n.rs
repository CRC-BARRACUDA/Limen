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
use std::time::Instant;

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
pub fn catalogs() -> &'static HashMap<Lang, HashMap<String, String>> {
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

/// Set the active UI language, with no transition.
pub fn set_locale(lang: Lang) {
    *SWITCH.write().unwrap() = None;
    *LOCALE.write().unwrap() = lang;
}

// ---- a language change, played out as text --------------------------------- //

/// How long the old text takes to be erased, and the new text to be typed.
///
/// Backspace held down is faster than typing, so the two differ; together they
/// are short enough that the screen is readable again before anyone reaches for
/// the mouse.
const ERASE_SECS: f64 = 0.16;
const TYPE_SECS: f64 = 0.26;

/// A language change in flight: the language being left, and when it started.
struct Switch {
    from: Lang,
    at: Instant,
}

static SWITCH: RwLock<Option<Switch>> = RwLock::new(None);

/// Where a language change is, `elapsed` seconds in: how much of the old text
/// is still on screen, then how much of the new text has been typed. `None`
/// once it is over.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Retype {
    /// The old language, with this share of each string left.
    Erasing(f32),
    /// The new language, with this share of each string typed.
    Typing(f32),
}

/// [`Retype`] at `elapsed` seconds, or `None` if the change has played out.
pub fn retype_at(elapsed: f64) -> Option<Retype> {
    if elapsed < 0.0 {
        return None;
    }
    if elapsed < ERASE_SECS {
        // Erasing *down* to nothing, so the share counts backwards.
        return Some(Retype::Erasing(1.0 - (elapsed / ERASE_SECS) as f32));
    }
    let typing = (elapsed - ERASE_SECS) / TYPE_SECS;
    if typing < 1.0 {
        return Some(Retype::Typing(typing as f32));
    }
    None
}

/// Change the active language, playing the change out as a retype.
///
/// The erase has to show the language being *left*, which is no longer the
/// active one — so the language it came from is kept here for as long as the
/// change takes.
pub fn switch_to(lang: Lang) {
    let from = locale();
    *LOCALE.write().unwrap() = lang;
    *SWITCH.write().unwrap() = if from == lang || !crate::ui::animations_enabled() {
        None
    } else {
        Some(Switch {
            from,
            at: Instant::now(),
        })
    };
}

/// Whether a language change is still playing — the frame loop keeps asking to
/// be drawn while it is, since nothing else is driving the clock.
pub fn switching() -> bool {
    SWITCH
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|s| retype_at(s.at.elapsed().as_secs_f64()).is_some())
}

/// `key` as it reads `elapsed` seconds into a change from `from` to `to` — the
/// whole of the new text once the change has played out.
///
/// Both languages are arguments rather than read from the global: this is the
/// whole of the behaviour, and a function that takes its inputs can be tested
/// without a screen or a clock.
pub fn t_at(key: &str, from: Lang, to: Lang, elapsed: f64) -> String {
    match retype_at(elapsed) {
        None => look_up(to, key),
        Some(Retype::Erasing(share)) => cut(look_up(from, key), share),
        Some(Retype::Typing(share)) => cut(look_up(to, key), share),
    }
}

/// `text` truncated to `share` of its characters.
fn cut(mut text: String, share: f32) -> String {
    text.truncate(crate::ui::share_len(&text, share));
    text
}

/// The active UI language.
pub fn locale() -> Lang {
    *LOCALE.read().unwrap()
}

/// Translate `key` in the active language, falling back to English then the key.
///
/// While a language change is playing, this is the string part-way through
/// being erased or retyped: every call site in the chrome goes through here, so
/// the whole screen does it together without any of them knowing.
pub fn t(key: &str) -> String {
    match SWITCH.read().unwrap().as_ref() {
        None => look_up(locale(), key),
        Some(s) => t_at(key, s.from, locale(), s.at.elapsed().as_secs_f64()),
    }
}

/// `key` in `lang`, falling back to English then to the key itself.
fn look_up(lang: Lang, key: &str) -> String {
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
