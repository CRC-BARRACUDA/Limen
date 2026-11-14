//! The commander: two file panes, as Midnight Commander has them.

use std::collections::HashMap;

use eframe::egui::{self, Key};
use limen_ui::*;
use serde_json::{json, Value};

fn pane(path: &str, up: bool, busy: bool, select: &str) -> Value {
    json!({
        "title": "pane", "path": path, "up": up, "busy": busy, "select": select,
        "entries": [
            {"name": "Documents", "dir": true, "modified": "2026-11-10 09:12"},
            {"name": "Music", "dir": true},
            {"name": "a.txt", "size": "4 KB"},
            {"name": "b.txt", "size": "1 KB"},
        ]
    })
}

fn view(left: Value, right: Value) -> View {
    serde_json::from_value(json!({
        "title": "Files",
        "widgets": [{
            "kind": "commander", "id": "files", "left": left, "right": right,
            "on_action": {"capability": "crowdstrike.panel", "method": "files_op",
                          "args": {"id": "host-1"}}
        }]
    }))
    .unwrap()
}

/// The two templates these tests read, as the host's catalog has them; any
/// other key comes back as itself. The same for every test in this binary.
fn words(key: &str) -> String {
    match key {
        "commander.copy_title" => "Copy {n} item(s) to {to}?".into(),
        "commander.delete_title" => "Delete {n} item(s) from {from}?".into(),
        _ => key.into(),
    }
}

struct Mc {
    ctx: egui::Context,
    time: f64,
    inputs: HashMap<String, String>,
    view: View,
}

impl Mc {
    fn new(view: View) -> Self {
        set_translator(words);
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut mc = Mc { ctx, time: 0.0, inputs: HashMap::new(), view };
        // A few frames: the commander takes the keyboard when it first shows.
        for _ in 0..3 {
            mc.frame(vec![]);
        }
        mc
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<Invoke> {
        self.time += 1.0 / 60.0;
        let mut asked = None;
        let (view, inputs) = (&self.view, &mut self.inputs);
        let _ = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    asked = render_view(ui, view, inputs, None, 0.0);
                });
            },
        );
        asked
    }

    fn key(&mut self, key: Key) -> Option<Invoke> {
        let asked = self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        // A frame after, as a real one follows.
        asked.or_else(|| self.frame(vec![]))
    }

    fn keys(&mut self, keys: &[Key]) {
        for k in keys {
            assert!(self.key(*k).is_none(), "{k:?} asked for something on its own");
        }
    }
}

fn op(i: &Invoke) -> (&str, &str, &str, Vec<&str>) {
    (
        i.args["op"].as_str().unwrap(),
        i.args["pane"].as_str().unwrap(),
        i.args["path"].as_str().unwrap(),
        i.args["names"].as_array().unwrap().iter().map(|n| n.as_str().unwrap()).collect(),
    )
}

#[test]
fn enter_goes_up_on_dotdot_and_opens_a_folder() {
    let mut mc = Mc::new(view(pane("/home/me", true, false, ""), pane("C:\\", true, false, "")));
    // The cursor starts on `..`.
    let up = mc.key(Key::Enter).expect("Enter on .. asks");
    assert_eq!(op(&up), ("up", "left", "/home/me", vec![]));
    assert!(up.confirm.is_none(), "going up is not asked about");
    assert_eq!(up.args["id"], "host-1", "the module's own args go along");

    mc.keys(&[Key::ArrowDown]);
    let open = mc.key(Key::Enter).expect("Enter on a folder asks");
    assert_eq!(op(&open), ("open", "left", "/home/me", vec!["Documents"]));

    // A file has nothing to open it with here.
    mc.keys(&[Key::End]);
    assert!(mc.key(Key::Enter).is_none(), "Enter on a file asks nothing");
}

#[test]
fn tab_switches_pane_and_f5_copies_after_asking_where_to() {
    let mut mc = Mc::new(view(pane("/home/me", true, false, ""), pane("C:\\Users", true, false, "")));
    mc.keys(&[Key::Tab, Key::End]);
    let copy = mc.key(Key::F5).expect("F5 asks");
    assert_eq!(op(&copy), ("copy", "right", "C:\\Users", vec!["b.txt"]));
    let confirm = copy.confirm.expect("a copy is asked about first");
    assert_eq!(confirm.title, "Copy 1 item(s) to /home/me?", "it says where to");
    assert!(confirm.subject.contains("b.txt"), "and what: {}", confirm.subject);

    // Back to the left pane: its own cursor, still on `..`, which is never copied.
    mc.keys(&[Key::Tab]);
    assert!(mc.key(Key::F5).is_none(), "`..` is not copied");
}

#[test]
fn marks_are_what_f8_deletes_and_dotdot_is_never_one() {
    let mut mc = Mc::new(view(pane("/srv", true, false, ""), pane("/", false, false, "")));
    // Insert on `..` marks nothing and moves on; then mark a.txt and b.txt.
    mc.keys(&[Key::Insert, Key::ArrowDown, Key::ArrowDown, Key::Insert, Key::Insert]);
    let del = mc.key(Key::F8).expect("F8 asks");
    assert_eq!(op(&del), ("delete", "left", "/srv", vec!["a.txt", "b.txt"]));
    let confirm = del.confirm.expect("a delete is asked about first");
    assert!(confirm.subject.contains("a.txt") && confirm.subject.contains("b.txt"));
    assert_eq!(confirm.title, "Delete 2 item(s) from /srv?", "it says where from");

    // Insert again takes a mark off; Delete is F8's other key.
    mc.keys(&[Key::Home, Key::ArrowDown, Key::ArrowDown, Key::ArrowDown, Key::Insert]);
    let del = mc.key(Key::Delete).expect("Delete asks");
    assert_eq!(op(&del).3, vec!["b.txt"]);
}

#[test]
fn a_new_path_puts_the_cursor_on_the_folder_come_out_of() {
    let mut mc = Mc::new(view(pane("/home/me/Music", true, false, ""), pane("/", false, false, "")));
    mc.keys(&[Key::End]);
    // The module answers `up` with the folder above, naming the one come out of.
    mc.view = view(pane("/home/me", true, false, "Music"), pane("/", false, false, ""));
    mc.frame(vec![]);
    let open = mc.key(Key::Enter).expect("the cursor is on a folder");
    assert_eq!(op(&open), ("open", "left", "/home/me", vec!["Music"]));

    // Without a name, a new path starts at the top: on `..`.
    mc.view = view(pane("/home", true, false, ""), pane("/", false, false, ""));
    mc.frame(vec![]);
    assert_eq!(op(&mc.key(Key::Enter).unwrap()).0, "up");
}

#[test]
fn a_busy_pane_asks_nothing_and_backspace_needs_an_up() {
    let mut mc = Mc::new(view(pane("/a", true, true, ""), pane("/", false, false, "")));
    for k in [Key::F5, Key::F8, Key::Enter, Key::Backspace] {
        assert!(mc.key(k).is_none(), "{k:?} on a busy pane asks nothing");
    }
    mc.keys(&[Key::Tab]);
    assert!(mc.key(Key::Backspace).is_none(), "no `..`, so Backspace goes nowhere");
    mc.view = view(pane("/a", true, false, ""), pane("/", false, false, ""));
    mc.keys(&[Key::Tab]);
    assert_eq!(op(&mc.key(Key::Backspace).unwrap()), ("up", "left", "/a", vec![]));
}
