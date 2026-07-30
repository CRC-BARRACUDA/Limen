//! A tiny translation catalog for modules.
//!
//! A module ships its own strings as embedded per-language TOML documents (the
//! same shape as the host's `resources/locales/*.toml`) and looks them up by key.
//! Query [`Host::locale`](crate::Host::locale) for the active language, then
//! [`Catalog::tr`] to translate — falling back to the default language, then the
//! key itself.
//!
//! ```ignore
//! use std::sync::OnceLock;
//! use limen_sdk_rust::Catalog;
//!
//! fn cat() -> &'static Catalog {
//!     static C: OnceLock<Catalog> = OnceLock::new();
//!     C.get_or_init(|| Catalog::new(&[
//!         ("en", include_str!("locales/en.toml")),
//!         ("uk", include_str!("locales/uk.toml")),
//!     ]))
//! }
//!
//! // in invoke():
//! let lang = host.locale();
//! let title = cat().tr(&lang, "scan.title");
//! ```

use std::collections::HashMap;

/// A module's per-language string tables.
pub struct Catalog {
    langs: HashMap<String, HashMap<String, String>>,
    default: String,
}

impl Catalog {
    /// Build from `(code, toml_source)` pairs. The **first** pair is the default
    /// (fallback) language.
    pub fn new(entries: &[(&str, &str)]) -> Self {
        let default = entries
            .first()
            .map(|(c, _)| (*c).to_string())
            .unwrap_or_else(|| "en".to_string());
        let langs = entries
            .iter()
            .map(|(code, src)| ((*code).to_string(), flatten(src)))
            .collect();
        Self { langs, default }
    }

    /// Translate `key` for `lang`, falling back to the default language then the
    /// key. `lang` may be a full locale (`"uk_UA"`); only the leading code is used.
    pub fn tr(&self, lang: &str, key: &str) -> String {
        let short = lang.get(0..2).unwrap_or(lang);
        self.langs
            .get(short)
            .or_else(|| self.langs.get(lang))
            .and_then(|m| m.get(key))
            .or_else(|| self.langs.get(&self.default).and_then(|m| m.get(key)))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_falls_back_default_then_key() {
        let c = Catalog::new(&[
            ("en", "[scan]\ntitle = \"Scan\""),
            ("uk", "[scan]\ntitle = \"Сканувати\""),
        ]);
        assert_eq!(c.tr("uk", "scan.title"), "Сканувати");
        assert_eq!(c.tr("uk_UA.UTF-8", "scan.title"), "Сканувати"); // locale suffix ignored
        assert_eq!(c.tr("fr", "scan.title"), "Scan"); // unknown lang → default
        assert_eq!(c.tr("uk", "missing.key"), "missing.key"); // unknown key → key
    }
}
