//! A widget under construction, and the menu items a table row can offer.

use serde_json::{json, Map, Value};


/// A single widget being built. Fluent setters return `Self`.
#[derive(Debug, Clone)]
pub struct Widget(Map<String, Value>);

impl Widget {
    pub(crate) fn of_kind(kind: &str) -> Self {
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        Widget(m)
    }

    pub(crate) fn set(mut self, key: &str, val: Value) -> Self {
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
    /// For a [`file`] widget: take a directory rather than a file.
    pub fn directory(self) -> Self {
        self.set("accepts", json!("dir"))
    }

    /// For a [`file`] widget: take *either* a file or a directory.
    ///
    /// The OS has no dialog that picks either, so such a field shows two Browse
    /// buttons; give the second one a label with [`Widget::browse_dir`].
    pub fn files_or_dirs(self) -> Self {
        self.set("accepts", json!("file|dir"))
    }

    /// For a [`file`] widget: the Browse button's label, so the module can
    /// localize it alongside the rest of its view.
    pub fn browse(self, text: impl Into<String>) -> Self {
        self.set("browse", json!(text.into()))
    }

    /// For a [`files_or_dirs`](Widget::files_or_dirs) widget: the label of the
    /// second button, the one that opens a folder chooser.
    pub fn browse_dir(self, text: impl Into<String>) -> Self {
        self.set("browse_dir", json!(text.into()))
    }

    pub fn label(self, text: impl Into<String>) -> Self {
        self.set("label", json!(text.into()))
    }
    pub fn placeholder(self, text: impl Into<String>) -> Self {
        self.set("placeholder", json!(text.into()))
    }
    /// Carry a time of day as well as a day. Off by default: most questions
    /// are about a day, and the ones that are not usually say so.
    pub fn with_time(self) -> Self {
        self.set("time", json!(true))
    }

    /// Mask what is typed, for a secret. Single-line only.
    pub fn password(self) -> Self {
        self.set("password", json!(true))
    }

    /// Draw the button, but do not let it be pressed.
    ///
    /// For a control that is real but not available yet — a Run with nothing
    /// chosen to run on. Hiding it instead would make the screen change shape
    /// as the form is filled in.
    pub fn enabled(self, on: bool) -> Self {
        self.set("enabled", json!(on))
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
    /// Draw a button in red: the action destroys something.
    pub fn danger(self) -> Self {
        self.set("style", json!("danger"))
    }
    /// Start a `checkbox` checked.
    pub fn checked(self) -> Self {
        self.set("default", json!(true))
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
    /// Close the pop-up this button is in, without calling the module. For the
    /// Cancel of a [`window_modal`], which has nothing to cancel remotely.
    pub fn dismiss(self) -> Self {
        self.set("dismiss", json!(true))
    }
    /// Ask before running this button's action.
    ///
    /// The host shows the question — the same dialog it uses to confirm removing
    /// a module — and only calls the module if the answer is yes. `subject` is
    /// what the action happens *to*, drawn in its own frame so the thing at
    /// stake is unmistakable. Labels default to the host's own wording.
    pub fn confirm(self, title: &str, subject: &str) -> Self {
        self.set("confirm", json!({ "title": title, "subject": subject }))
    }

    /// As [`Widget::confirm`], with the module's own words on the buttons.
    pub fn confirm_labelled(self, title: &str, subject: &str, yes: &str, no: &str) -> Self {
        self.set(
            "confirm",
            json!({ "title": title, "subject": subject,
                    "confirm_label": yes, "cancel_label": no }),
        )
    }

    /// Draw a painted icon instead of the label, which becomes the tooltip.
    ///
    /// Known: `"trash"`. Painted rather than typed because the host ships one
    /// font and it has no trash character.
    pub fn icon(self, name: &str) -> Self {
        self.set("icon", json!(name))
    }

    // ---- table interactivity ---------------------------------------------- //
    /// Per-row identity (parallel to the table's `rows`); the row's id is sent
    /// as the `id` param when a row is activated or a menu item is chosen.
    pub fn row_ids(self, ids: Vec<String>) -> Self {
        self.set("row_ids", json!(ids))
    }
    /// Attach a right-click context menu shared by every row (see [`menu_item`]).
    pub fn row_menu(self, items: Vec<MenuItem>) -> Self {
        let arr: Vec<Value> = items.into_iter().map(MenuItem::into_value).collect();
        self.set("menu", Value::Array(arr))
    }
    /// Per-row menus (parallel to `rows`); a non-empty entry overrides the
    /// shared `row_menu` for that row — so rows can offer different actions (or
    /// an empty menu for none).
    pub fn row_menus(self, per_row: Vec<Vec<MenuItem>>) -> Self {
        let arr: Vec<Value> = per_row
            .into_iter()
            .map(|items| Value::Array(items.into_iter().map(MenuItem::into_value).collect()))
            .collect();
        self.set("row_menus", Value::Array(arr))
    }
    /// Invoke `capability`.`method` when a row is double-clicked, opening the
    /// returned view in a new tab.
    /// Call `capability`/`method` as soon as a [`select`] changes, so the module
    /// can answer with a different screen rather than only recording the answer.
    ///
    /// The chosen value arrives in the params under the select's own id, along
    /// with anything passed to [`Widget::args`].
    pub fn on_change(self, capability: impl Into<String>, method: impl Into<String>) -> Self {
        let args = self.0.get("args").cloned().unwrap_or_else(|| json!({}));
        self.set(
            "on_change",
            json!({
                "capability": capability.into(),
                "method": method.into(),
                "args": args,
            }),
        )
    }

    pub fn on_activate(self, capability: impl Into<String>, method: impl Into<String>) -> Self {
        self.set(
            "on_activate",
            json!({
                "action": { "capability": capability.into(), "method": method.into() },
                "open_in_tab": true,
            }),
        )
    }

    /// As [`Widget::on_activate`], but the result replaces the current view
    /// rather than opening a tab — for a table inside a pop-up, where a new tab
    /// would take the user out of the window they are working in.
    pub fn on_activate_here(
        self,
        capability: impl Into<String>,
        method: impl Into<String>,
    ) -> Self {
        self.set(
            "on_activate",
            json!({
                "action": { "capability": capability.into(), "method": method.into() },
                "open_in_tab": false,
            }),
        )
    }

    /// The JSON this widget becomes — what a module actually sends. Public
    /// because a caller assembling a view by hand, or a test inspecting one,
    /// has no other way to see it.
    pub fn into_value(self) -> Value {
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
    /// Ask before running this, exactly as a button can — a destructive action
    /// is no less destructive for being in a menu. The host shows the question
    /// and only calls the module if the answer is yes.
    pub fn confirm(mut self, title: &str, subject: &str) -> Self {
        self.0.insert(
            "confirm".into(),
            json!({ "title": title, "subject": subject }),
        );
        self
    }

    /// As [`MenuItem::confirm`], with the module's own words on the buttons.
    pub fn confirm_labelled(mut self, title: &str, subject: &str, yes: &str, no: &str) -> Self {
        self.0.insert(
            "confirm".into(),
            json!({ "title": title, "subject": subject,
                    "confirm_label": yes, "cancel_label": no }),
        );
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

/// A submenu entry containing `children` (e.g. a Windows "Open path" submenu).
/// The host draws the arrow that says it opens one; the label should not.
pub fn submenu(label: impl Into<String>, children: Vec<MenuItem>) -> MenuItem {
    MenuItem::new(label).submenu(children)
}
