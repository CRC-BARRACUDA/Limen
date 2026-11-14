//! The terminal: a transcript and a prompt in one box, as an SSH session.

use std::collections::HashMap;

use eframe::egui;
use limen_ui::*;
use serde_json::json;

fn view(busy: bool) -> View {
    serde_json::from_value(json!({
        "title": "Console",
        "widgets": [{
            "kind": "terminal", "id": "rtr.cmd", "busy": busy,
            "lines": "C:\\> cd Users\nC:\\Users> dir /b\nPublic",
            "prompt": "C:\\Users>",
            "on_submit": {"capability": "crowdstrike.panel", "method": "console_run",
                          "args": {"id": "aaa"}}
        }]
    }))
    .unwrap()
}

struct Term {
    ctx: egui::Context,
    time: f64,
    inputs: HashMap<String, String>,
    busy: bool,
}

impl Term {
    fn new(busy: bool) -> Self {
        let ctx = egui::Context::default();
        apply_theme(&ctx);
        let mut t = Term { ctx, time: 0.0, inputs: HashMap::new(), busy };
        // A few frames: the terminal takes the keyboard when it first shows.
        for _ in 0..3 {
            t.frame(vec![]);
        }
        t
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Option<Invoke> {
        self.time += 1.0 / 60.0;
        let v = view(self.busy);
        let mut asked = None;
        let inputs = &mut self.inputs;
        let _ = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    asked = render_view(ui, &v, inputs, None, 0.0);
                });
            },
        );
        asked
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> Option<Invoke> {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }])
    }

    fn typing(&mut self, text: &str) {
        self.frame(vec![egui::Event::Text(text.into())]);
    }

    /// How tall the terminal's box is, as painted — once the screen's
    /// entrance is over, which dims its colours until it is.
    fn height(&mut self) -> f32 {
        for _ in 0..40 {
            self.frame(vec![]);
        }
        self.time += 1.0 / 60.0;
        let v = view(self.busy);
        let inputs = &mut self.inputs;
        let out = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_view(ui, &v, inputs, None, 0.0);
                });
            },
        );
        out.shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == color::BG_ELEVATED => Some(r.rect.height()),
                _ => None,
            })
            .expect("the terminal's box is painted")
    }

    fn focused(&self) -> bool {
        let input = egui::Id::new(("limen_terminal_input", "rtr.cmd"));
        self.ctx.memory(|m| m.has_focus(input))
    }

    fn font(&self) -> f32 {
        self.ctx
            .data(|d| d.get_temp::<f32>(terminal_font_id("rtr.cmd")))
            .unwrap_or(13.0)
    }
}

/// Type a command, press Enter: it goes to the module with the line, the line
/// empties as a shell's does, and the keyboard stays at the prompt.
#[test]
fn enter_sends_the_line_and_empties_it() {
    let mut t = Term::new(false);
    assert!(t.focused(), "the prompt has the keyboard as soon as it shows");
    t.typing("ls C:\\Users");
    let sent = t
        .key(egui::Key::Enter, egui::Modifiers::NONE)
        .expect("Enter runs the command");
    assert_eq!(sent.action.method, "console_run");
    assert_eq!(sent.args.get("rtr.cmd"), Some(&json!("ls C:\\Users")));
    assert_eq!(sent.args.get("id"), Some(&json!("aaa")), "with the terminal's own args");
    assert_eq!(t.inputs.get("rtr.cmd").map(String::as_str), Some(""), "the line empties");
    t.frame(vec![]);
    assert!(t.focused(), "and the cursor stays at the prompt");
}

/// Enter on an empty line is a new prompt in a shell, not a command.
#[test]
fn an_empty_line_sends_nothing() {
    let mut t = Term::new(false);
    t.typing("   ");
    assert!(t.key(egui::Key::Enter, egui::Modifiers::NONE).is_none());
}

/// While a command runs there is no prompt to type at.
#[test]
fn a_busy_terminal_takes_no_command() {
    let mut t = Term::new(true);
    t.typing("ls");
    assert!(t.key(egui::Key::Enter, egui::Modifiers::NONE).is_none());
    assert_eq!(t.inputs.get("rtr.cmd").map(String::as_str).unwrap_or(""), "", "typing went nowhere");
}

