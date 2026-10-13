//! A table that gains or loses a row shows which row it was.
//!
//! The module answers with a whole view each time, so the table is handed a new
//! list of rows and has to work out the difference itself. What is measured here
//! is what somebody actually sees: where each row is painted, and how solid it
//! is while it gets there.

use eframe::egui;
use limen_ui as ui;

/// One painted string: what it says, where, and how far through a fade.
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
            // The opacity a `Ui` was set to is folded into the painted colour,
            // which is why this is readable at all from outside.
            alpha: t.fallback_color.a(),
        }),
        egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

/// A table, drawn at `t` seconds with these rows, and everything it painted.
fn frame(ctx: &egui::Context, t: f64, rows: &[Vec<String>], ids: &[String]) -> Vec<Ink> {
    let columns: Vec<String> = vec!["Path".into()];
    let input = egui::RawInput {
        time: Some(t),
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        ..Default::default()
    };
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut clicked = None;
            // Row ids and a row menu: the interactive table is the one a module
            // gets when its rows are things rather than readings.
            let menu = vec![ui::MenuItem {
                label: "Remove".into(),
                action: Some(ui::Action {
                    capability: "x".into(),
                    method: "drop".into(),
                }),
                args: Default::default(),
                open_in_tab: false,
                confirm: None,
                children: Vec::new(),
            }];
            ui::render_table(
                ui, &columns, rows, ids, &menu, &[], None, &[], &mut clicked,
            );
        });
    });
    let mut ink = Vec::new();
    for c in &out.shapes {
        collect(&c.shape, &mut ink);
    }
    ink
}

fn find(ink: &[Ink], text: &str) -> Option<Ink> {
    ink.iter().find(|i| i.text == text).cloned()
}

fn at(ink: &[Ink], text: &str) -> Ink {
    find(ink, text).unwrap_or_else(|| panic!("{text} was not painted: {ink:?}"))
}

fn rows_of(names: &[&str]) -> (Vec<Vec<String>>, Vec<String>) {
    (
        names.iter().map(|n| vec![n.to_string()]).collect(),
        names.iter().map(|n| n.to_string()).collect(),
    )
}

/// Let the table arrive and settle, so what follows is measured against a table
/// standing still rather than against an entrance.
fn settled(ctx: &egui::Context, names: &[&str]) -> Vec<Ink> {
    let (rows, ids) = rows_of(names);
    let mut last = Vec::new();
    for t in [0.0, 0.3, 0.6, 1.0] {
        last = frame(ctx, t, &rows, &ids);
    }
    last
}

/// One row joining the list is the only thing that moves on its own account.
/// Everything above it stays exactly where it was, at full strength — the whole
/// difference between an edit and a page that has reloaded.
#[test]
fn an_added_row_arrives_while_the_rest_hold_still() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(&ctx, &["/srv/data", "/var/log"]);
    let was = at(&before, "/var/log");

    let (rows, ids) = rows_of(&["/srv/data", "/var/log", "/home/me"]);
    // The frame the new row arrives on: it is at nothing, having only just been
    // heard of, and nothing else has stirred.
    let arrives = frame(&ctx, 1.02, &rows, &ids);
    assert_eq!(at(&arrives, "/srv/data").alpha, 255, "{arrives:?}");
    assert_eq!(at(&arrives, "/var/log").alpha, 255, "{arrives:?}");
    assert_eq!(at(&arrives, "/var/log").y, was.y, "{arrives:?}");

    let during = frame(&ctx, 1.1, &rows, &ids);
    // The rows that were already there have not re-animated.
    assert_eq!(at(&during, "/srv/data").alpha, 255, "{during:?}");
    assert_eq!(at(&during, "/var/log").alpha, 255, "{during:?}");
    // And they are still where they were: a row added at the end moves nothing.
    assert_eq!(at(&during, "/var/log").y, was.y);

    // The new one is on its way in: not yet solid, and still short of its place.
    let new = at(&during, "/home/me");
    assert!(new.alpha < 200, "the new row arrived instantly: {new:?}");
    assert!(
        new.x < at(&during, "/var/log").x,
        "the new row should slide in from the side: {new:?}"
    );

    // And once it has arrived it is an ordinary row.
    let after = frame(&ctx, 1.6, &rows, &ids);
    let new = at(&after, "/home/me");
    assert_eq!(new.alpha, 255);
    assert_eq!(new.x, at(&after, "/var/log").x);
    assert!(new.y > was.y, "it sits below the row it followed: {new:?}");
}

