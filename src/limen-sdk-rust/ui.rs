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
    /// Extra params merged into this button's call (e.g. a device id/path).
    pub fn args(self, args: Value) -> Self {
        self.set("args", args)
    }
    /// Open this button's result view in a new tab instead of replacing the
    /// current one.
    pub fn open_in_tab(self) -> Self {
        self.set("open_in_tab", json!(true))
    }

    // ---- table interactivity ---------------------------------------------- //
    /// Per-row identity (parallel to the table's `rows`); the row's id is sent
    /// as the `id` param when a row is activated or a menu item is chosen.
    pub fn row_ids(self, ids: Vec<String>) -> Self {
        self.set("row_ids", json!(ids))
    }
    /// Attach a right-click context menu to each row (see [`menu_item`]).
    pub fn row_menu(self, items: Vec<MenuItem>) -> Self {
        let arr: Vec<Value> = items.into_iter().map(MenuItem::into_value).collect();
        self.set("menu", Value::Array(arr))
    }
    /// Invoke `capability`.`method` when a row is double-clicked, opening the
    /// returned view in a new tab.
    pub fn on_activate(self, capability: impl Into<String>, method: impl Into<String>) -> Self {
        self.set(
            "on_activate",
            json!({
                "action": { "capability": capability.into(), "method": method.into() },
                "open_in_tab": true,
            }),
        )
    }

    fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

/// One right-click menu entry on a table row. Build a leaf with [`menu_item`]
/// (invokes a method) or a submenu with [`submenu`].
#[derive(Debug, Clone)]
pub struct MenuItem(Map<String, Value>);

impl MenuItem {
    fn new(label: impl Into<String>) -> Self {
        let mut m = Map::new();
        m.insert("label".into(), json!(label.into()));
        MenuItem(m)
    }
    /// Extra params merged into the call (e.g. `json!({"via":"explorer"})`).
    pub fn args(mut self, args: Value) -> Self {
        self.0.insert("args".into(), args);
        self
    }
    /// Open the result view in a new tab instead of replacing the current one.
    pub fn open_in_tab(mut self) -> Self {
        self.0.insert("open_in_tab".into(), json!(true));
        self
    }
    /// Turn this into a submenu with the given children (its own action, if any,
    /// is then ignored).
    pub fn submenu(mut self, children: Vec<MenuItem>) -> Self {
        let arr: Vec<Value> = children.into_iter().map(MenuItem::into_value).collect();
        self.0.insert("children".into(), Value::Array(arr));
        self
    }
    fn into_value(self) -> Value {
        Value::Object(self.0)
    }
}

/// A leaf menu item that invokes `capability`.`method` (with the row's `id`).
pub fn menu_item(
    label: impl Into<String>,
    capability: impl Into<String>,
    method: impl Into<String>,
) -> MenuItem {
    let mut m = MenuItem::new(label);
    m.0.insert(
        "action".into(),
        json!({ "capability": capability.into(), "method": method.into() }),
    );
    m
}

/// A submenu entry containing `children` (e.g. Windows "Open path ▸").
pub fn submenu(label: impl Into<String>, children: Vec<MenuItem>) -> MenuItem {
    MenuItem::new(label).submenu(children)
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

    #[test]
    fn builds_an_interactive_table() {
        let t = table(vec!["Name".into()], vec![vec!["hub".into()]])
            .row_ids(vec!["usb:0bda".into()])
            .on_activate("devices.local", "about")
            .row_menu(vec![
                menu_item("About", "devices.local", "about").open_in_tab(),
                submenu(
                    "Open path",
                    vec![menu_item("File Explorer", "devices.local", "open_path")
                        .args(json!({ "via": "explorer" }))],
                ),
            ])
            .into_value();

        assert_eq!(t["row_ids"][0], "usb:0bda");
        assert_eq!(t["on_activate"]["action"]["method"], "about");
        assert_eq!(t["on_activate"]["open_in_tab"], true);
        assert_eq!(t["menu"][0]["label"], "About");
        assert_eq!(t["menu"][0]["action"]["method"], "about");
        assert_eq!(t["menu"][0]["open_in_tab"], true);
        // Submenu: no action, has children carrying per-item args.
        assert_eq!(t["menu"][1]["label"], "Open path");
        assert_eq!(t["menu"][1]["children"][0]["args"]["via"], "explorer");
        assert_eq!(t["menu"][1]["children"][0]["action"]["method"], "open_path");
    }
}