/// Ctrl+ and Ctrl− size the terminal's text, Ctrl+0 puts it back — and the
/// window's own zoom, which those keys drive everywhere else, does not move.
#[test]
fn ctrl_plus_and_minus_size_the_text_not_the_window() {
    let mut t = Term::new(false);
    let ctrl = egui::Modifiers::COMMAND;
    t.key(egui::Key::Equals, ctrl);
    t.key(egui::Key::Plus, ctrl);
    assert_eq!(t.font(), 15.0, "two steps bigger");
    t.key(egui::Key::Minus, ctrl);
    assert_eq!(t.font(), 14.0, "one smaller");
    t.key(egui::Key::Num0, ctrl);
    assert_eq!(t.font(), 13.0, "and back");
    t.frame(vec![]);
    assert_eq!(t.ctx.zoom_factor(), 1.0, "the window was never zoomed");
}

/// The box keeps its size whatever the text size: Ctrl+ fits fewer lines in
/// it, Ctrl− more, and the screen around it does not move.
#[test]
fn sizing_the_text_does_not_resize_the_box() {
    let mut t = Term::new(false);
    let before = t.height();
    for _ in 0..4 {
        t.key(egui::Key::Equals, egui::Modifiers::COMMAND);
    }
    assert_eq!(t.font(), 17.0, "the text did grow");
    let bigger = t.height();
    for _ in 0..8 {
        t.key(egui::Key::Minus, egui::Modifiers::COMMAND);
    }
    let smaller = t.height();
    assert!((bigger - before).abs() < 0.5 && (smaller - before).abs() < 0.5,
            "the box stayed {before}: {bigger} bigger, {smaller} smaller");
}

/// The box at `fill`, as the host panel shows it — in a sidebar's section, on a
/// page that scrolls — in a window `tall` points high: where its bottom edge
/// is, and how far down the page could scroll.
fn filled_box(tall: f32) -> (f32, f32) {
    let view: View = serde_json::from_value(json!({
        "title": "PC", "widgets": [{
            "kind": "sidebar", "id": "panel.sections", "selected": "console",
            "action": {"capability": "c", "method": "section"},
            "items": [{"key": "console", "text": "Console"}],
            "body": [
                {"kind": "label", "text": "Console", "style": "strong"},
                {"kind": "terminal", "id": "rtr.cmd", "prompt": "C:\\>", "fill": true,
                 "on_submit": {"capability": "c", "method": "run"}}
            ]}]
    })).unwrap();
    let ctx = egui::Context::default();
    apply_theme(&ctx);
    let mut inputs = HashMap::new();
    let (mut bottom, mut overflow) = (0.0, 0.0);
    for f in 0..60 {
        let out = ctx.run(
            egui::RawInput {
                time: Some(f as f64 / 60.0),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, tall))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let s = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        render_view(ui, &view, &mut inputs, None, 0.0);
                    });
                    overflow = s.content_size.y - s.inner_rect.height();
                });
            },
        );
        if let Some(b) = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == color::BG_ELEVATED => Some(r.rect.bottom()),
            _ => None,
        }) {
            bottom = b;
        }
    }
    (bottom, overflow)
}

/// A terminal that fills goes down to the bottom of the window, follows the
/// window when it is made taller, and never pushes the page into scrolling.
#[test]
fn a_filling_terminal_reaches_the_bottom_of_the_window_and_follows_it() {
    let (short, spill_short) = filled_box(700.0);
    let (tall, spill_tall) = filled_box(900.0);
    assert!(700.0 - short < 30.0, "it ends at the window's bottom: {short} of 700");
    assert!(spill_short <= 0.5 && spill_tall <= 0.5,
            "and the page does not scroll: {spill_short}, {spill_tall}");
    assert!((tall - short - 200.0).abs() < 2.0, "200 more window, 200 more box: {short} -> {tall}");
}
