//! The wire format a module answers with, and the renderer.

use std::collections::HashMap;
use limen_ui::*;
/// `modal` and `dismiss` are opt-in: every view built before they existed
/// must keep replacing the screen and calling the module, as it always did.
#[test]
fn views_are_not_pop_ups_unless_they_say_so() {
let plain: View = serde_json::from_str(r#"{"title":"S","widgets":[]}"#).unwrap();
assert!(plain.modal.is_none());

let popup: View =
    serde_json::from_str(r#"{"title":"S","widgets":[],"modal":"settings"}"#).unwrap();
assert_eq!(popup.modal.as_deref(), Some("settings"));
// Size is optional: a pop-up that does not ask gets the host's default.
assert!(popup.modal_width.is_none());
let sized: View = serde_json::from_str(
    r#"{"title":"S","widgets":[],"modal":"s","modal_width":860.0}"#,
)
.unwrap();
assert_eq!(sized.modal_width, Some(860.0));

let btn: Widget = serde_json::from_str(
    r#"{"kind":"button","text":"Cancel","action":{"capability":"c","method":"m"},"dismiss":true}"#,
)
.unwrap();
match btn {
    Widget::Button { dismiss, .. } => assert!(dismiss),
    _ => panic!("expected a button"),
}
let old: Widget = serde_json::from_str(
    r#"{"kind":"button","text":"Go","action":{"capability":"c","method":"m"}}"#,
)
.unwrap();
match old {
    Widget::Button { dismiss, .. } => assert!(!dismiss, "a plain button still calls out"),
    _ => panic!("expected a button"),
}
}
/// A module's view parses, and its fields are collected into the params its
/// buttons carry.
#[test]
fn parses_a_module_view_and_collects_params() {
// The exact shape modules emit from their `ui` method.
let json = serde_json::json!({
    "title": "PowerShell",
    "widgets": [
        {"kind": "label", "text": "hi", "style": "weak"},
        {"kind": "text", "id": "command", "label": "Command", "multiline": true},
        {"kind": "button", "text": "Run", "style": "primary",
         "action": {"capability": "ops.powershell", "method": "run"}}
    ]
});
let view: View = serde_json::from_value(json).unwrap();
assert_eq!(view.title, "PowerShell");
assert_eq!(view.widgets.len(), 3);

// The renderer would populate `inputs`; collecting turns them into params.
let mut inputs = HashMap::new();
inputs.insert("command".to_string(), "Get-Process".to_string());
let params = collect_params(&view, &inputs);
assert_eq!(params["command"], "Get-Process");
}
/// Chrome is skipped by the entrance, not by everything else. A wrapper the
/// walkers cannot see through would take its inputs out of the params — the
/// picker inside it would submit nothing, and choosing a tool would quietly
/// stop working.
#[test]
fn chrome_is_invisible_to_the_animation_and_to_nothing_else() {
let view: View = serde_json::from_str(
    r#"{"title":"S","widgets":[
        {"kind":"chrome","children":[
            {"kind":"label","text":"Chainsaw v2.16.3"},
            {"kind":"select","id":"task_pick","options":["Hunt","Gaps"],"default":"Hunt"}
        ]},
        {"kind":"text","id":"target"}
    ]}"#,
)
.unwrap();

// Its inputs are declared...
let ids = widget_ids(&view);
assert!(ids.contains(&"task_pick".to_string()), "{ids:?}");
assert!(ids.contains(&"target".to_string()), "{ids:?}");

// ...and submitted.
let mut inputs = HashMap::new();
inputs.insert("task_pick".to_string(), "Gaps".to_string());
inputs.insert("target".to_string(), "/evidence".to_string());
let params = collect_params(&view, &inputs);
assert_eq!(params["task_pick"], "Gaps");
assert_eq!(params["target"], "/evidence");
}
/// A date field's value has to reach the module like any other input.
#[test]
fn a_date_field_submits_its_value() {
let view: View = serde_json::from_str(
    r#"{"title":"S","widgets":[{"kind":"date","id":"from","time":true}]}"#,
)
.unwrap();
assert!(widget_ids(&view).contains(&"from".to_string()));
let mut inputs = HashMap::new();
inputs.insert("from".to_string(), "2024-01-31T00:00:00".to_string());
assert_eq!(collect_params(&view, &inputs)["from"], "2024-01-31T00:00:00");
}
