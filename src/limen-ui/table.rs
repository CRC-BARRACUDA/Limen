//! Tables, plain and interactive.

use crate::*;

/// When this table's entrance started, and how far row `r` is into it.
///
/// The clock lives in egui memory keyed by the table's shape, so a table whose
/// row count changes (results arriving, a page turned) is a *new* id and
/// restarts — while a view that merely refreshes in place, like a progress list
/// ticking through its steps, keeps its clock and does not re-animate.
///
/// The stagger index is capped: a 500-row table staggered per row would take
/// half a minute to finish appearing. Past the cap every remaining row shares
/// the last slot, so it reads as a cascade without ever outstaying it.
pub fn row_reveal(ui: &mut egui::Ui, cols: &[String], nrows: usize, r: usize) -> f32 {
    const CAP: usize = 18;
    let id = egui::Id::new(("tablereveal", cols.len(), cols.first().cloned(), nrows));
    let now = ui.input(|i| i.time);
    // When this table's shape was first seen...
    let shape = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    // ...and when the view around it last began an entrance. The later of the
    // two wins, so the table replays both when its contents change *and* when
    // you switch back to a tab that was already open — but sits still while a
    // view merely refreshes in place.
    let view = ui
        .data(|d| d.get_temp::<f64>(view_reveal_id()))
        .unwrap_or(0.0);
    reveal_t(ui, r.min(CAP), shape.max(view), now, 0.010, 0.14)
}

/// Render a [`Widget::Table`]. A plain data table is a striped grid; when the
/// module attaches a `menu` or `on_activate`, rows become interactive — the
/// whole row (full width, not just its text) is clickable, highlights on hover,
/// and emits an [`Invoke`] carrying that row's id on right-click / double-click.
#[allow(clippy::too_many_arguments)]
pub fn render_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_ids: &[String],
    menu: &[MenuItem],
    row_menus: &[Vec<MenuItem>],
    on_activate: Option<&RowAction>,
    clicked: &mut Option<Invoke>,
) {
    let ncols = columns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if ncols == 0 {
        return;
    }
    let has_row_menus = row_menus.iter().any(|m| !m.is_empty());
    if menu.is_empty() && !has_row_menus && on_activate.is_none() {
        render_plain_table(ui, columns, rows, ncols);
    } else {
        render_interactive_table(
            ui,
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate,
            clicked,
            ncols,
        );
    }
}

/// A non-interactive table: a striped grid of labels inside a scroll area.
pub fn render_plain_table(ui: &mut egui::Ui, columns: &[String], rows: &[Vec<String>], ncols: usize) {
    let id = ui.make_persistent_id(("limen_table", ncols, rows.len()));
    // Never wider than what it was given. Left to shrink-wrap its content, a
    // wide table stretches whatever holds it — and in a pop-up that means the
    // frame grows while the title bar stays at the declared width, leaving the
    // close control stranded short of the corner. A table scrolls inside its
    // container; it does not widen it.
    egui::ScrollArea::horizontal()
        .id_source(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
        egui::Grid::new(id)
            .striped(true)
            .num_columns(ncols)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for c in columns {
                    ui.label(egui::RichText::new(c).strong());
                }
                ui.end_row();
                for (r, row) in rows.iter().enumerate() {
                    let t = row_reveal(ui, columns, rows.len(), r);
                    for cell in row {
                        ui.scope(|ui| {
                            ui.set_opacity(t);
                            ui.label(cell);
                        });
                    }
                    ui.end_row();
                }
            });
    });
}

