//! A screen that gains or loses a widget shows which widget it was.
//!
//! Modules answer with the whole view every time, so "one line was added" only
//! exists as a difference between two screens. What is measured here is what
//! somebody sees: where each widget is painted, and how solid it is on the way.

use eframe::egui;
use limen_ui as ui;

#[derive(Debug, Clone)]
struct Ink {
    text: String,
    x: f32,
    y: f32,
    alpha: u8,
}

fn collect(shape: &egui::Shape, out: &mut Vec<Ink>) {
    match shape {
        egui::Shape::Text(t) => out.push(Ink {
            text: t.galley.text().to_string(),
            x: t.pos.x,
            y: t.pos.y,
            // The opacity a `Ui` was set to is folded into the painted colour.
            alpha: t.fallback_color.a(),
        }),
        egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

/// Draw this view at `t` seconds and hand back everything it painted.
fn frame(ctx: &egui::Context, t: f64, view: &ui::View) -> Vec<Ink> {
    let input = egui::RawInput {
        time: Some(t),
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        ..Default::default()
    };
    let mut inputs = std::collections::HashMap::new();
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Already revealed: the tab's own entrance is not what is being
            // measured here.
            ui::render_view(ui, view, &mut inputs, None, 0.0);
        });
    });
    let mut ink = Vec::new();
    for c in &out.shapes {
        collect(&c.shape, &mut ink);
    }
    ink
}

fn view(widgets: &str) -> ui::View {
    serde_json::from_str(&format!(r#"{{"title":"T","widgets":[{widgets}]}}"#)).expect("a view")
}

fn label(text: &str) -> String {
    format!(r#"{{"kind":"label","text":"{text}"}}"#)
}

fn find(ink: &[Ink], text: &str) -> Option<Ink> {
    // Buttons are painted in capitals by the theme; what is being looked for is
    // the widget, not its casing.
    ink.iter()
        .find(|i| i.text.eq_ignore_ascii_case(text))
        .cloned()
}

fn at(ink: &[Ink], text: &str) -> Ink {
    find(ink, text).unwrap_or_else(|| panic!("{text} was not painted: {ink:?}"))
}

/// Let a screen arrive and settle, so what follows is measured against a screen
/// standing still rather than against an entrance.
fn settled(ctx: &egui::Context, v: &ui::View) -> Vec<Ink> {
    let mut last = Vec::new();
    for t in [0.0, 0.3, 0.6, 1.0] {
        last = frame(ctx, t, v);
    }
    last
}

/// The case this was written for: a module answers a press with the same screen
/// plus a line saying what happened. The line opens from nothing and the screen
/// travels down with it — and nothing that was already there is redrawn as if it
/// were new.
#[test]
fn a_line_added_to_a_screen_opens_and_pushes_the_rest_down() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(
        &ctx,
        &view(&[label("Where to look"), label("Search")].join(",")),
    );
    let was = at(&before, "Where to look").y;

    let after = view(
        &[
            label("Added /var/log"),
            label("Where to look"),
            label("Search"),
        ]
        .join(","),
    );
    // The frame it arrives on: known of, but at no height yet, and nothing else
    // has stirred.
    let arrives = frame(&ctx, 1.02, &after);
    assert_eq!(at(&arrives, "Where to look").y, was, "{arrives:?}");
    assert_eq!(at(&arrives, "Where to look").alpha, 255, "{arrives:?}");

    let during = frame(&ctx, 1.1, &after);
    let note = at(&during, "Added /var/log");
    assert!(note.alpha < 255, "the notice snapped in: {note:?}");
    let moved = at(&during, "Where to look").y - was;
    assert!(moved > 0.0, "nothing made room for it: {during:?}");
    // Still at full strength, and so is everything below it: this is an edit,
    // not a page that has reloaded.
    assert_eq!(at(&during, "Where to look").alpha, 255, "{during:?}");
    assert_eq!(at(&during, "Search").alpha, 255, "{during:?}");

    let settled_after = frame(&ctx, 1.6, &after);
    let note = at(&settled_after, "Added /var/log");
    assert_eq!(note.alpha, 255);
    let moved_by = at(&settled_after, "Where to look").y - was;
    assert!(
        moved_by > moved,
        "it should have gone on moving: {moved} then {moved_by}"
    );
    // And the screen it pushed down is in one piece, in order.
    assert!(note.y < at(&settled_after, "Where to look").y);
    assert!(at(&settled_after, "Where to look").y < at(&settled_after, "Search").y);
}

/// A line that has gone closes, and the gap closes after it rather than
/// vanishing from under what follows.
#[test]
fn a_line_taken_away_closes_and_the_rest_comes_up() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let three = view(&[label("one"), label("two"), label("three")].join(","));
    let before = settled(&ctx, &three);
    let gap = at(&before, "three").y - at(&before, "two").y;

    let two = view(&[label("one"), label("three")].join(","));
    frame(&ctx, 1.02, &two);
    let during = frame(&ctx, 1.07, &two);
    let ghost = at(&during, "two");
    assert!(ghost.alpha < 255, "it should be fading as it closes: {ghost:?}");
    let risen = at(&before, "three").y - at(&during, "three").y;
    assert!(
        risen >= 0.0 && risen < gap,
        "the gap snapped shut instead of closing: {risen} of {gap}"
    );

    let after = frame(&ctx, 1.4, &two);
    assert!(find(&after, "two").is_none(), "the ghost outstayed: {after:?}");
    let risen = at(&before, "three").y - at(&after, "three").y;
    assert!(
        (risen - gap).abs() < 1.0,
        "expected the gap closed, closed {risen} of {gap}"
    );
}

/// A screen with nothing in common with the last one is another screen, and
/// plays the entrance a screen gets rather than a dozen separate arrivals.
#[test]
fn another_screen_altogether_replays_the_entrance() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    settled(&ctx, &view(&[label("one"), label("two")].join(",")));

    let other: Vec<String> = (0..10).map(|i| label(&format!("other {i}"))).collect();
    let other = view(&other.join(","));
    frame(&ctx, 1.02, &other);
    let during = frame(&ctx, 1.08, &other);
    // A cascade: the top has arrived and the bottom has not started.
    assert!(at(&during, "other 0").alpha > 0, "{during:?}");
    assert!(
        find(&during, "other 9").is_none(),
        "the whole screen arrived at once: {during:?}"
    );
}

