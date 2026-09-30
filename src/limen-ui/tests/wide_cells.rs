//! A table wider than its window, and the tables that scroll with it.
use eframe::egui;

use limen_ui::*;

/// Tables with the same columns share a horizontal place; tables with
/// different columns keep their own.
///
/// This is what stops a screen split into one table per section from giving
/// each of them its own scroll position, with identical headers no longer
/// above one another.
#[test]
fn tables_of_the_same_columns_share_one_horizontal_place() {
    let a = vec![
        "Source".to_string(),
        "Name".to_string(),
        "Command".to_string(),
    ];
    let b = a.clone();
    let c = vec!["Source".to_string(), "Name".to_string()];

    assert_eq!(hscroll_key(&a), hscroll_key(&b), "same columns, same place");
    assert_ne!(hscroll_key(&a), hscroll_key(&c), "different columns, its own");
}

/// The key is the columns, not their concatenation: two layouts that would run
/// together into the same string must not collide.
#[test]
fn the_column_names_cannot_run_together_into_one_key() {
    let split = vec!["ab".to_string(), "c".to_string()];
    let joined = vec!["a".to_string(), "bc".to_string()];
    assert_ne!(hscroll_key(&split), hscroll_key(&joined));
}

/// A table that cannot reach the shared place must not drag everybody back to
/// its own end.
///
/// Tables on one screen share their columns but not their widths. Handed an
/// offset past its own end, a narrow table clamps — and if it reports that as
/// the shared place, every wider table snaps back to the narrowest one's limit.
/// That is a table which scrolls, but never all the way.
#[test]
fn a_table_that_cannot_reach_the_shared_place_does_not_drag_the_others_back() {
    let ctx = egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let shared = hscroll_key(&["A".to_string(), "B".to_string()]);
            let wide = egui::Id::new("wide");
            let narrow = egui::Id::new("narrow");
            let at = |id: egui::Id| ui.data(|d| d.get_temp::<f32>(shared)).map(|s| (s, id));

            // Both settle at the left on the first frame. Nothing moved, so
            // nothing is claimed — the shared place is not written at all until
            // somebody actually scrolls.
            keep_hscroll(ui, shared, wide, 0.0, 0.0, true, false);
            keep_hscroll(ui, shared, narrow, 0.0, 0.0, true, false);
            assert_eq!(ui.data(|d| d.get_temp::<f32>(shared)), None);

            // The user scrolls the wide one to 200. That is intent, and it
            // carries.
            keep_hscroll(ui, shared, wide, 0.0, 200.0, true, false);
            assert_eq!(ui.data(|d| d.get_temp::<f32>(shared)), Some(200.0));

            // The narrow one is forced there and clamps to 80 — its own end. It
            // *can* scroll, just not that far, so the old rule let it through.
            keep_hscroll(ui, shared, narrow, 200.0, 80.0, true, true);
            assert_eq!(
                ui.data(|d| d.get_temp::<f32>(shared)),
                Some(200.0),
                "a table clamped at its own end moved everybody back to it"
            );

            // And on the settled frame after — no longer forced, no input — it
            // still says nothing, rather than reporting 80 as a fresh move.
            keep_hscroll(ui, shared, narrow, 200.0, 80.0, true, false);
            assert_eq!(
                ui.data(|d| d.get_temp::<f32>(shared)),
                Some(200.0),
                "the clamped table reported its position as a movement"
            );
            assert!(at(wide).is_some());

            // The user scrolling the narrow one itself is still intent.
            keep_hscroll(ui, shared, narrow, 200.0, 40.0, true, false);
            assert_eq!(ui.data(|d| d.get_temp::<f32>(shared)), Some(40.0));
        });
    });
}
