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
    // Keyed by *where* this table is as well as what shape it is. Shape alone
    // made two same-shaped tables in one view — a sysinfo page's "System" and
    // "Operating system", both seven rows of two columns — share one id, and
    // egui reports that as an ID clash across the screen. `auto_id_with` adds the
    // widget's position in the sequence, which is stable frame to frame, so a
    // table still keeps its clock across refreshes and still restarts when its
    // shape changes.
    let id = ui.auto_id_with(("tablereveal", cols.len(), cols.first().cloned(), nrows));
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


/// Where a set of tables sharing one column layout keeps its horizontal scroll.
///
/// Every table is its own `ScrollArea`, so a screen split into one table per
/// section gets a scrollbar each — and scrolling one to reach its last column
/// used to leave the others showing their first, with identical headers no
/// longer above one another. Tables with the same columns are the same table as
/// far as a reader is concerned, so they share an offset and move together.
pub fn hscroll_key(columns: &[String]) -> egui::Id {
    egui::Id::new(("limen_table_hscroll", columns.join("\u{1}")))
}

/// The shared offset, and whether this table should be moved to it.
///
/// Only when somebody *else* moved it. Setting the offset on every frame
/// overrides the position each frame before the drag is applied, so the scroll
/// fights the hand holding it and stops short of the end — a table you cannot
/// scroll to the last column of, which is worse than one that scrolls alone.
fn shared_hscroll(ui: &egui::Ui, hkey: egui::Id, id: egui::Id) -> (f32, bool) {
    let shared: f32 = ui.data(|d| d.get_temp(hkey)).unwrap_or(0.0);
    // A table whose own rows already fit has nowhere to go. Handed somebody
    // else's offset it was shoved sideways anyway — out of its own box, over
    // whatever was beside it — and put back on the frame after, which read as
    // the short table twitching while the long one was being scrolled.
    //
    // What it could do last frame, rather than what it can do now: the answer
    // is only known after the area has been drawn.
    let could: bool = ui.data(|d| d.get_temp(id.with("hscroll_can"))).unwrap_or(true);
    // Where this table was left last frame. Absent on the first frame, when
    // there is nothing to catch up with. What is remembered is the shared value
    // this table last *tried*, not where it ended up: a table too narrow to
    // reach that place clamps to its own end, and comparing against the clamped
    // result would have it try — and fail — again on every frame.
    let tried: Option<f32> = ui.data(|d| d.get_temp(id.with("hscroll_tried")));
    let force = could && tried.is_some_and(|t| (shared - t).abs() > 0.5);
    (shared, force)
}

/// Remember where this table ended up, and tell its siblings **only** if the
/// user moved it.
///
/// The distinction is the whole of it. Tables on one screen share their columns
/// but not their widths, so a place one of them can reach is a place another
/// cannot: handed an offset past its own end, a table clamps to that end. If it
/// then reported the clamped value as the shared place, every wider table would
/// be dragged back to the narrowest one's limit — a table that scrolls, but
/// never all the way.
///
/// So a frame in which this table was *forced* says nothing: whatever it ended
/// up at is clamping, not intent. Only a table that moved on its own, from
/// where it was last frame, gets to say where everybody is.
pub fn keep_hscroll(
    ui: &egui::Ui,
    hkey: egui::Id,
    id: egui::Id,
    shared: f32,
    now: f32,
    can_scroll: bool,
    forced: bool,
) {
    let was = ui.data(|d| d.get_temp::<f32>(id.with("hscroll_at")));
    let moved = !forced && was.is_some_and(|w| (now - w).abs() > 0.5);
    ui.data_mut(|d| {
        d.insert_temp(id.with("hscroll_at"), now);
        // Whether there was anywhere to scroll to — read next frame, before
        // this table is moved to somebody else's place. See `shared_hscroll`.
        d.insert_temp(id.with("hscroll_can"), can_scroll);
        // What it last tried to reach — so a table that cannot get there does
        // not spend every frame trying again.
        d.insert_temp(id.with("hscroll_tried"), if moved { now } else { shared });
        if moved && can_scroll {
            d.insert_temp(hkey, now);
        }
    });
}

/// The colour a marked row is picked out in.
///
/// The same words a notice takes, so a module spells severity one way
/// throughout. `None` for anything else — including the empty string, which is
/// what most rows carry.
fn row_tone(level: Option<&String>) -> Option<egui::Color32> {
    match level.map(String::as_str).unwrap_or_default() {
        "error" => Some(color::ERROR),
        "warning" => Some(color::WARNING),
        "ok" | "success" => Some(color::SUCCESS),
        _ => None,
    }
}

/// How strongly a marked row's background is washed with its colour.
///
/// The whole row, not the text: a band of colour is found at a glance down a
/// list of three hundred, and it leaves the text at full contrast — tinted
/// text is both harder to read and easy to miss where it matters most, in the
/// one row that is not like the others.
const ROW_WASH: f32 = 0.16;

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
    row_levels: &[String],
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
        render_plain_table(ui, columns, rows, row_levels, ncols);
    } else {
        render_interactive_table(
            ui,
            columns,
            rows,
            row_ids,
            menu,
            row_menus,
            on_activate,
            row_levels,
            clicked,
            ncols,
        );
    }
}

