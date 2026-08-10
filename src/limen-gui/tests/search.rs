//! The module search box's query language.

use limen_registry::RemoteModule;
use limen_gui::app::*;

/// The remote and installed matchers share their term logic, so the simpler
/// struct exercises both. `RemoteModule` needs no launch/permission setup.
fn spec() -> RemoteModule {
    RemoteModule {
        name: "programs-banlist-ua".into(),
        title: Some("Banned Programs in Ukraine".into()),
        repo: "CRC-BARRACUDA/limen-programs-banlist-ua".into(),
        description: Some("Flags installed software banned in Ukraine.".into()),
        url: String::new(),
        version: Some("0.1.0".into()),
        capabilities: vec!["banlist.ua".into()],
        tags: ["ua", "programs", "ssscip.banlist"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        branch: None,
        commit: None,
    }
}

#[test]
fn a_bare_word_searches_names_only() {
    // "banned" is in the display name -> hit.
    assert!(remote_matches(&spec(), &parse_query("banned")));
    // "software" appears only in the description -> miss, by design.
    assert!(!remote_matches(&spec(), &parse_query("software")));
}

#[test]
fn qualifiers_target_their_field() {
    let m = spec();
    assert!(remote_matches(&m, &parse_query("tag:ua")));
    assert!(remote_matches(&m, &parse_query("description:software")));
    assert!(remote_matches(&m, &parse_query("cap:banlist")));
    assert!(!remote_matches(&m, &parse_query("tag:windows")));
    // A tag value is not a name, and vice versa.
    assert!(!remote_matches(&m, &parse_query("name:ssscip")));
    assert!(remote_matches(&m, &parse_query("tag:ssscip")));
}

#[test]
fn terms_are_anded_and_shorthands_work() {
    let m = spec();
    assert!(remote_matches(&m, &parse_query("tag:ua programs")));
    // Second term fails -> whole query fails.
    assert!(!remote_matches(&m, &parse_query("tag:ua nonsense")));
    assert!(remote_matches(&m, &parse_query("desc:banned")));
}

#[test]
fn an_empty_query_matches_everything() {
    assert!(parse_query("   ").is_empty());
    assert!(remote_matches(&spec(), &[]));
}

/// An unknown prefix must not be read as a field, or it would silently match
/// nothing instead of being searched literally.
#[test]
fn an_unknown_prefix_is_searched_literally() {
    let terms = parse_query("weird:thing");
    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].field, Field::Name);
    assert_eq!(terms[0].text, "weird:thing");
}
