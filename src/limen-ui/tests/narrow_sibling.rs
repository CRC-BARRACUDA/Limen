//! A table that has nothing to scroll, beside one that has.
//!
//! Two tables of the same columns share a horizontal place, so reaching the last
//! column of one brings the other with it. But a table whose own content already
//! fits has nowhere to go: handed somebody else's offset, it was shoved sideways
//! out of its own box — and put back on the frame after, which is why it read as
//! the bottom half of the screen twitching while the top half scrolled.

use eframe::egui;
use limen_ui::*;

fn columns() -> Vec<String> {
    ["Категорія", "Тип", "Ідентифікатор", "Виробник", "Модель", "Серійний номер"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// The wide one: long values, far more than fits.
fn wide_rows() -> Vec<Vec<String>> {
    (0..8)
        .map(|i| {
            vec![
                "usb".into(),
                "storage".into(),
                format!("{}{i}", "0000:c4:00.3 Linux 7.2.4-200.fc44.x86_64 xhci-hcd ".repeat(3)),
                "Linux 7.2.4-200.fc44.x86_64 xhci-hcd".into(),
                "xHCI Host Controller".into(),
                format!("{}{i}", "0101a53c7fcecdb3a14cdf8b9b2a83b00d6fec35aa9693c8e45fd".repeat(2)),
            ]
        })
        .collect()
}

/// The narrow one: three short rows, which fit with room to spare.
fn narrow_rows() -> Vec<Vec<String>> {
    vec![
        vec!["usb".into(), "usb".into(), "18d1:4ee7".into(), "Google".into(), "Pixel 7".into(), "29201FDH200GB5".into()],
        vec!["net".into(), "virtual".into(), "c2:1b:ed:e8:4d:e8".into(), String::new(), "docker0".into(), "down".into()],
        vec!["net".into(), "virtual".into(), "fe:54:00:7e:4f:b6".into(), String::new(), "vnet0".into(), "unknown".into()],
    ]
}

/// Draw both, as one screen does, and hand back where the narrow one's first
/// cell was painted.
fn frame(ctx: &egui::Context, events: Vec<egui::Event>, time: f64) -> Option<f32> {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 800.0));
    let out = ctx.run(
        egui::RawInput { screen_rect: Some(screen), events, time: Some(time), ..Default::default() },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // A pop-up's pinned width: both tables are wider than this, but
                // one of them by a few hundred pixels and the other by a few
                // thousand.
                ui.set_max_width(640.0);
                let (cols, mut clicked) = (columns(), None);
                render_table(ui, &cols, &wide_rows(), &[], &[], &[], None, &[], &mut clicked);
                ui.separator();
                render_table(ui, &cols, &narrow_rows(), &[], &[], &[], None, &[], &mut clicked);
            });
        },
    );
    // "18d1:4ee7" belongs only to the narrow table's first row.
    out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == "18d1:4ee7" => Some(t.pos.x),
        _ => None,
    })
}

/// Scrolling the wide table does not drag the narrow one out of its box.
#[test]
fn a_table_with_nothing_to_scroll_stays_where_it_is() {
    let ctx = egui::Context::default();
    limen_ui::apply_theme(&ctx);
    let mut at_rest = None;
    for (n, time) in [1.0_f64, 1.2, 1.4].into_iter().enumerate() {
        at_rest = frame(&ctx, Vec::new(), time).or(at_rest);
        let _ = n;
    }
    let at_rest = at_rest.expect("the narrow table's first id cell");

    // Shift+wheel over the wide table, which is what moves the shared place.
    let scrolling = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        modifiers: egui::Modifiers { shift: true, ..Default::default() },
    };
    let mut worst = 0.0_f32;
    for (n, time) in [1.6_f64, 1.8, 2.0, 2.2].into_iter().enumerate() {
        let events = vec![
            egui::Event::PointerMoved(egui::pos2(500.0, 120.0)),
            scrolling.clone(),
        ];
        let seen = frame(&ctx, if n < 2 { events } else { Vec::new() }, time);
        // Gone altogether is the same bug seen further along: shoved so far
        // that egui drops the cell rather than drawing it.
        let x = seen.unwrap_or_else(|| {
            panic!("frame {n}: the narrow table's row was not drawn at all")
        });
        worst = worst.max((x - at_rest).abs());
    }
    println!("at rest x={at_rest:.0}; worst shift while the sibling scrolled: {worst:.0}px");
    assert!(
        worst < 1.0,
        "the narrow table moved {worst:.0}px while its wide sibling was scrolled"
    );
}
