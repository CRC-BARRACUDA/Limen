//! UI builder for native Rust modules — mirrors the scripted SDKs' builders so
//! you don't hand-write the JSON view spec.
//!
//! ```ignore
//! use limen_sdk_rust::ui::*;
//!
//! fn view() -> serde_json::Value {
//!     window("Hello", vec![
//!         label("Pick a name and greet.").weak(),
//!         text("name").label("Name").placeholder("world"),
//!         button("Greet", "demo.hello", "greet").primary(),
//!     ])
//! }
//! ```
//!
//! Every constructor returns a [`Widget`]; fluent methods tweak it. [`window`]
//! collects widgets into the final view value the GUI core renders.

use serde_json::{json, Value};


mod widget;
mod window;

pub use widget::*;
pub use window::*;

/// A text label.
pub fn label(text: impl Into<String>) -> Widget {
    Widget::of_kind("label").set("text", json!(text.into())).style("normal")
}

/// A text input; its `id` keys the value passed back in params.
pub fn text(id: impl Into<String>) -> Widget {
    Widget::of_kind("text").set("id", json!(id.into()))
}

/// A filesystem path input; its `id` keys the chosen path in params.
///
/// The user can type a path, drag a file onto the field, or press Browse for the
/// OS picker. Chain [`Widget::directory`] to pick a folder instead, and
/// [`Widget::browse`] to localize the button.
pub fn file(id: impl Into<String>) -> Widget {
    Widget::of_kind("file").set("id", json!(id.into()))
}

/// A dropdown; its `id` keys the value passed back in params.
/// A date, typed or picked off a calendar.
///
/// Whichever way it is given, the value that reaches the module is canonical —
/// `2024-01-31` and `2024-01-31T00:00:00` are the two forms, chosen by
/// [`Widget::with_time`], and anything else the user typed is rejected at the
/// field rather than by whatever the module hands it to.
pub fn date(id: impl Into<String>) -> Widget {
    Widget::of_kind("date").set("id", json!(id.into()))
}

pub fn select(id: impl Into<String>, options: Vec<String>) -> Widget {
    Widget::of_kind("select")
        .set("id", json!(id.into()))
        .set("options", json!(options))
}

/// An on/off checkbox; its `id` keys a boolean returned in params. Unchecked
/// unless `.checked()` is called.
pub fn checkbox(id: impl Into<String>, label: impl Into<String>) -> Widget {
    Widget::of_kind("checkbox")
        .set("id", json!(id.into()))
        .set("label", json!(label.into()))
}

/// A button that invokes `capability`.`method` when clicked.
pub fn button(text: impl Into<String>, capability: impl Into<String>, method: impl Into<String>) -> Widget {
    Widget::of_kind("button")
        .set("text", json!(text.into()))
        .set("action", json!({ "capability": capability.into(), "method": method.into() }))
}

/// A horizontal divider.
pub fn separator() -> Widget {
    Widget::of_kind("separator")
}

/// Inside a [`row`], pushes everything after it to the right edge.
///
/// What makes a column of trailing buttons line up when the text before them
/// does not — a delete button per row, say, which should sit at the same place
/// on every line rather than wherever that line's text happened to end.
pub fn spacer() -> Widget {
    Widget::of_kind("spacer")
}

/// A progress step with an animated status icon — a loading spinner that morphs
/// into a check when done. `state` is "pending", "loading", or "done".
pub fn step(label: impl Into<String>, state: impl Into<String>) -> Widget {
    Widget::of_kind("step")
        .set("label", json!(label.into()))
        .set("state", json!(state.into()))
}

/// A horizontal group of widgets.
/// Widgets that belong to the module rather than to the screen — a title, a
/// version, the picker that chooses which screen you are on.
///
/// They take no part in the entrance animation. A module that swaps one screen
/// for another gets that entrance so the change reads as a change; anything
/// wrapped here is the part that did not change, and replaying it makes the
/// header flinch every time.
pub fn chrome(children: Vec<Widget>) -> Widget {
    let kids: Vec<Value> = children.into_iter().map(Widget::into_value).collect();
    Widget::of_kind("chrome").set("children", Value::Array(kids))
}

pub fn row(children: Vec<Widget>) -> Widget {
    let kids: Vec<Value> = children.into_iter().map(Widget::into_value).collect();
    Widget::of_kind("row").set("children", Value::Array(kids))
}

/// A table with a header row and string cells.
pub fn table(columns: Vec<String>, rows: Vec<Vec<String>>) -> Widget {
    Widget::of_kind("table")
        .set("columns", json!(columns))
        .set("rows", json!(rows))
}

/// A horizontal bar chart: `(label, value)` bars under an optional title.
pub fn chart(title: impl Into<String>, data: Vec<(String, f64)>) -> Widget {
    let bars: Vec<Value> = data
        .into_iter()
        .map(|(label, value)| json!({ "label": label, "value": value }))
        .collect();
    Widget::of_kind("chart")
        .set("title", json!(title.into()))
        .set("data", Value::Array(bars))
}
