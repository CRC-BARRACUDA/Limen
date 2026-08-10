//! The module search box: a small query language over names, tags and
//! capabilities.

use super::*;

/// Which part of a module a search term looks at.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Field {
    /// The default: identifier and display name only, so a bare word stays
    /// precise instead of matching half the list through its description.
    Name,
    Tag,
    Description,
    Capability,
}

/// One term of a search query: `programs` (name), or `tag:ua` / `description:banned`
/// / `cap:programs.local` for a specific field.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Term {
    pub(crate) field: Field,
    pub(crate) text: String,
}

/// Split a query into terms, resolving `field:value` prefixes.
///
/// Terms are ANDed, so `tag:ua programs` means "tagged ua *and* named programs".
/// An unrecognised prefix is not a field — `foo:bar` is searched literally as a
/// name, so a colon in a name never silently matches nothing.
pub(crate) fn parse_query(query: &str) -> Vec<Term> {
    query
        .split_whitespace()
        .filter_map(|tok| {
            let (field, text) = match tok.split_once(':') {
                Some((k, v)) => match k.to_lowercase().as_str() {
                    "name" => (Field::Name, v),
                    "tag" | "tags" => (Field::Tag, v),
                    "desc" | "description" => (Field::Description, v),
                    "cap" | "capability" | "capabilities" => (Field::Capability, v),
                    _ => (Field::Name, tok),
                },
                None => (Field::Name, tok),
            };
            let text = text.trim().to_lowercase();
            (!text.is_empty()).then_some(Term { field, text })
        })
        .collect()
}

/// Does any of `haystack` contain `needle`?
pub(crate) fn any_contains<'a>(haystack: impl IntoIterator<Item = &'a str>, needle: &str) -> bool {
    haystack
        .into_iter()
        .any(|h| h.to_lowercase().contains(needle))
}

pub(crate) fn module_matches(m: &ModuleSpec, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t.field {
        Field::Name => any_contains(
            [m.name.as_str(), m.display_name.as_deref().unwrap_or("")],
            &t.text,
        ),
        Field::Tag => any_contains(m.tags.iter().map(String::as_str), &t.text),
        Field::Description => any_contains([m.description.as_deref().unwrap_or("")], &t.text),
        Field::Capability => any_contains(m.capabilities.iter().map(String::as_str), &t.text),
    })
}

pub(crate) fn remote_matches(r: &RemoteModule, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t.field {
        Field::Name => any_contains([r.name.as_str(), r.title.as_deref().unwrap_or("")], &t.text),
        Field::Tag => any_contains(r.tags.iter().map(String::as_str), &t.text),
        Field::Description => any_contains([r.description.as_deref().unwrap_or("")], &t.text),
        Field::Capability => any_contains(r.capabilities.iter().map(String::as_str), &t.text),
    })
}

#[cfg(test)]
mod search_tests {
    use super::*;

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
}
