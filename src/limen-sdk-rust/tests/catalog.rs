//! A module's own translations, and what falls back to what.

use limen_sdk_rust::catalog::*;

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
