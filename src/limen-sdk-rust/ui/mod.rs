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
/// One node of a [`diagram`]: a circle, its label, and what to say about it
/// when the pointer is on it.
///
/// `kind` decides the circle's colour: `router`, `server`, `switch`, `self`, or
/// anything else for the neutral one.
#[derive(Clone, Debug)]
pub struct DiagramNode {
    id: String,
    label: String,
    kind: String,
    detail: String,
    info: Vec<(String, String)>,
}

/// A node, named by an `id` that edges refer to and drawn with `label` under it.
pub fn node(id: impl Into<String>, label: impl Into<String>) -> DiagramNode {
    DiagramNode {
        id: id.into(),
        label: label.into(),
        kind: String::new(),
        detail: String::new(),
        info: Vec::new(),
    }
}

impl DiagramNode {
    /// What it is, which is what colour it is drawn in.
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    /// A second, quieter line under the label — an address, a version, a count.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// One more named value for the hover card, which the app draws itself.
    ///
    /// This is where everything that does not fit beside a circle goes: the
    /// hardware address, what answered, when it was last seen. Added in the
    /// order it should be read.
    pub fn info(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.info.push((name.into(), value.into()));
        self
    }

    fn to_value(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
            "info": self.info.iter()
                .map(|(n, v)| json!({ "name": n, "value": v }))
                .collect::<Vec<Value>>(),
        })
    }
}

/// Circles and the lines between them.
///
/// The companion of [`chart`]: a chart says how much of each, a diagram says
/// what is connected to what — hosts around a gateway, a module's dependencies,
/// a process tree. The host lays it out, draws it, and answers the pointer, so a
/// module hands over what it knows and nothing about where it goes on the page.
///
/// Nodes are built with [`node`]. An edge is `(from, to, dashed)`, naming node
/// ids; `dashed` is for a link that was inferred rather than observed.
///
/// Add [`Widget::fills`] when the diagram *is* the screen: it then takes all the
/// height left over and whatever follows it goes to the bottom of the window.
pub fn diagram(
    title: impl Into<String>,
    nodes: Vec<DiagramNode>,
    edges: Vec<(String, String, bool)>,
) -> Widget {
    let nodes: Vec<Value> = nodes.iter().map(DiagramNode::to_value).collect();
    let edges: Vec<Value> = edges
        .into_iter()
        .map(|(from, to, dashed)| json!({ "from": from, "to": to, "dashed": dashed }))
        .collect();
    Widget::of_kind("diagram")
        .set("title", json!(title.into()))
        .set("nodes", Value::Array(nodes))
        .set("edges", Value::Array(edges))
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
