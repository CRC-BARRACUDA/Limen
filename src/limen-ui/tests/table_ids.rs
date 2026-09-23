//! Two tables of the same shape in one view must not collide.

use eframe::egui;
use limen_ui as ui;

/// Every piece of text a frame painted.
fn painted(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
        egui::Shape::Vec(v) => v.iter().for_each(|s| painted(s, out)),
        _ => {}
    }
}

/// Draw `n` tables of identical shape in one view and return what was painted.
///
/// egui does not return ID clashes as errors — it paints them over the offending
/// widgets, which is why they are read back out of the frame here.
fn draw(n: usize, interactive: bool) -> Vec<String> {
    let ctx = egui::Context::default();
    ui::apply_theme(&ctx);
    let columns: Vec<String> = vec!["Item".into(), "Value".into()];
    let rows: Vec<Vec<String>> = (0..7)
        .map(|i| vec![format!("item {i}"), format!("value {i}")])
        .collect();
    let mut text = Vec::new();
    // Several frames, and the clock advanced between them: a clash is only
    // reported once both uses have been seen, and the rows fade in on a stagger
    // — at time zero only the first of each table has arrived, and a test that
    // read that frame would be asserting about an animation rather than a table.
    for t in [0.0_f64, 5.0, 10.0] {
        let input = egui::RawInput {
            time: Some(t),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 1400.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for s in 0..n {
                        ui.label(format!("section {s}"));
                        if interactive {
                            let mut clicked = None;
                            // Interactive needs row ids to be interactive at
                            // all; the menus and the activate action are what a
                            // read-only table leaves out.
                            let ids: Vec<String> =
                                (0..rows.len()).map(|r| format!("{s}:{r}")).collect();
                            ui::render_table(
                                ui, &columns, &rows, &ids, &[], &[], None, &mut clicked,
                            );
                        } else {
                            ui::render_plain_table(ui, &columns, &rows, 2);
                        }
                    }
                });
            });
        });
        text.clear();
        for c in &out.shapes {
            painted(&c.shape, &mut text);
        }
    }
    text
}

fn clashes(text: &[String]) -> Vec<&String> {
    // egui's wording for a duplicate id, whichever widget reports it.
    text.iter()
        .filter(|t| t.contains("First use of") || t.contains("Second use of"))
        .collect()
}

/// A sysinfo page is one table per section, and several sections have the same
/// number of rows. Keyed by shape alone, those shared a ScrollArea and a Grid id
/// and egui painted its clash warning across the page.
#[test]
fn several_plain_tables_of_one_shape_do_not_clash() {
    let text = draw(4, false);
    let bad = clashes(&text);
    assert!(bad.is_empty(), "ID clash reported: {bad:?}");
    // And all four actually rendered — a test that silences the warning by
    // drawing nothing would pass too.
    let cells = text.iter().filter(|t| t.as_str() == "value 6").count();
    assert_eq!(cells, 4, "expected four tables, found {cells}");
}

/// The interactive table derives its id the same way, so it has the same trap.
#[test]
fn several_interactive_tables_of_one_shape_do_not_clash() {
    let text = draw(3, true);
    let bad = clashes(&text);
    assert!(bad.is_empty(), "ID clash reported: {bad:?}");
}

/// One table on its own must still be fine — the guard against a fix that makes
/// every id unique per frame and so loses state between frames.
#[test]
fn a_single_table_is_unaffected() {
    assert!(clashes(&draw(1, false)).is_empty());
}
