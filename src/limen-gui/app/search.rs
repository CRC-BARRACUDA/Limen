//! The module search box: a small query language over names, tags and
//! capabilities.

use super::*;

/// Which part of a module a search term looks at.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field {
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
pub struct Term {
    pub field: Field,
    pub text: String,
}

/// Split a query into terms, resolving `field:value` prefixes.
///
/// Terms are ANDed, so `tag:ua programs` means "tagged ua *and* named programs".
/// An unrecognised prefix is not a field — `foo:bar` is searched literally as a
/// name, so a colon in a name never silently matches nothing.
pub fn parse_query(query: &str) -> Vec<Term> {
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
pub fn any_contains<'a>(haystack: impl IntoIterator<Item = &'a str>, needle: &str) -> bool {
    haystack
        .into_iter()
        .any(|h| h.to_lowercase().contains(needle))
}

pub fn module_matches(m: &ModuleSpec, terms: &[Term]) -> bool {
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

pub fn remote_matches(r: &RemoteModule, terms: &[Term]) -> bool {
    terms.iter().all(|t| match t.field {
        Field::Name => any_contains([r.name.as_str(), r.title.as_deref().unwrap_or("")], &t.text),
        Field::Tag => any_contains(r.tags.iter().map(String::as_str), &t.text),
        Field::Description => any_contains([r.description.as_deref().unwrap_or("")], &t.text),
        Field::Capability => any_contains(r.capabilities.iter().map(String::as_str), &t.text),
    })
}
