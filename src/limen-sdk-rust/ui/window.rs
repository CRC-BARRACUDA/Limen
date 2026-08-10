//! What a module answers with: a screen, a pop-up, or a screen that asks to
//! be called again.

use serde_json::{json, Value};

use super::*;

/// Build the final view value (a titled list of widgets) — return this from your
/// module's `ui` method.
pub fn window(title: impl Into<String>, widgets: Vec<Widget>) -> Value {
    let ws: Vec<Value> = widgets.into_iter().map(Widget::into_value).collect();
    json!({ "title": title.into(), "widgets": ws })
}

/// A view shown as a pop-up over the screen it came from, instead of replacing
/// it.
///
/// The view underneath stays visible and stops responding, so settings or a
/// sub-form can be put in front of the user without losing their place. Pop-ups
/// stack — one can open another — and Esc always closes the top one, so give
/// every pop-up a way out that the user would think to press.
/// `id` is the pop-up's identity: returning a view with an id that is already
/// open redraws that pop-up in place instead of opening another over it, so a
/// form can refresh itself after every edit.
/// A pop-up that asks for a particular size.
///
/// `width` is in points and the host clamps it to the window — a module cannot
/// know how much room there is. Changing it between steps is the point: the
/// pop-up animates from one size to the next, so a step that needs more room
/// grows into it rather than reappearing at a different size.
pub fn window_modal_sized(
    title: impl Into<String>,
    id: impl Into<String>,
    width: f32,
    widgets: Vec<Widget>,
) -> Value {
    let mut v = window_modal(title, id, widgets);
    if let Value::Object(m) = &mut v {
        m.insert("modal_width".into(), json!(width));
    }
    v
}

pub fn window_modal(
    title: impl Into<String>,
    id: impl Into<String>,
    widgets: Vec<Widget>,
) -> Value {
    let mut v = window(title, widgets);
    if let Value::Object(m) = &mut v {
        m.insert("modal".into(), json!(id.into()));
    }
    v
}

/// A view that auto-invokes `capability`.`method` once, right after it renders
/// (no user click). Use it to chain a multi-step flow — each step returns a view
/// with the next step's `auto`, and the last step returns a plain [`window`] to
/// stop. `args` are merged into the call.
pub fn window_auto(
    title: impl Into<String>,
    widgets: Vec<Widget>,
    capability: impl Into<String>,
    method: impl Into<String>,
    args: Value,
) -> Value {
    let mut v = window(title, widgets);
    if let Value::Object(m) = &mut v {
        m.insert(
            "auto".into(),
            json!({ "capability": capability.into(), "method": method.into(), "args": args }),
        );
    }
    v
}