/// An interactive table: each row is one full-width clickable band that zebra-
/// stripes, highlights (with an accent edge) on hover, and dims slightly on
/// press — so it reads as clickable — carrying the row context menu + double
/// click. Laid out manually (not a `Grid`) so a row spans the whole width.
#[allow(clippy::too_many_arguments)]
pub fn render_interactive_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_ids: &[String],
    menu: &[MenuItem],
    row_menus: &[Vec<MenuItem>],
    on_activate: Option<&RowAction>,
    clicked: &mut Option<Invoke>,
    ncols: usize,
) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let col_gap = 18.0;
    let pad_x = 8.0;
    let pad_y = 5.0;

    // Column widths = the widest of the header and every cell in that column.
    let measure = |ui: &egui::Ui, s: &str| -> f32 {
        ui.fonts(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT)
                .size()
                .x
        })
    };
    let mut widths = vec![0f32; ncols];
    for (c, col) in columns.iter().enumerate() {
        widths[c] = widths[c].max(measure(ui, col));
    }
    for row in rows {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(measure(ui, cell));
        }
    }
    let content_w =
        widths.iter().sum::<f32>() + col_gap * ncols.saturating_sub(1) as f32 + pad_x * 2.0;
    let row_h = font.size + pad_y * 2.0;
    let rounding = egui::Rounding::ZERO;

    let id = ui.make_persistent_id(("limen_itable", ncols, rows.len()));
    egui::ScrollArea::horizontal().id_source(id).show(ui, |ui| {
        // Stretch to the viewport so a row's highlight spans the full width.
        let table_w = content_w.max(ui.available_width());

        // Header.
        let (hrect, _) = ui.allocate_exact_size(egui::vec2(table_w, row_h), egui::Sense::hover());
        let mut hx = hrect.left() + pad_x;
        for (c, col) in columns.iter().enumerate() {
            ui.painter().text(
                egui::pos2(hx, hrect.center().y),
                egui::Align2::LEFT_CENTER,
                col,
                font.clone(),
                color::TEXT_MUTED,
            );
            hx += widths.get(c).copied().unwrap_or(0.0) + col_gap;
        }
        ui.painter().hline(
            hrect.x_range(),
            hrect.bottom(),
            egui::Stroke::new(1.0_f32, color::BORDER),
        );

        // Rows.
        for (r, row) in rows.iter().enumerate() {
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(table_w, row_h), egui::Sense::click());
            let it = interact(ui, &resp);
            // Fade this row in on its slot of the table's entrance. Set on the
            // Ui so the painted chrome (zebra, hover, separators) and the cell
            // text below both pick it up.
            let row_t = row_reveal(ui, columns, rows.len(), r);
            ui.set_opacity(row_t);

            // Zebra base, then a hover highlight faded in by the eased factor,
            // with a small accent bar on the leading edge and a press tint.
            if r % 2 == 1 {
                ui.painter()
                    .rect_filled(rect, rounding, with_alpha(color::BG_WIDGET, 0.30));
            }
            if it.hover > 0.0 {
                let tint = lerp_color(color::BG_HOVER, color::ACCENT, it.press * 0.18);
                ui.painter()
                    .rect_filled(rect, rounding, with_alpha(tint, it.hover));
                let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2.5, rect.height()));
                ui.painter().rect_filled(
                    bar,
                    egui::Rounding::ZERO,
                    with_alpha(color::ACCENT, it.hover),
                );
            }

            // Cells.
            let mut x = rect.left() + pad_x;
            for (c, cell) in row.iter().enumerate() {
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    cell,
                    font.clone(),
                    color::TEXT,
                );
                x += widths.get(c).copied().unwrap_or(0.0) + col_gap;
            }

            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let row_id = row_ids.get(r).cloned().unwrap_or_default();
            // A non-empty per-row menu overrides the shared one for this row.
            let this_menu: &[MenuItem] = match row_menus.get(r) {
                Some(m) if !m.is_empty() => m,
                _ => menu,
            };
            if !this_menu.is_empty() {
                let mut picked: Option<Invoke> = None;
                resp.context_menu(|ui| render_row_menu(ui, this_menu, &row_id, &mut picked));
                if picked.is_some() {
                    *clicked = picked;
                }
            }
            if let Some(act) = on_activate
                && resp.double_clicked()
            {
                let mut args = serde_json::Map::new();
                args.insert("id".into(), Value::String(row_id.clone()));
                *clicked = Some(Invoke {
                    dismiss: false,
                    // A double-click is not a menu entry; nothing to ask.
                    confirm: None,
                    action: act.action.clone(),
                    args,
                    open_in_tab: act.open_in_tab,
                });
            }
        }
    });
}

/// Build a table row's right-click menu, recursing into submenus. Writes the
/// chosen [`Invoke`] (with the row's `id` merged in) into `out`.
pub fn render_row_menu(ui: &mut egui::Ui, items: &[MenuItem], row_id: &str, out: &mut Option<Invoke>) {
    for item in items {
        if !item.children.is_empty() {
            let mut sub: Option<Invoke> = None;
            ui.menu_button(&item.label, |ui| {
                render_row_menu(ui, &item.children, row_id, &mut sub)
            });
            if sub.is_some() {
                *out = sub;
                ui.close_menu();
            }
        } else if let Some(action) = &item.action {
            if ui.button(&item.label).clicked() {
                let mut args = item.args.clone();
                args.insert("id".into(), Value::String(row_id.to_string()));
                *out = Some(Invoke {
                    dismiss: false,
                    // A menu entry can carry a question just as a button can.
                    confirm: item.confirm.clone(),
                    action: action.clone(),
                    args,
                    open_in_tab: item.open_in_tab,
                });
                ui.close_menu();
            }
        } else {
            ui.label(&item.label);
        }
    }
}
