//! Starred modules: what the list does with them, and what it does without them.

use std::collections::HashSet;

use limen_core::{Launch, ModuleSpec};
use limen_gui::app::*;

/// The engine's order, as the list would receive it.
fn installed(names: &[&str]) -> Vec<ModuleSpec> {
    names
        .iter()
        .map(|n| ModuleSpec {
            name: (*n).into(),
            display_name: None,
            version: "1".into(),
            description: None,
            authors: vec![],
            tags: vec![],
            repo: None,
            capabilities: vec![],
            requires: Default::default(),
            optional: Default::default(),
            permissions: Default::default(),
            language: limen_proto::manifest::Language::Native,
            launch: Launch::Native("x".into()),
            cwd: std::path::PathBuf::from("."),
        })
        .collect()
}

fn order(modules: &[ModuleSpec], favorites: &[&str]) -> Vec<String> {
    let set: HashSet<String> = favorites.iter().map(|s| (*s).to_string()).collect();
    favorites_first(modules, &set)
        .into_iter()
        .map(|m| m.name.clone())
        .collect()
}

/// The point of the feature: what you starred is what you see first.
#[test]
fn starred_modules_come_first() {
    let m = installed(&["autoruns", "devices", "loki", "report"]);
    assert_eq!(
        order(&m, &["loki"]),
        ["loki", "autoruns", "devices", "report"]
    );
    // Several stars keep their own relative order too.
    assert_eq!(
        order(&m, &["report", "devices"]),
        ["devices", "report", "autoruns", "loki"]
    );
}

/// Starring one module must move **only** that module.
///
/// The sort has to be stable for this: an unstable one would leave every module
/// present and still shuffle the rest of the page as a side effect of one click,
/// which reads as a bug even though nothing was lost.
#[test]
fn starring_one_module_does_not_reshuffle_the_others() {
    let names = ["autoruns", "chainsaw", "devices", "loki", "programs", "report"];
    let m = installed(&names);
    let before = order(&m, &[]);
    assert_eq!(before, names, "with no stars the engine order is untouched");

    let after = order(&m, &["programs"]);
    assert_eq!(after[0], "programs");
    // Everything else, in the order it was in.
    let rest: Vec<&String> = after[1..].iter().collect();
    let expected: Vec<&String> = before.iter().filter(|n| *n != "programs").collect();
    assert_eq!(rest, expected);
}

/// A star for something that is not installed is not an error.
///
/// The set is a record of what the user marked, not a claim about what exists:
/// removing a module and putting it back has to keep its star, so the name stays
/// in settings while the module is gone.
#[test]
fn a_star_for_a_module_that_is_not_installed_is_ignored() {
    let m = installed(&["devices", "loki"]);
    assert_eq!(order(&m, &["nothing-like-this"]), ["devices", "loki"]);
    // And it still lifts the ones that *are* there.
    assert_eq!(order(&m, &["nothing-like-this", "loki"]), ["loki", "devices"]);
    // An empty list is the ordinary first-run case.
    assert_eq!(order(&installed(&[]), &["loki"]).len(), 0);
}

/// Both languages name the filter chip and the star's tooltip.
///
/// A missing key renders as the key itself, so this is the difference between a
/// chip that reads "Favourites" and one that reads "modules.filter.favorites".
#[test]
fn the_favorites_strings_are_translated() {
    // Each test file is its own binary, so the locale global is this process's
    // alone — no lock needed, unlike the i18n tests where several cases share one.
    use limen_gui::i18n::*;
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        for key in [
            "modules.filter.favorites",
            "modules.none_favorite",
            "modules.favorite_add",
            "modules.favorite_remove",
        ] {
            assert_ne!(t(key), key, "{key} missing for {lang:?}");
        }
    }
    set_locale(Lang::En);
}

// ---- categories ----------------------------------------------------------- //

/// The Modules page narrows to a category by asking the same question the page
/// asks: is this installed module a member? Mirrored here so the rules are
/// pinned even though the loop itself lives inside the draw.
fn in_category(
    modules: &[ModuleSpec],
    categories: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    category: &str,
) -> Vec<String> {
    modules
        .iter()
        .filter(|m| categories.get(category).is_some_and(|ms| ms.contains(&m.name)))
        .map(|m| m.name.clone())
        .collect()
}

