//! Building a view the way a module builds one.

use serde_json::json;

use limen_sdk_rust::ui::*;

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