/// A non-interactive table: a striped grid of labels inside a scroll area.
pub fn render_plain_table(
    ui: &mut egui::Ui,
    columns: &[String],
    rows: &[Vec<String>],
    row_levels: &[String],
    ncols: usize,
) {
    // Position as well as shape — see `row_reveal`. Two tables of the same size
    // in one view are ordinary here (one per section), and sharing a ScrollArea
    // or Grid id between them paints egui's clash warning over the page.
    let id = ui.auto_id_with(("limen_table", ncols, rows.len()));
    // So that a table which allocates nothing still advances the counter, and the
    // next one cannot land on this id.
    ui.skip_ahead_auto_ids(1);
    // Never wider than what it was given. Left to shrink-wrap its content, a
    // wide table stretches whatever holds it — and in a pop-up that means the
    // frame grows while the title bar stays at the declared width, leaving the
    // close control stranded short of the corner. A table scrolls inside its
    // container; it does not widen it.
    // Fill the width (so it scrolls rather than stretching its container), but
    // shrink to the rows vertically. `false` on the y axis takes every pixel
    // left in the view, which a table with a handful of rows spends on empty
    // space — and anything after it is pushed off the bottom.
    // Shares its horizontal place with the other tables of these columns — see
    // [`hscroll_key`].
    let hkey = hscroll_key(columns);
    let (shared_x, force) = shared_hscroll(ui, hkey, id);
    // Both axes, capped the same way as the interactive table — see
    // [`VISIBLE_ROWS`]. The row height is the body text plus the grid's own
    // vertical spacing, which is what a Grid row comes to.
    let mut area = egui::ScrollArea::horizontal()
        .id_source(id)
        .auto_shrink([false, true]);
    if force {
        area = area.scroll_offset(egui::vec2(shared_x, 0.0));
    }
    let out = area.show(ui, |ui| {
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
                    // A Grid paints no row band of its own, so there is nothing
                    // to wash here the way the interactive table does. The text
                    // carries the mark instead — second best, and the reason a
                    // table that marks rows is usually an interactive one.
                    let ink = row_tone(row_levels.get(r)).unwrap_or(color::TEXT);
                    for cell in row {
                        ui.scope(|ui| {
                            ui.set_opacity(t);
                            // Whole, like the interactive table: the cell is as
                            // wide as its text and the table scrolls for it.
                            ui.label(egui::RichText::new(cell).color(ink));
                        });
                    }
                    ui.end_row();
                }
            });
    });

    // Whether this table had anywhere to scroll to — see `keep_hscroll`.
    let can_scroll = out.content_size.x - out.inner_rect.width() > 1.0;
    keep_hscroll(ui, hkey, id, shared_x, out.state.offset.x, can_scroll, force);
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
    row_levels: &[String],
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
    // Nothing is cut unless the table would otherwise not fit: a table with
    // room to spare shows its cells whole, which is the common case and the one
    // a fixed cap got wrong — it shortened every cell on a wide screen and left
    // the space beside them empty.
    //
    // When it does not fit, only the columns past their share give way, and a
    // Every column takes the width its widest cell needs, and the table scrolls
    // sideways for whatever that comes to.
    //
    // Capping them instead — which this did — cuts the cell, and a cut cell is
    // gone: scrolling moves the window over the table, it does not redraw text
    // that was never painted. So a capped table is one you can scroll to the
    // end of and still not read, which is the worst of both. A command line is
    // 170 characters because that is how long it is; the table is as wide as
    // its contents and the scrollbar does the rest.
    let content_w =
        widths.iter().sum::<f32>() + col_gap * ncols.saturating_sub(1) as f32 + pad_x * 2.0;
    let row_h = font.size + pad_y * 2.0;
    let rounding = egui::Rounding::ZERO;

    let id = ui.auto_id_with(("limen_itable", ncols, rows.len()));
    ui.skip_ahead_auto_ids(1);
    // Shared with every other table on the screen that has these columns, so
    // they scroll as one. Applied from last frame's value and written back
    // after, which is a frame behind — invisible at any scrolling speed a hand
    // produces, and the alternative is each table keeping its own place.
    let hkey = hscroll_key(columns);
    let (shared_x, force) = shared_hscroll(ui, hkey, id);
    // Both axes: sideways for columns that do not fit, and downwards once
    // there are more rows than `VISIBLE_ROWS`. `auto_shrink` on y so a short
    // table still shrink-wraps its rows and shows no scrollbar.
    let mut area = egui::ScrollArea::horizontal()
        .id_source(id)
        .auto_shrink([false, true]);
    if force {
        area = area.scroll_offset(egui::vec2(shared_x, 0.0));
    }
    let out = area.show(ui, |ui| {
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
            // A marked row is washed in its colour, over the zebra and under
            // the hover highlight — so it still lights up when the pointer is
            // on it, and still reads as marked when it is not.
            if let Some(tone) = row_tone(row_levels.get(r)) {
                ui.painter()
                    .rect_filled(rect, rounding, with_alpha(tone, ROW_WASH));
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

            // The text stays the ordinary colour: the row is picked out by its
            // background, which costs the words none of their contrast.
            let ink = color::TEXT;
            let mut x = rect.left() + pad_x;
            for (c, cell) in row.iter().enumerate() {
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    cell,
                    font.clone(),
                    ink,
                );
                x += widths.get(c).copied().unwrap_or(0.0) + col_gap;
            }

            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let row_id = row_ids.get(r).cloned().unwrap_or_default();
            // A non-empty per-row menu overrides the shared one for this row.
            if !menu_for(menu, row_menus, r).is_empty() && resp.secondary_clicked() {
                open_row_menu(ui, id, r);
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

        // The menu stands over the rows, so it is drawn after them — and it
        // outlives the right-click that opened it, being drawn on every frame
        // until it has finished animating away. See [`row_menu_layer`].
        row_menu_layer(ui, id, menu, row_menus, row_ids, clicked);
    });

    // Whether this table had anywhere to scroll to — see `keep_hscroll`.
    let can_scroll = out.content_size.x - out.inner_rect.width() > 1.0;
    keep_hscroll(ui, hkey, id, shared_x, out.state.offset.x, can_scroll, force);
}
