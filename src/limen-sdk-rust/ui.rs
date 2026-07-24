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

use serde_json::{json, Map, Value};

/// A single widget being built. Fluent setters return `Self`.
#[derive(Debug, Clone)]
pub struct Widget(Map<String, Value>);

impl Widget {
    fn of_kind(kind: &str) -> Self {
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        Widget(m)
    }

    fn set(mut self, key: &str, val: Value) -> Self {
        self.0.insert(key.into(), val);
        self
    }

    // ---- label styling ---------------------------------------------------- //
    /// Set an explicit style ("normal" | "heading" | "strong" | "weak" | "mono").
    pub fn style(self, style: &str) -> Self {
        self.set("style", json!(style))
    }
    pub fn weak(self) -> Self {
        self.style("weak")
    }
    pub fn strong(self) -> Self {
        self.style("strong")
    }
    pub fn heading(self) -> Self {
        self.style("heading")
    }
    pub fn mono(self) -> Self {
        self.style("mono")
    }

    // ---- input options ---------------------------------------------------- //
    /// A field label (for `text` / `select`).
    pub fn label(self, text: impl Into<String>) -> Self {
        self.set("label", json!(text.into()))
    }
    pub fn placeholder(self, text: impl Into<String>) -> Self {
        self.set("placeholder", json!(text.into()))
    }
    pub fn multiline(self) -> Self {
        self.set("multiline", json!(true))
    }
    /// Default value for a `text` / `select`.
    pub fn default(self, value: impl Into<String>) -> Self {
        self.set("default", json!(value.into()))
    }

    // ---- button ----------------------------------------------------------- //
    /// Make a button the primary (accent) style.
    pub fn primary(self) -> Self {
        self.set("style", json!("primary"))
    }

    fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

/// A text label.
pub fn label(text: impl Into<String>) -> Widget {
    Widget::of_kind("label").set("text", json!(text.into())).style("normal")
}

/// A text input; its `id` keys the value passed back in params.
pub fn text(id: impl Into<String>) -> Widget {
    Widget::of_kind("text").set("id", json!(id.into()))
}

/// A dropdown; its `id` keys the value passed back in params.
pub fn select(id: impl Into<String>, options: Vec<String>) -> Widget {
    Widget::of_kind("select")
        .set("id", json!(id.into()))
        .set("options", json!(options))
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

/// A horizontal group of widgets.
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

/// Build the final view value (a titled list of widgets) — return this from your
/// module's `ui` method.
pub fn window(title: impl Into<String>, widgets: Vec<Widget>) -> Value {
    let ws: Vec<Value> = widgets.into_iter().map(Widget::into_value).collect();
    json!({ "title": title.into(), "widgets": ws })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_view_spec() {
        let v = window(
            "Hello",
            vec![
                label("hi").weak(),
                text("name").label("Name").placeholder("world"),
                separator(),
                button("Go", "demo.hello", "greet").primary(),
                table(vec!["A".into(), "B".into()], vec![vec!["1".into(), "2".into()]]),
            ],
        );
        assert_eq!(v["title"], "Hello");
        let ws = v["widgets"].as_array().unwrap();
        assert_eq!(ws[0]["kind"], "label");
        assert_eq!(ws[0]["style"], "weak");
        assert_eq!(ws[1]["kind"], "text");
        assert_eq!(ws[1]["label"], "Name");
        assert_eq!(ws[3]["style"], "primary");
        assert_eq!(ws[3]["action"]["method"], "greet");
        assert_eq!(ws[4]["kind"], "table");
        assert_eq!(ws[4]["columns"][1], "B");
    }
}
