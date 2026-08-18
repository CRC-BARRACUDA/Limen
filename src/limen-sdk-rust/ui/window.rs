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

/// [`window_auto`], with what comes back opening in a tab of its own.
///
/// For the end of a chain rather than the middle of one: a long job's last step
/// is often a *result* — a scan report, a generated document — and this leaves it
/// beside the screen that produced it instead of replacing that screen with it.
/// The tab that ran the job goes back to being ready for the next one.
///
/// Once, not on every render: the view carrying this opens a tab each time the
/// host accepts it, so a module that returns it from `ui` opens another one every
/// time the user comes back to that tab.
pub fn window_auto_in_tab(
    title: impl Into<String>,
    widgets: Vec<Widget>,
    capability: impl Into<String>,
    method: impl Into<String>,
    args: Value,
) -> Value {
    let mut v = window_auto(title, widgets, capability, method, args);
    if let Some(auto) = v.get_mut("auto").and_then(Value::as_object_mut) {
        auto.insert("open_in_tab".into(), json!(true));
    }
    v
}

/// Attach [`window_auto_in_tab`]'s auto-action to a view that is already built —
/// a screen assembled elsewhere, which now has a result to hand over.
pub fn auto_in_tab(
    view: Value,
    capability: impl Into<String>,
    method: impl Into<String>,
    args: Value,
) -> Value {
    let mut v = view;
    if let Value::Object(m) = &mut v {
        m.insert(
            "auto".into(),
            json!({
                "capability": capability.into(),
                "method": method.into(),
                "args": args,
                "open_in_tab": true,
            }),
        );
    }
    v
}

/// Attach a passing notice to a view: what just happened, said in the corner
/// when this view arrives.
///
/// `level` is "info", "ok", "warning" or "error". It is raised once, when the
/// view is accepted — a redraw of the same screen does not raise it again.
///
/// For an event, not for a state: "installed" belongs here, "not installed"
/// belongs on the screen, where it stays until it stops being true.
pub fn notice(view: Value, level: &str, text: impl Into<String>) -> Value {
    let mut v = view;
    if let Value::Object(m) = &mut v {
        m.insert(
            "notice".into(),
            json!({ "level": level, "text": text.into() }),
        );
    }
    v
}
