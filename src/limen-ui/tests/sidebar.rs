//! The sidebar: sections down the left, the open one beside it.

use std::collections::HashMap;

use eframe::egui;
use limen_ui::*;
use serde_json::json;

/// A screen shaped exactly as the host panel module sends it.
fn panel(selected: &str, body_text: &str) -> View {
    serde_json::from_value(json!({
        "title": "WS-017",
        "widgets": [
            {"kind": "label", "text": "WS-017", "style": "heading"},
            {"kind": "sidebar", "id": "panel.sections", "selected": selected,
             "action": {"capability": "crowdstrike.panel", "method": "section"},
             "args": {"id": "aaa"},
             "items": [
                 {"key": "overview", "text": "Overview"},
                 {"key": "network", "text": "Network"},
                 {"key": "updates", "text": "Updates", "muted": true}
             ],
             "body": [
                 {"kind": "label", "text": body_text, "style": "strong"},
                 {"kind": "text", "id": "panel.filter", "default": "dns"}
             ]}
        ]
    }))
    .expect("the module's screen parses")
}

/// One piece of text on screen.
struct Drawn {
    text: String,
    at: egui::Pos2,
    alpha: u8,
    color: egui::Color32,
}

struct Screen {
    ctx: egui::Context,
    time: f64,
    inputs: HashMap<String, String>,
}

impl Screen {
    fn new() -> Self {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        set_animations(true);
        Self { ctx, time: 0.0, inputs: HashMap::new() }
    }

    /// Draw `view` for one frame, with `events`; what it painted, and what it
    /// asked the module for.
    fn frame(&mut self, view: &View, events: Vec<egui::Event>) -> (Vec<Drawn>, Option<Invoke>) {
        self.time += 1.0 / 60.0;
        let mut clicked = None;
        let inputs = &mut self.inputs;
        let out = self.ctx.run(
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
                    clicked = render_view(ui, view, inputs, None, 0.0);
                });
            },
        );
        let mut drawn = Vec::new();
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                let color = t
                    .galley
                    .job
                    .sections
                    .first()
                    .map_or(t.fallback_color, |s| s.format.color);
                drawn.push(Drawn {
                    text: t.galley.text().to_string(),
                    at: t.pos,
                    alpha: t.fallback_color.a(),
                    color,
                });
            }
        }
        (drawn, clicked)
    }

    /// Draw `view` until its animations have finished.
    fn settle(&mut self, view: &View) -> Vec<Drawn> {
        let mut last = Vec::new();
        for _ in 0..60 {
            last = self.frame(view, vec![]).0;
        }
        last
    }

    /// Press and release at `at`; what the click asked the module for.
    fn click(&mut self, view: &View, at: egui::Pos2) -> Option<Invoke> {
        let mut asked = None;
        for pressed in [true, false] {
            let (_, invoke) = self.frame(
                view,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
            asked = asked.or(invoke);
        }
        asked
    }
}

fn find<'a>(drawn: &'a [Drawn], text: &str) -> Option<&'a Drawn> {
    drawn.iter().find(|d| d.text == text)
}

/// Picking a section is the module's business: it is told which, with the
/// args the sidebar carries — which host — so it can answer with that section.
#[test]
fn picking_a_section_tells_the_module_which_one() {
    let mut s = Screen::new();
    let view = panel("overview", "Overview body");
    let drawn = s.settle(&view);
    let network = find(&drawn, "Network").expect("the item is drawn").at;

    let invoke = s
        .click(&view, network + egui::vec2(4.0, 6.0))
        .expect("picking a section calls the module");
    assert_eq!(invoke.action.capability, "crowdstrike.panel");
    assert_eq!(invoke.action.method, "section");
    assert_eq!(invoke.args.get("key"), Some(&json!("network")), "which one");
    assert_eq!(invoke.args.get("id"), Some(&json!("aaa")), "about which host");
    assert!(!invoke.open_in_tab, "the answer replaces this screen");
}

