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
/// Swapping a module's screen must count as a change; refreshing one in
/// place must not, or a progress checklist would restart its entrance on
/// every tick.
#[test]
fn view_shape_tracks_the_screen_not_its_contents() {
let parse = |s: &str| -> View { serde_json::from_str(s).unwrap() };

let basic = parse(
    r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
);
// Same screen, different text — a progress step advancing.
let ticked = parse(
    r#"{"title":"S","widgets":[{"kind":"label","text":"b"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
);
assert_eq!(
    view_shape(&basic),
    view_shape(&ticked),
    "text changes must not restart the entrance"
);

// A control appears — Basic becoming Advanced.
let advanced = parse(
    r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"checkbox","id":"x"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
);
assert_ne!(view_shape(&basic), view_shape(&advanced));

// Two screens built from the same widgets, differing only in what the
// button says — a mode switch offering the next scan. Same shape by the
// old rule, and so no entrance at all.
let to_procs = parse(
    r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"Scan running processes instead","action":{"capability":"c","method":"m"}}]}"#,
);
let to_autoruns = parse(
    r#"{"title":"S","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"Scan what starts automatically instead","action":{"capability":"c","method":"m"}}]}"#,
);
assert_ne!(
    view_shape(&to_procs),
    view_shape(&to_autoruns),
    "a screen whose only difference is its button is still a different screen"
);

// ...and a button nested in a row counts the same way.
let row_a = parse(
    r#"{"title":"S","widgets":[{"kind":"row","children":[{"kind":"button","text":"Files","action":{"capability":"c","method":"m"}}]}]}"#,
);
let row_b = parse(
    r#"{"title":"S","widgets":[{"kind":"row","children":[{"kind":"button","text":"Processes","action":{"capability":"c","method":"m"}}]}]}"#,
);
assert_ne!(view_shape(&row_a), view_shape(&row_b));

// A progress view keeps its buttons while its counts change, so it must
// still not re-animate — that is what the whole rule protects.
let polling = |n: &str| {
    parse(&format!(
        r#"{{"title":"S","widgets":[{{"kind":"label","text":"{n} lines logged"}},{{"kind":"button","text":"Stop","action":{{"capability":"c","method":"m"}}}}]}}"#
    ))
};
assert_eq!(view_shape(&polling("12")), view_shape(&polling("4310")));

// A different screen entirely.
let other = parse(
    r#"{"title":"Other","widgets":[{"kind":"label","text":"a"},{"kind":"button","text":"go","action":{"capability":"c","method":"m"}}]}"#,
);
assert_ne!(view_shape(&basic), view_shape(&other));

// Same kinds, different order still counts — the arrangement changed.
let reordered = parse(
    r#"{"title":"S","widgets":[{"kind":"button","text":"go","action":{"capability":"c","method":"m"}},{"kind":"label","text":"a"}]}"#,
);
assert_ne!(view_shape(&basic), view_shape(&reordered));
}
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
/// The header is chrome because it does not change. When it does — a
/// different version after an update — that is still a different screen.
#[test]
fn a_change_inside_chrome_is_still_a_change() {
let parse = |v: &str| -> View {
    serde_json::from_str(&format!(
        r#"{{"title":"S","widgets":[{{"kind":"chrome","children":[
            {{"kind":"button","text":"{v}","action":{{"capability":"c","method":"m"}}}}
        ]}}]}}"#
    ))
    .unwrap()
};
assert_ne!(
    view_shape(&parse("Reinstall 2.16.3")),
    view_shape(&parse("Reinstall 2.17.0"))
);
assert_eq!(
    view_shape(&parse("Reinstall 2.16.3")),
    view_shape(&parse("Reinstall 2.16.3"))
);
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
