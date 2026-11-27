//! A dropdown's menu: as tall as its options up to a point, and scrolling
//! beyond it.

use eframe::egui;
use limen_ui::*;

struct Screen {
    ctx: egui::Context,
    time: f64,
    value: String,
    options: Vec<String>,
    /// Where the field was drawn, and the menu's area.
    field: egui::Rect,
    popup: egui::Id,
}

impl Screen {
    fn new(count: usize, value: &str) -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            value: value.to_string(),
            options: (0..count).map(|i| format!("option {i:02}")).collect(),
            field: egui::Rect::NOTHING,
            popup: egui::Id::NULL,
        }
    }

    /// One frame of a 1000×700 window with the dropdown near its top.
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let (value, options) = (&mut self.value, &self.options);
        let (mut field, mut popup) = (self.field, self.popup);
        let _ = self.ctx.run(
            egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    popup = ui.make_persistent_id("dd").with("dd_popup");
                    field = dropdown(ui, "dd", value, options).rect;
                });
            },
        );
        self.field = field;
        self.popup = popup;
    }

    fn settle(&mut self) {
        for _ in 0..40 {
            self.frame(vec![]);
        }
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }

    fn open(&mut self) {
        self.frame(vec![]);
        self.click(self.field.center());
    }

    /// The menu, while it is open.
    fn menu(&self) -> Option<egui::Rect> {
        if !self.ctx.memory(|m| m.is_popup_open(self.popup)) {
            return None;
        }
        self.ctx.memory(|m| m.area_rect(self.popup))
    }
}

/// A menu of a few options is as tall as they are; one of many stops growing
/// and stays on the screen.
#[test]
fn a_long_menu_stops_growing_and_a_short_one_is_as_tall_as_its_options() {
    let mut short = Screen::new(4, "option 00");
    short.open();
    let few = short.menu().expect("the menu is open").height();

    let mut long = Screen::new(60, "option 00");
    long.open();
    let many = long.menu().expect("the menu is open");

    let row = few / 4.0;
    assert!(many.height() > few, "more options, a taller menu: {} vs {few}", many.height());
    assert!(
        many.height() < row * (DROPDOWN_MAX_ROWS + 1.5),
        "sixty options, and the menu is {} tall ({row} a row)",
        many.height()
    );
    assert!(many.bottom() <= 700.0, "the menu ends at {} in a window 700 tall", many.bottom());

    // A list only a little over the limit is still held to it.
    let mut twelve = Screen::new(12, "option 00");
    twelve.open();
    assert!((twelve.menu().unwrap().height() - many.height()).abs() < 1.0);
}

/// Opened, the menu shows where you are: the current value is in the middle
/// of it, not somewhere below the fold of a list that starts at its top.
#[test]
fn the_menu_opens_at_the_current_value() {
    let mut s = Screen::new(60, "option 40");
    s.open();
    let menu = s.menu().expect("the menu is open");
    // What is in the middle of the menu is what is picked by a click there.
    s.click(menu.center());
    assert!(s.menu().is_none(), "picking closes the menu");
    let picked: i32 = s.value.trim_start_matches("option ").parse().unwrap();
    assert!((picked - 40).abs() <= 1, "the middle of the menu was {} — the value was option 40", s.value);
}

/// Every option can still be reached: the ones past the fold by scrolling.
#[test]
fn an_option_below_the_fold_is_reached_by_scrolling() {
    let mut s = Screen::new(60, "option 00");
    s.open();
    let menu = s.menu().expect("the menu is open");
    let inside = menu.center();
    s.frame(vec![egui::Event::PointerMoved(inside)]);
    // A long way down — to the end of the list.
    for _ in 0..30 {
        s.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            modifiers: Default::default(),
        }]);
    }
    s.settle();
    assert!(s.menu().is_some(), "scrolling does not close the menu");
    // The last option is now the last row of the menu.
    s.click(egui::pos2(menu.center().x, s.menu().unwrap().bottom() - 12.0));
    assert_eq!(s.value, "option 59");
}

/// Picking an option that was in view works as it always did.
#[test]
fn an_option_in_view_is_picked_with_a_click() {
    let mut s = Screen::new(4, "option 00");
    s.open();
    let menu = s.menu().expect("the menu is open");
    s.click(egui::pos2(menu.center().x, menu.bottom() - 12.0));
    assert_eq!(s.value, "option 03");
    assert!(s.menu().is_none());
}
