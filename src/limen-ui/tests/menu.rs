//! The right-click menu: how it arrives, how it goes, and what it answers with.

use eframe::egui;
use limen_ui::*;

/// A table of eight rows carrying one menu, drawn frame by frame.
///
/// The menu is not part of the table's own drawing — it is an area over it that
/// outlives the click that opened it — so the only way to test it is to run the
/// frames, which is what the application does.
struct Table {
    ctx: egui::Context,
    time: f64,
    menu: Vec<MenuItem>,
    picked: Option<Invoke>,
}

impl Table {
    fn new(menu_json: &str) -> Table {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        Table {
            ctx,
            time: 0.0,
            menu: serde_json::from_str(menu_json).expect("menu"),
            picked: None,
        }
    }

    /// One frame, with whatever the user did during it.
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        let menu = &self.menu;
        let picked = &mut self.picked;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let cols = vec!["name".to_string()];
                let rows: Vec<Vec<String>> =
                    (0..8).map(|i| vec![format!("row {i}")]).collect();
                let ids: Vec<String> = (0..8).map(|i| i.to_string()).collect();
                render_table(ui, &cols, &rows, &ids, menu, &[], None, picked);
            });
        });
    }

    /// Several frames of nothing happening — the pointer stays where it was.
    fn settle(&mut self) {
        for _ in 0..15 {
            self.frame(Vec::new());
        }
    }

    fn moved(&mut self, to: egui::Pos2) {
        self.frame(vec![egui::Event::PointerMoved(to)]);
    }

    /// A press and its release, which is what egui counts as a click.
    fn click(&mut self, at: egui::Pos2, button: egui::PointerButton) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }

    /// Every menu panel currently on screen, left to right.
    ///
    /// Read out of egui rather than worked out from the layout, so the test
    /// clicks where the menu actually is and not where the arithmetic in it
    /// says it should be.
    fn panels(&self) -> Vec<egui::Rect> {
        let visible = self.ctx.memory(|m| m.areas().visible_layer_ids());
        let mut rects: Vec<egui::Rect> = visible
            .into_iter()
            .filter(|l| l.order == egui::Order::Foreground)
            .filter_map(|l| egui::AreaState::load(&self.ctx, l.id).map(|a| a.rect()))
            .collect();
        rects.sort_by(|a, b| a.left().total_cmp(&b.left()));
        rects
    }
}

/// The first entry of a panel: past the frame's margin, into the first row.
fn first_entry(panel: egui::Rect) -> egui::Pos2 {
    panel.min + egui::vec2(30.0, 14.0)
}

/// Somewhere in the rows, well clear of the header.
const IN_THE_TABLE: egui::Pos2 = egui::pos2(120.0, 120.0);

const ONE_ENTRY: &str = r#"[{"label":"About","action":{"capability":"c","method":"about"}}]"#;

/// A menu that is not there yet, and one that is no longer there, are both
/// nothing on screen; between them it has to be drawn on frames where the click
/// that opened it is long over and the one that shut it has already happened.
#[test]
fn a_right_click_opens_a_menu_that_arrives_and_then_leaves() {
    let mut t = Table::new(ONE_ENTRY);
    t.settle();
    assert!(t.panels().is_empty(), "nothing was asked for yet");

    t.click(IN_THE_TABLE, egui::PointerButton::Secondary);
    t.settle();
    let panels = t.panels();
    assert_eq!(panels.len(), 1, "one menu, where the pointer was");
    assert!(
        panels[0].contains(IN_THE_TABLE),
        "it grows from the point that asked for it: {:?}",
        panels[0]
    );

    // Escape dismisses it — and it is still on screen for the frames it takes to
    // go, which is the whole point of the menu outliving its own state.
    t.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    t.frame(Vec::new());
    assert_eq!(t.panels().len(), 1, "it should still be leaving");
    t.settle();
    assert!(t.panels().is_empty(), "and then be gone");
}

/// What is chosen comes back carrying the row it was chosen on, and choosing
/// closes the menu.
#[test]
fn choosing_an_entry_answers_with_the_row_it_opened_on() {
    let mut t = Table::new(ONE_ENTRY);
    t.settle();
    t.click(IN_THE_TABLE, egui::PointerButton::Secondary);
    t.settle();

    let entry = first_entry(t.panels()[0]);
    t.click(entry, egui::PointerButton::Primary);
    let picked = t.picked.clone().expect("the entry was chosen");
    assert_eq!(picked.action.method, "about");
    assert!(picked.args.contains_key("id"), "the row rides along: {:?}", picked.args);

    t.settle();
    assert!(t.panels().is_empty(), "choosing something closes the menu");
}

/// A submenu flies out beside the entry that owns it, and is a menu like any
/// other: what is chosen in it is what comes back.
#[test]
fn a_submenu_opens_beside_the_entry_that_owns_it() {
    let mut t = Table::new(
        r#"[{"label":"More","children":[
              {"label":"Copy","action":{"capability":"c","method":"copy"}}]},
            {"label":"About","action":{"capability":"c","method":"about"}}]"#,
    );
    t.settle();
    t.click(IN_THE_TABLE, egui::PointerButton::Secondary);
    t.settle();

    // Resting on the entry opens it; a menu you have to click into is one that
    // behaves unlike every other menu on the machine.
    let root = t.panels()[0];
    t.moved(first_entry(root));
    t.settle();
    let panels = t.panels();
    assert_eq!(panels.len(), 2, "the flyout should be open: {panels:?}");
    assert!(panels[1].left() > root.left(), "beside it, not over it");

    t.click(first_entry(panels[1]), egui::PointerButton::Primary);
    let picked = t.picked.clone().expect("the submenu entry was chosen");
    assert_eq!(picked.action.method, "copy");
    t.settle();
    assert!(t.panels().is_empty(), "both panels go together");
}

/// A press anywhere else puts the menu away — including, in the same gesture,
/// the right-click that opens the next one.
#[test]
fn a_press_outside_puts_it_away_and_the_next_right_click_opens_another() {
    let mut t = Table::new(ONE_ENTRY);
    t.settle();
    t.click(IN_THE_TABLE, egui::PointerButton::Secondary);
    t.settle();
    assert_eq!(t.panels().len(), 1);

    // Somewhere else in the table: the old menu goes, and a new one opens where
    // this click was — a right-click is both at once.
    let elsewhere = IN_THE_TABLE + egui::vec2(300.0, 60.0);
    t.click(elsewhere, egui::PointerButton::Secondary);
    t.settle();
    let panels = t.panels();
    assert_eq!(panels.len(), 1, "one menu at a time: {panels:?}");
    assert!(panels[0].contains(elsewhere), "and it is the new one");

    // A left-click away from it is a dismissal and nothing else.
    t.click(egui::pos2(900.0, 400.0), egui::PointerButton::Primary);
    t.settle();
    assert!(t.panels().is_empty());
}
