//! A module's manifest: what it provides, needs and is allowed to do.

use limen_proto::manifest::*;

#[test]
fn parses_a_full_manifest() {
    let m = Manifest::from_toml_str(
        r#"
        [module]
        name = "usb"
        version = "0.1.0"
        language = "python"
        entry = "main.py"

        [provides]
        capabilities = ["usb.enumerate"]

        [requires.capabilities]
        "crowdstrike.rtr" = ">=1.0"
        "#,
    )
    .unwrap();

    assert_eq!(m.module.name, "usb");
    assert_eq!(m.module.language, Language::Python);
    assert_eq!(m.module.abi, Abi::Rpc); // defaulted
    assert_eq!(m.provides.capabilities, vec!["usb.enumerate"]);
    assert_eq!(m.requires.capabilities["crowdstrike.rtr"], ">=1.0");
    // No [permissions] table => nothing sensitive.
    assert!(!m.permissions.sensitive());
    // Absent `tags` is an empty list, never an error — every manifest
    // predating the field must still parse.
    assert!(m.module.tags.is_empty());
}

/// Tags are optional metadata the module manager groups and filters by.
#[test]
fn parses_tags() {
    let m = Manifest::from_toml_str(
        r#"
        [module]
        name = "usb"
        version = "0.1.0"
        language = "python"
        entry = "main.py"
        tags = ["security", "inventory"]
        "#,
    )
    .unwrap();

    assert_eq!(m.module.tags, vec!["security", "inventory"]);
}

#[test]
fn parses_permissions_and_flags_sensitive() {
    let m = Manifest::from_toml_str(
        r#"
        [module]
        name = "crowdstrike"
        version = "0.1.0"
        language = "native"
        entry = "crowdstrike"
        abi = "native"

        [permissions]
        run_hosts = true
        network = ["api.crowdstrike.com"]
        "#,
    )
    .unwrap();

    assert!(m.permissions.run_hosts);
    assert!(m.permissions.sensitive());
    assert!(m.permissions.summary().iter().any(|s| s.contains("fleet hosts")));
}

#[test]
fn admin_is_sensitive_and_listed_first() {
    let m = Manifest::from_toml_str(
        "[module]\nname=\"a\"\nversion=\"0.1.0\"\nlanguage=\"python\"\nentry=\"m.py\"\n\
         [permissions]\nadmin = true\n",
    )
    .unwrap();
    assert!(m.permissions.admin);
    assert!(m.permissions.sensitive());
    assert_eq!(m.permissions.summary().first().map(String::as_str), Some("administrator privileges"));
}

/// Elevation is opt-in and must be visible. A module that never mentions it
/// cannot ask for root, and one that does has to show up on the consent
/// screen saying so — otherwise the permission is a formality.
#[test]
fn elevation_is_declared_or_refused() {
    let silent: Permissions = toml::from_str("subprocess = true").unwrap();
    assert!(!silent.elevate, "not asking is the default");
    assert!(!silent.summary().iter().any(|p| p.contains("administrator")));

    let asking: Permissions = toml::from_str("subprocess = true\nelevate = true").unwrap();
    assert!(asking.elevate);
    assert!(asking.sensitive(), "it must require consent");
    assert!(
        asking
            .summary()
            .iter()
            .any(|p| p == "run commands as administrator"),
        "and say so in words the consent screen shows: {:?}",
        asking.summary()
    );
}

/// A module's card is translated from the module's own catalog, and modules keep
/// that catalog in one of two places: `resources/locales/` once sources and data
/// are separated, `locales/` before that. Both must be read.
///
/// The failure this guards against is quiet. A module's own screens embed their
/// catalog at compile time, so moving the files leaves every screen it draws
/// still translated while its card alone reverts to English — which looks like a
/// missing translation rather than a lookup that stopped finding one.
#[test]
fn a_card_is_translated_from_either_place_a_module_keeps_its_catalog() {
    let base = std::env::temp_dir().join("limen-manifest-locales-test");
    let _ = std::fs::remove_dir_all(&base);

    let catalog = "[module]\ntitle = \"Пила\"\ndescription = \"Опис українською\"\n";
    for (case, sub) in [("new", "resources/locales"), ("old", "locales")] {
        let dir = base.join(case);
        let locales = dir.join(sub);
        std::fs::create_dir_all(&locales).expect("make the catalog directory");
        std::fs::write(locales.join("uk.toml"), catalog).expect("write the catalog");

        assert_eq!(
            localized_description(&dir, "uk").as_deref(),
            Some("Опис українською"),
            "{case} layout: the description was not found"
        );
        assert_eq!(
            localized_title(&dir, "uk").as_deref(),
            Some("Пила"),
            "{case} layout: the title was not found"
        );
        // English is the manifest's own language, so there is nothing to look up.
        assert_eq!(localized_description(&dir, "en"), None);
        // A language the module does not speak leaves the manifest's default.
        assert_eq!(localized_description(&dir, "fr"), None);
    }

    let _ = std::fs::remove_dir_all(&base);
}