fn cats(pairs: &[(&str, &[&str])])
    -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>>
{
    pairs
        .iter()
        .map(|(name, ms)| {
            ((*name).to_string(), ms.iter().map(|s| (*s).to_string()).collect())
        })
        .collect()
}

/// A category shows its members and nothing else.
#[test]
fn a_category_lists_only_its_own_modules() {
    let m = installed(&["crowdstrike", "crowdstrike-devices", "loki", "report"]);
    let c = cats(&[
        ("CrowdStrike", &["crowdstrike", "crowdstrike-devices"]),
        ("Triage", &["loki"]),
    ]);
    assert_eq!(
        in_category(&m, &c, "CrowdStrike"),
        ["crowdstrike", "crowdstrike-devices"]
    );
    assert_eq!(in_category(&m, &c, "Triage"), ["loki"]);
    // A category nobody made is empty rather than a panic.
    assert!(in_category(&m, &c, "Nope").is_empty());
}

/// A module may be in several categories at once — they are views, not folders,
/// so nothing about putting it in one takes it out of another.
#[test]
fn a_module_can_be_in_several_categories() {
    let m = installed(&["loki"]);
    let c = cats(&[("Triage", &["loki"]), ("Scanners", &["loki"])]);
    assert_eq!(in_category(&m, &c, "Triage"), ["loki"]);
    assert_eq!(in_category(&m, &c, "Scanners"), ["loki"]);
}

/// A category may name a module that is no longer installed.
///
/// Removing a module must not silently rewrite the user's categories, so the
/// name stays in settings — and the page simply does not show what is not there.
/// The count in the dropdown has to agree with the list, or it advertises rows
/// that never appear.
#[test]
fn a_category_naming_an_uninstalled_module_shows_only_what_is_there() {
    let m = installed(&["loki"]);
    let c = cats(&[("Triage", &["loki", "eml-analyzer"])]);
    let shown = in_category(&m, &c, "Triage");
    assert_eq!(shown, ["loki"]);

    // The dropdown counts installed members only — the same rule as the list.
    let counted = c["Triage"].iter().filter(|n| m.iter().any(|s| &s.name == *n)).count();
    assert_eq!(counted, shown.len(), "the count promised rows the list will not show");
}

/// Both languages name every category string.
#[test]
fn the_category_strings_are_translated() {
    use limen_gui::i18n::*;
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        for key in [
            "modules.category.button",
            "modules.category.any",
            "modules.category.new",
            "modules.category.delete",
        ] {
            assert_ne!(t(key), key, "{key} missing for {lang:?}");
        }
    }
    set_locale(Lang::En);
}

/// The Categories page and its nav entry are named in both languages.
///
/// A missing key renders as the key itself, so this is the difference between a
/// nav chip reading "Categories" and one reading "nav.categories".
#[test]
fn the_categories_page_strings_are_translated() {
    use limen_gui::i18n::*;
    for lang in [Lang::En, Lang::Uk] {
        set_locale(lang);
        for key in [
            "nav.categories",
            "tab.categories",
            "categories.title",
            "categories.hint",
            "categories.new_hint",
            "categories.add",
            "categories.none",
            "categories.empty",
            "categories.add_module",
            "categories.all_in",
            "categories.remove_module",
            "categories.delete",
            "categories.delete_hint",
            "categories.missing",
        ] {
            assert_ne!(t(key), key, "{key} missing for {lang:?}");
        }
    }
    set_locale(Lang::En);
}

/// The Categories tab titles itself from the catalog like every other tab.
#[test]
fn the_categories_tab_is_titled() {
    use limen_gui::i18n::*;
    set_locale(Lang::En);
    assert_eq!(Tab::Categories.title(), "Categories");
    set_locale(Lang::Uk);
    assert_ne!(Tab::Categories.title(), "tab.categories");
    set_locale(Lang::En);
}
