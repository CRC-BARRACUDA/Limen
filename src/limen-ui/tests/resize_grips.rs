//! The window's own resize strips: the cursor and the resize, on the frame the
//! pointer gets to an edge.

use eframe::egui;
use limen_ui::*;

struct Win {
    ctx: egui::Context,
    time: f64,
}

impl Win {
    fn new() -> Self {
        Self { ctx: egui::Context::default(), time: 0.0 }
    }

    /// One frame of a 1000×700 window with something in it. Whether a resize
    /// was asked for (and which way), and the cursor.
    fn frame(&mut self, events: Vec<egui::Event>) -> (Option<egui::ResizeDirection>, egui::CursorIcon) {
        self.time += 1.0 / 60.0;
        let out = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                window_resize_grips(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for i in 0..100 {
                            ui.label(format!("row {i}"));
                        }
                    });
                });
            },
        );
        let resize = out.viewport_output.values().flat_map(|v| v.commands.iter()).find_map(|c| match c {
            egui::ViewportCommand::BeginResize(d) => Some(*d),
            _ => None,
        });
        (resize, out.platform_output.cursor_icon)
    }
}

fn press(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }
}

/// A hand that reaches the edge and stops gets no more frames: the cursor has
/// to turn on the one it arrived in, and a press there has to resize.
#[test]
fn the_cursor_turns_and_a_press_resizes_on_the_frame_the_pointer_arrives() {
    use egui::{CursorIcon as Cur, ResizeDirection as Dir};
    for (at, dir, cursor) in [
        (egui::pos2(997.0, 300.0), Dir::East, Cur::ResizeEast),
        (egui::pos2(3.0, 300.0), Dir::West, Cur::ResizeWest),
        (egui::pos2(500.0, 3.0), Dir::North, Cur::ResizeNorth),
        (egui::pos2(500.0, 697.0), Dir::South, Cur::ResizeSouth),
        (egui::pos2(993.0, 300.0), Dir::East, Cur::ResizeEast),
        (egui::pos2(990.0, 690.0), Dir::SouthEast, Cur::ResizeSouthEast),
        (egui::pos2(5.0, 5.0), Dir::NorthWest, Cur::ResizeNorthWest),
    ] {
        let mut w = Win::new();
        w.frame(vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0))]);
        let (resize, seen) = w.frame(vec![egui::Event::PointerMoved(at)]);
        assert_eq!(seen, cursor, "the cursor at {at:?}, at once");
        assert_eq!(resize, None, "arriving resizes nothing");
        assert_eq!(w.frame(vec![press(at)]).0, Some(dir), "a press at {at:?}");

        // Arriving and pressing in one frame is a press at the edge all the same.
        let mut w = Win::new();
        assert_eq!(w.frame(vec![egui::Event::PointerMoved(at), press(at)]).0, Some(dir));
    }
}

#[test]
fn away_from_the_edge_nothing_is_a_grip() {
    let mut w = Win::new();
    for at in [egui::pos2(985.0, 300.0), egui::pos2(500.0, 350.0), egui::pos2(500.0, 20.0)] {
        let (resize, cursor) = w.frame(vec![egui::Event::PointerMoved(at)]);
        assert!(resize.is_none());
        assert!(!format!("{cursor:?}").starts_with("Resize"), "{at:?}: {cursor:?}");
        assert!(w.frame(vec![press(at)]).0.is_none(), "a press at {at:?} is the page's");
        w.frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
    }
}

/// Something dragged into the edge — a scroll bar, a selection — is not a
/// resize, and does not turn the cursor.
#[test]
fn a_drag_that_wanders_to_the_edge_resizes_nothing() {
    let mut w = Win::new();
    let start = egui::pos2(500.0, 300.0);
    w.frame(vec![egui::Event::PointerMoved(start)]);
    w.frame(vec![press(start)]);
    for x in [700.0, 900.0, 997.0, 999.0] {
        let (resize, cursor) = w.frame(vec![egui::Event::PointerMoved(egui::pos2(x, 300.0))]);
        assert!(resize.is_none(), "at {x}");
        assert!(!format!("{cursor:?}").starts_with("Resize"), "at {x}: {cursor:?}");
    }
}

/// A resize takes the pointer away: the release never arrives, and egui goes
/// on thinking the button is held. The edges have to work all the same — the
/// cursor turning, and the next press resizing.
#[test]
fn after_a_resize_whose_release_never_came_the_edges_still_answer() {
    use egui::{CursorIcon as Cur, ResizeDirection as Dir};
    let mut w = Win::new();
    let east = egui::pos2(997.0, 300.0);
    w.frame(vec![egui::Event::PointerMoved(east)]);
    assert_eq!(w.frame(vec![press(east)]).0, Some(Dir::East));
    // No release. The pointer comes back, to the page and then to an edge.
    w.frame(vec![egui::Event::PointerMoved(egui::pos2(500.0, 300.0))]);
    let south = egui::pos2(500.0, 697.0);
    assert_eq!(w.frame(vec![egui::Event::PointerMoved(south)]).1, Cur::ResizeSouth, "the cursor still turns");
    assert_eq!(w.frame(vec![egui::Event::PointerMoved(east)]).1, Cur::ResizeEast);
    assert_eq!(w.frame(vec![press(east)]).0, Some(Dir::East), "and a press still resizes");
}

/// Moving the window by its title bar loses the release the same way.
#[test]
fn after_the_window_was_moved_the_edges_still_answer() {
    let mut w = Win::new();
    let bar = egui::pos2(500.0, 300.0);
    w.frame(vec![egui::Event::PointerMoved(bar)]);
    w.frame(vec![press(bar)]);
    window_took_pointer(&w.ctx);
    let east = egui::pos2(997.0, 300.0);
    assert_eq!(w.frame(vec![egui::Event::PointerMoved(east)]).1, egui::CursorIcon::ResizeEast);
}
