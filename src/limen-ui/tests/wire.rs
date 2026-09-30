//! What the other languages send.
//!
//! Limen has four SDKs — Rust, Python, JavaScript, Lua — and only the Rust one
//! is checked by the compiler against the host that reads it. These are the
//! exact JSON shapes the other three produce, so a change to a widget here that
//! leaves them behind fails a test rather than a module.

use limen_ui::{View, Widget};

fn view_of(widget: serde_json::Value) -> View {
    serde_json::from_value(serde_json::json!({ "title": "t", "widgets": [widget] }))
        .expect("the host reads it back")
}

/// `limen_sdk.Donut("Findings", [...], centre=...)`, and the same from
/// `Donut()` in limen.js and `M.Donut` in limen.lua.
#[test]
fn a_donut_from_the_other_sdks() {
    let sent = serde_json::json!({
        "kind": "donut",
        "title": "Findings",
        "data": [
            { "label": "critical", "value": 3.0, "kind": "critical" },
            { "label": "low", "value": 8.0, "kind": "low" }
        ],
        "centre": "11 findings"
    });
    let Widget::Donut { title, data, centre } = &view_of(sent).widgets[0] else {
        panic!("not a donut");
    };
    assert_eq!(title, "Findings");
    assert_eq!(centre, "11 findings");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0].label, "critical");
    assert_eq!(data[0].value, 3.0);
    assert_eq!(data[0].kind, "critical");
}

/// A donut with no `kind` and no `centre` — the two-item form every SDK offers
/// — still reads, and means "colour these by position, and put the total in the
/// hole".
#[test]
fn a_donut_that_said_only_what_it_had_to() {
    let sent = serde_json::json!({
        "kind": "donut",
        "title": "",
        "data": [{ "label": "used", "value": 12.5 }]
    });
    let Widget::Donut { data, centre, .. } = &view_of(sent).widgets[0] else {
        panic!("not a donut");
    };
    assert!(centre.is_empty(), "no centre was asked for");
    assert!(data[0].kind.is_empty());
    assert_eq!(data[0].value, 12.5);
}

/// `limen_sdk.Diagram(...)`, `Diagram()` in limen.js, `M.Diagram` in limen.lua:
/// nodes with hover facts, a dashed edge, the fill flag and a double-click that
/// opens a tab.
#[test]
fn a_diagram_from_the_other_sdks() {
    let sent = serde_json::json!({
        "kind": "diagram",
        "title": "Net",
        "nodes": [{
            "id": "gw",
            "label": "gateway",
            "kind": "router",
            "detail": "10.0.0.1",
            "info": [{ "name": "Address", "value": "10.0.0.1" }]
        }],
        "edges": [{ "from": "pc", "to": "gw", "dashed": true }],
        "fill": true,
        "on_activate": {
            "action": { "capability": "network.map", "method": "host" },
            "open_in_tab": true
        }
    });
    let Widget::Diagram { nodes, edges, fill, on_activate, .. } = &view_of(sent).widgets[0] else {
        panic!("not a diagram");
    };
    assert!(*fill, "the map asked for the screen");
    assert_eq!(nodes[0].kind, "router");
    assert_eq!(nodes[0].info[0].name, "Address");
    assert_eq!(nodes[0].info[0].value, "10.0.0.1");
    assert!(edges[0].dashed, "the link was inferred, not observed");
    let act = on_activate.as_ref().expect("a double click asks something");
    assert_eq!(act.action.method, "host");
    assert!(act.open_in_tab);
}

/// The plainest diagram any SDK can send: ids, labels, and the lines. Everything
/// else is optional, and leaving it out must not be an error.
#[test]
fn a_diagram_that_said_only_what_it_had_to() {
    let sent = serde_json::json!({
        "kind": "diagram",
        "nodes": [{ "id": "a", "label": "A" }, { "id": "b", "label": "B" }],
        "edges": [{ "from": "a", "to": "b" }]
    });
    let Widget::Diagram { nodes, edges, fill, on_activate, title } = &view_of(sent).widgets[0]
    else {
        panic!("not a diagram");
    };
    assert!(title.is_empty() && !*fill && on_activate.is_none());
    assert!(nodes[0].info.is_empty() && nodes[0].detail.is_empty());
    assert!(!edges[0].dashed);
}

/// `Chart` — the one the other SDKs had already, unchanged by any of this.
#[test]
fn a_chart_from_the_other_sdks() {
    let sent = serde_json::json!({
        "kind": "chart",
        "title": "Sev",
        "data": [{ "label": "high", "value": 11.0 }]
    });
    let Widget::Chart { title, data } = &view_of(sent).widgets[0] else {
        panic!("not a chart");
    };
    assert_eq!(title, "Sev");
    assert_eq!(data[0].value, 11.0);
}