/// A label whose words change is the same label: it does not close and reopen,
/// which is what a progress line would do on every tick.
#[test]
fn a_line_that_changes_what_it_says_does_not_re_arrive() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let ticking = |n: u32| view(&[label("Looking…"), label(&format!("{n} found"))].join(","));
    let before = settled(&ctx, &ticking(3));

    let during = frame(&ctx, 1.02, &ticking(9));
    let counted = at(&during, "9 found");
    assert_eq!(counted.alpha, 255, "it faded in as if new: {during:?}");
    assert_eq!(counted.y, at(&before, "3 found").y, "and it stayed put");
    // Nothing is left closing underneath it.
    assert!(find(&during, "3 found").is_none(), "{during:?}");
}

fn button(text: &str) -> String {
    format!(
        r#"{{"kind":"button","text":"{text}","action":{{"capability":"c","method":"m"}}}}"#
    )
}

fn row(children: &[String]) -> String {
    format!(r#"{{"kind":"row","children":[{}]}}"#, children.join(","))
}

/// Every kind of widget goes through the same machinery, so a control arriving
/// is a control arriving whatever it is — here a checkbox, which is neither a
/// label nor a table.
#[test]
fn a_checkbox_added_to_a_screen_opens_like_anything_else() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(&ctx, &view(&[label("Options"), label("Go")].join(",")));
    let was = at(&before, "Go").y;

    let with_box = view(
        &[
            label("Options"),
            r#"{"kind":"checkbox","id":"deep","label":"Search inside archives"}"#.to_string(),
            label("Go"),
        ]
        .join(","),
    );
    frame(&ctx, 1.02, &with_box);
    let during = frame(&ctx, 1.1, &with_box);
    let box_ = at(&during, "Search inside archives");
    assert!(box_.alpha < 255, "the checkbox snapped in: {box_:?}");
    let moved = at(&during, "Go").y - was;
    assert!(moved > 0.0, "nothing made room for it: {during:?}");
    assert_eq!(at(&during, "Options").alpha, 255, "{during:?}");

    let after = frame(&ctx, 1.6, &with_box);
    assert_eq!(at(&after, "Search inside archives").alpha, 255);
    assert!(at(&after, "Go").y - was > moved, "{after:?}");
}

/// A row is a screen of its own, laid sideways: a button that joins one opens
/// across rather than down, and the buttons after it travel to the right.
#[test]
fn a_button_added_to_a_row_opens_and_the_rest_slide_across() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(
        &ctx,
        &view(&row(&[button("Save"), button("Import")])),
    );
    let was = at(&before, "Import").x;

    let with_three = view(&row(&[button("Save"), button("Export"), button("Import")]));
    // The frame it arrives on: known of, nothing moved.
    let arrives = frame(&ctx, 1.02, &with_three);
    assert_eq!(at(&arrives, "Import").x, was, "{arrives:?}");
    assert_eq!(at(&arrives, "Save").alpha, 255, "{arrives:?}");

    let during = frame(&ctx, 1.1, &with_three);
    let new = at(&during, "Export");
    assert!(new.alpha < 255, "the button snapped in: {new:?}");
    let moved = at(&during, "Import").x - was;
    assert!(
        moved > 0.0,
        "the buttons after it did not move aside: {during:?}"
    );
    assert_eq!(at(&during, "Save").alpha, 255, "{during:?}");
    assert_eq!(at(&during, "Save").x, at(&before, "Save").x, "{during:?}");

    let after = frame(&ctx, 1.6, &with_three);
    assert_eq!(at(&after, "Export").alpha, 255);
    assert!(
        at(&after, "Import").x - was > moved,
        "it should have gone on moving: {after:?}"
    );
    // In the order the module asked for, left to right.
    assert!(at(&after, "Save").x < at(&after, "Export").x);
    assert!(at(&after, "Export").x < at(&after, "Import").x);
}
