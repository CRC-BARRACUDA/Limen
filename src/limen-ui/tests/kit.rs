//! The UI Kit gallery, and that it holds every widget.

use limen_ui::*;
/// The gallery is only a source of truth if it holds every widget. This
/// reads the kinds out of the enum's own source and checks each one is in
/// it — so adding a widget and forgetting the gallery fails here rather
/// than being noticed months later by someone who needed to see it.
#[test]
fn the_ui_kit_shows_every_widget() {
// The enum's own source, which is where the widget kinds are defined.
let src = include_str!("../view.rs");
let body = src
    .split_once("pub enum Widget {")
    .expect("the Widget enum")
    .1
    .split_once("\n}\n")
    .expect("its end")
    .0;

let mut kinds = Vec::new();
for (i, line) in body.lines().enumerate() {
    let t = line.trim();
    // `#[serde(rename = "date")]` renames the variant on the wire.
    if let Some(r) = t.strip_prefix("#[serde(rename = \"") {
        kinds.push(r.split('"').next().unwrap().to_string());
        continue;
    }
    // A variant is a capitalised name at one indent, ending in `{` or
    // `,`; anything else at that depth is a field or a comment.
    if !line.starts_with("    ") || line.starts_with("     ") {
        continue;
    }
    let name = t.trim_end_matches([' ', '{', ',']);
    if name.is_empty()
        || !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.chars().all(|c| c.is_ascii_alphanumeric())
    {
        continue;
    }
    // Renamed variants were taken from the attribute on the line above.
    let renamed = body
        .lines()
        .nth(i.wrapping_sub(1))
        .is_some_and(|p| p.contains("serde(rename"));
    if !renamed {
        kinds.push(name.to_lowercase());
    }
}
kinds.sort();
kinds.dedup();
assert!(kinds.len() >= 12, "only found {kinds:?}");

let view = demo_view();
let shown = view.to_string();
let missing: Vec<&String> = kinds
    .iter()
    .filter(|k| !shown.contains(&format!("\"kind\":\"{k}\"")))
    .collect();
assert!(
    missing.is_empty(),
    "the UI Kit does not show: {missing:?} — every widget must be in it"
);
}
/// And it must be a view the host can actually render, not merely valid
/// JSON: a typo in an action or a style is a gallery that renders wrong.
#[test]
fn the_ui_kit_view_is_one_the_host_understands() {
let view: View = serde_json::from_value(demo_view()).expect("the UI Kit view deserialises");
// Its inputs are the ones the gallery names, all distinct — two widgets
// sharing an id would edit each other.
let ids = widget_ids(&view);
let mut sorted = ids.clone();
sorted.sort();
sorted.dedup();
assert_eq!(sorted.len(), ids.len(), "duplicate ids in the UI Kit: {ids:?}");
assert!(ids.iter().all(|i| i.starts_with("demo.")), "{ids:?}");
}
