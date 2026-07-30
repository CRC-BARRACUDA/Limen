//! The active UI locale, shared process-wide.
//!
//! The host application (the GUI) sets it; it's read by the module host (for the
//! `host.locale` callback a running module queries) and by the registry (to pick
//! a localized module description when listing available modules). Kept here, in
//! the dependency-light contract crate, so every layer can reach it without a
//! dependency cycle.

use std::sync::RwLock;

static LOCALE: RwLock<String> = RwLock::new(String::new());

/// Set the active locale code (e.g. `"en"`, `"uk"`). Empty clears it to default.
pub fn set(code: impl Into<String>) {
    *LOCALE.write().unwrap() = code.into();
}

/// The active locale code, or `"en"` when unset.
pub fn current() -> String {
    let l = LOCALE.read().unwrap();
    if l.is_empty() {
        "en".to_string()
    } else {
        l.clone()
    }
}
