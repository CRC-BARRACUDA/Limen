//! Scrolling a table that is wider than its window.

use eframe::egui;
use limen_ui::*;

/// The whole of the command in the sample row, as it should be painted.
const FULL_COMMAND: &str = concat!(
    r#""C:\Program Files (x86)\Microsoft\Edge\Application\154.0.4258.37"#,
    r#"\Installer\setup.exe" --configure-user-settings --verbose-logging"#,
    r#" --system-level --msedge"#
);

fn columns() -> Vec<String> {
    [
        "Source", "Name", "Command", "Publisher", "Scope", "Enabled", "Location",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// An autoruns-shaped table: seven columns, one of them a real command line.
fn draw(ui: &mut egui::Ui) {
    let columns = columns();
    let rows: Vec<Vec<String>> = (0..40)
        .map(|i| {
            vec![
                "active-setup".to_string(),
                format!("Microsoft Edge {i}"),
                FULL_COMMAND.to_string(),
                "(Verified) Microsoft Corporation".to_string(),
                "system".to_string(),
                "yes".to_string(),
                r"HKLM\Software\Microsoft\Active Setup\Installed Components\{9459C573-B17A}"
                    .to_string(),
            ]
        })
        .collect();
    let ids: Vec<String> = (0..rows.len()).map(|i| i.to_string()).collect();
    let menu: Vec<MenuItem> =
        serde_json::from_str(r#"[{"label":"About","action":{"capability":"a","method":"about"}}]"#)
            .expect("menu");
    let mut clicked = None;
    render_table(ui, &columns, &rows, &ids, &menu, &[], None, &[], &mut clicked);
}

/// The pointer over the table, and optionally a shift+wheel turn.
fn events(scroll: bool) -> Vec<egui::Event> {
    let mut e = vec![egui::Event::PointerMoved(egui::pos2(600.0, 300.0))];
    if scroll {
        e.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            modifiers: egui::Modifiers {
                shift: true,
                ..Default::default()
            },
        });
    }
    e
}

/// One frame, inside a page that scrolls vertically the way the module page
/// does. Returns the shared horizontal offset and everything painted.
fn frame(ctx: &egui::Context, events: Vec<egui::Event>) -> (f32, Vec<String>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1270.0, 560.0),
        )),
        events,
        ..Default::default()
    };
    let mut offset = 0.0;
    let out = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw(ui);
                    offset = ui
                        .data(|d| d.get_temp::<f32>(hscroll_key(&columns())))
                        .unwrap_or(0.0);
                });
        });
    });

    fn painted(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| painted(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in out.shapes.iter() {
        painted(&shape.shape, &mut texts);
    }
    (offset, texts)
}

/// Shift and the wheel move a table that is wider than its window.
///
/// The tables on one screen share a horizontal place, and getting that wrong
/// meant a table that snapped back every frame — so this drives the real event
/// rather than trusting the arithmetic.
#[test]
fn shift_and_the_wheel_scroll_a_wide_table_sideways() {
    let ctx = egui::Context::default();
    frame(&ctx, events(false));

    let mut offset = 0.0;
    for _ in 0..8 {
        offset = frame(&ctx, events(true)).0;
    }
    assert!(
        offset > 1.0,
        "shift+wheel left the table at {offset} — it did not scroll sideways"
    );
}

/// Every cell is painted whole, and the scroll reaches the end of it.
///
/// Columns used to be capped, which cut the cell — and a cut cell is gone:
/// scrolling moves the window over the table, it does not redraw text that was
/// never painted. A table you can scroll to the end of and still not read is
/// the worst of both, so a column takes the width its contents need.
#[test]
fn the_whole_of_a_long_cell_is_painted() {
    let ctx = egui::Context::default();
    frame(&ctx, events(false));
    let mut texts = Vec::new();
    for _ in 0..40 {
        texts = frame(&ctx, events(true)).1;
    }

    assert!(
        texts.iter().any(|t| t == FULL_COMMAND),
        "the command was not painted whole — the longest drawn was {:?}",
        texts.iter().max_by_key(|t| t.len())
    );
    assert!(
        !texts.iter().any(|t| t.ends_with('…')),
        "something was still cut short: {:?}",
        texts.iter().find(|t| t.ends_with('…'))
    );
}

/// The table holds up at a UI scale other than 1.
///
/// The app applies its scale with `set_zoom_factor`, which changes how many
/// points a window is worth. A table measured against anything but the room it
/// actually has — a fixed pixel figure, a width taken before the zoom — comes
/// out too wide or too narrow at every scale but the default.
#[test]
fn a_zoomed_window_still_scrolls_to_the_end() {
    for zoom in [0.8_f32, 1.0, 1.25, 1.6] {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(zoom);
        frame(&ctx, events(false));

        let mut offset = 0.0;
        let mut texts = Vec::new();
        for _ in 0..60 {
            let (o, t) = frame(&ctx, events(true));
            offset = o;
            texts = t;
        }
        // Wider than the window at every scale this is tried at, so there is
        // always somewhere to go — and it goes there.
        assert!(
            offset > 1.0,
            "at zoom {zoom} the table did not scroll (offset {offset})"
        );
        // And the cells are whole whatever the scale: nothing is cut to a size
        // that only suits one zoom level.
        assert!(
            texts.iter().any(|t| t == FULL_COMMAND),
            "at zoom {zoom} the command was not painted whole"
        );
    }
}