/// A row added in the middle pushes what is below it down — gradually, by as
/// much as it has itself grown, rather than in one jump.
#[test]
fn the_rows_below_move_by_as_much_as_the_new_one_has_grown() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(&ctx, &["/a", "/c"]);
    let pitch_was = at(&before, "/c").y - at(&before, "/a").y;

    let (rows, ids) = rows_of(&["/a", "/b", "/c"]);
    frame(&ctx, 1.02, &rows, &ids);
    let during = frame(&ctx, 1.1, &rows, &ids);
    let moved = at(&during, "/c").y - at(&before, "/c").y;
    assert!(
        moved > 0.0 && moved < pitch_was,
        "a row below the insertion jumped instead of moving: moved {moved}, a row is {pitch_was}"
    );

    // By the time the arrival is over it has moved exactly one row.
    let after = frame(&ctx, 1.6, &rows, &ids);
    let moved = at(&after, "/c").y - at(&before, "/c").y;
    assert!(
        (moved - pitch_was).abs() < 0.5,
        "expected it to end one row down, moved {moved} of {pitch_was}"
    );
}

/// A row that has gone collapses, so the gap closes instead of the list
/// snapping shut.
#[test]
fn a_removed_row_collapses_and_the_gap_closes_after_it() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let before = settled(&ctx, &["/a", "/b", "/c"]);
    let pitch = at(&before, "/b").y - at(&before, "/a").y;

    let (rows, ids) = rows_of(&["/a", "/c"]);
    frame(&ctx, 1.02, &rows, &ids);
    let during = frame(&ctx, 1.07, &rows, &ids);
    let ghost = at(&during, "/b");
    assert!(ghost.alpha < 255, "it should be fading out: {ghost:?}");
    let closed = at(&before, "/c").y - at(&during, "/c").y;
    assert!(
        closed >= 0.0 && closed < pitch,
        "the gap snapped shut instead of closing: {closed} of {pitch}"
    );

    // And when it is over, the row is gone and the gap with it.
    let after = frame(&ctx, 1.4, &rows, &ids);
    assert!(find(&after, "/b").is_none(), "the ghost outstayed: {after:?}");
    let closed = at(&before, "/c").y - at(&after, "/c").y;
    assert!(
        (closed - pitch).abs() < 0.5,
        "expected the gap closed by one row, closed {closed} of {pitch}"
    );
}

/// A row whose id stays the same while its text changes has not arrived: it is
/// the same row, saying something else.
#[test]
fn a_row_that_only_changes_what_it_says_does_not_re_arrive() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    settled(&ctx, &["/a", "/b"]);

    let rows = vec![vec!["/a".to_string()], vec!["/b (changed)".to_string()]];
    let ids = vec!["/a".to_string(), "/b".to_string()];
    let during = frame(&ctx, 1.02, &rows, &ids);
    let changed = at(&during, "/b (changed)");
    assert_eq!(changed.alpha, 255, "it faded in as if it were new: {changed:?}");
}

/// A table replaced wholesale is a different table, and plays its own entrance
/// rather than a dozen separate arrivals.
#[test]
fn replacing_every_row_replays_the_tables_own_entrance() {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    settled(&ctx, &["/a", "/b", "/c"]);

    let names: Vec<String> = (0..20).map(|i| format!("/new/{i}")).collect();
    let rows: Vec<Vec<String>> = names.iter().map(|n| vec![n.clone()]).collect();
    frame(&ctx, 1.02, &rows, &names);
    let during = frame(&ctx, 1.1, &rows, &names);

    // The entrance is a cascade: the first rows are on their way in and the
    // last have not started. Twenty rows each sliding in on their own clock
    // would have them all at the same place in the motion instead.
    assert!(at(&during, "/new/0").alpha > 0, "{during:?}");
    assert!(
        find(&during, "/new/19").is_none(),
        "the whole table arrived at once: {during:?}"
    );
}

