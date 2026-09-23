//! Settings: what survives a save, and what an older file still loads as.

use std::collections::BTreeMap;

use limen_core::Config;

/// Each test gets its own LIMEN_HOME, because `Config` reads a process-wide path
/// and these would otherwise overwrite each other's file.
fn in_temp_home<T>(tag: &str, body: impl FnOnce() -> T) -> T {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("limen-cfg-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the temp home");
    // SAFETY: the lock above means no other test in this binary is reading or
    // writing the environment while this runs.
    unsafe { std::env::set_var("LIMEN_HOME", &dir) };
    let out = body();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Stars and categories have to come back exactly as they went in: they are the
/// user's own arrangement, and there is nothing to recover them from.
#[test]
fn favorites_and_categories_survive_a_round_trip() {
    in_temp_home("roundtrip", || {
        // Built from `Default` so this test keeps compiling when a field is
        // added, rather than becoming a list nobody remembers to extend.
        let cfg = Config {
            favorites: vec!["loki".into(), "crowdstrike-devices".into()],
            categories: BTreeMap::from([
                ("CrowdStrike".to_string(), vec!["crowdstrike".to_string(),
                                                 "crowdstrike-devices".to_string()]),
                ("Triage".to_string(), vec!["loki".to_string()]),
            ]),
            ..Config::default()
        };
        cfg.save().expect("save");

        let back = Config::load().expect("load");
        assert_eq!(back.favorites, cfg.favorites);
        assert_eq!(back.categories, cfg.categories);
        // A category with nobody in it is still a category — it is what you get
        // the moment you create one, and losing it on reload would make the
        // "new category" button look broken.
        let mut cfg2 = back;
        cfg2.categories.insert("Empty".into(), vec![]);
        cfg2.save().expect("save again");
        assert_eq!(Config::load().expect("load").categories["Empty"], Vec::<String>::new());
    });
}

/// A settings file written before these fields existed must still load.
///
/// This is the upgrade path for every install already out there: without the
/// serde defaults, one unknown-to-them field turns into a parse error and the
/// app comes up with no settings at all.
#[test]
fn a_settings_file_from_an_older_version_still_loads() {
    in_temp_home("older", || {
        let path = limen_core::paths::settings_path();
        std::fs::write(
            &path,
            r#"{"module_dirs":[],"ui_scale_percent":125,"animations":false,"alerts":true}"#,
        )
        .expect("write an old settings file");

        let cfg = Config::load().expect("an older file still loads");
        assert_eq!(cfg.ui_scale_percent, 125, "existing settings are kept");
        assert!(!cfg.animations);
        assert!(cfg.favorites.is_empty(), "absent means none, not an error");
        assert!(cfg.categories.is_empty());
    });
}
