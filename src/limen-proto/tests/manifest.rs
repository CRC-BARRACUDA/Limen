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