/// Folding the list away asks nothing of the module, takes the items away,
/// and gives their width to the section.
#[test]
fn folding_the_list_is_the_hosts_alone_and_gives_the_section_the_width() {
    let mut s = Screen::new();
    let view = panel("overview", "Overview body");
    let before = s.settle(&view);
    let body_x = find(&before, "Overview body").expect("the body is drawn").at.x;
    let item = find(&before, "Overview").expect("the item is drawn").at;

    // The toggle is the top of the list, above the first item.
    let toggle = egui::pos2(item.x - 2.0, item.y - 20.0);
    assert!(s.click(&view, toggle).is_none(), "folding calls nobody");

    let after = s.settle(&view);
    for gone in ["Overview", "Network", "Updates"] {
        assert!(find(&after, gone).is_none(), "{gone} is folded away");
    }
    let moved = find(&after, "Overview body").expect("the body is still there").at.x;
    assert!(
        body_x - moved > 100.0,
        "the section takes the list's width: {body_x} -> {moved}"
    );

    // And back again.
    assert!(s.click(&view, toggle).is_none());
    let back = s.settle(&view);
    assert!(find(&back, "Network").is_some(), "unfolded, the list returns");
}

/// A section change is a whole new screen from the module. The fold belongs to
/// the sidebar's id, not to that screen, so it does not spring open every time
/// a section is picked.
#[test]
fn a_folded_list_stays_folded_across_sections() {
    let mut s = Screen::new();
    let first = panel("overview", "Overview body");
    let drawn = s.settle(&first);
    let item = find(&drawn, "Overview").unwrap().at;
    s.click(&first, egui::pos2(item.x - 2.0, item.y - 20.0));
    s.settle(&first);

    let next = panel("network", "Network body");
    let drawn = s.settle(&next);
    assert!(find(&drawn, "Network body").is_some(), "the new section is shown");
    assert!(find(&drawn, "Overview").is_none(), "and the list is still folded");
}

/// The sidebar is the same widget on both screens, so only the section
/// changes: the list stays put and fully drawn while the new body arrives. If
/// the sidebar were taken for a new widget, the whole list would fly in again
/// on every click.
#[test]
fn the_list_stays_put_while_the_section_changes() {
    let mut s = Screen::new();
    let first = panel("overview", "Overview body");
    let settled = s.settle(&first);
    let was = find(&settled, "Network").unwrap().at;

    let next = panel("network", "Network body");
    let (drawn, _) = s.frame(&next, vec![]);
    let (drawn2, _) = s.frame(&next, vec![]);
    let item = find(&drawn2, "Network").expect("the list is drawn");
    assert_eq!(item.alpha, 255, "the list does not fade in again");
    assert!((item.at - was).length() < 0.5, "nor move");
    // The new body is on its way in, not already there.
    let arriving = find(&drawn, "Network body")
        .or_else(|| find(&drawn2, "Network body"))
        .map_or(0, |d| d.alpha);
    assert!(arriving < 255, "the section's change is animated: alpha {arriving}");
}

/// A section that has nothing to show yet is still listed, only quieter, so
/// the list never looks more complete than it is.
#[test]
fn a_muted_section_is_listed_but_quieter() {
    let mut s = Screen::new();
    let drawn = s.settle(&panel("overview", "Overview body"));
    let muted = find(&drawn, "Updates").expect("a muted section is still listed");
    let plain = find(&drawn, "Network").unwrap();
    assert_eq!(muted.color, color::TEXT_MUTED);
    assert_eq!(plain.color, color::TEXT);
}

/// A field inside a section is a field on the screen: its value goes with
/// every call, and it is forgotten with the rest when the screen closes.
#[test]
fn a_field_inside_a_section_is_still_a_field_on_the_screen() {
    let view = panel("overview", "Overview body");
    assert!(widget_ids(&view).contains(&"panel.filter".to_string()));

    let mut inputs = HashMap::new();
    inputs.insert("panel.filter".to_string(), "gateway".to_string());
    let mut sent = serde_json::Map::new();
    collect_ids(&view.widgets, &inputs, &mut sent);
    assert_eq!(sent.get("panel.filter"), Some(&json!("gateway")));
}
