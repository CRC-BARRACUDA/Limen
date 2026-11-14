//! Two file panes side by side, as Midnight Commander has them: one place on
//! each side, a cursor in the active one, marks, and the keys an MC user's
//! fingers already know.
//!
//! The module owns what is in each pane — its title, its path and its entries —
//! and does the work; Limen owns the cursor, the marks and which pane is
//! active, so moving about never waits on the module. What is asked of the
//! module goes through `on_action` as `{op, pane, path, names}`: `open` a
//! folder, go `up`, `refresh`, `copy` to the other pane, `delete`. Copy and
//! delete are asked about first, in Limen's own confirmation window, naming
//! every file.
//!
//! Keys, while the commander has the keyboard: ↑ ↓ PgUp PgDn Home End move;
//! Enter opens a folder (or `..`); Backspace goes up; Tab switches pane; Insert
//! or Space marks; F5 copies; F8 or Delete deletes; Ctrl+R refreshes.

use crate::*;

/// What Limen keeps between frames: which pane is active, each pane's cursor
/// and marks, and the path each was at — a new path starts at the top, unmarked.
#[derive(Clone, Default)]
struct State {
    active: usize,
    cursor: [usize; 2],
    marked: [Vec<String>; 2],
    path: [String; 2],
    seen: bool,
}

const ROW_H: f32 = 22.0;
const SIZE_W: f32 = 96.0;
const DATE_W: f32 = 138.0;
const GAP: f32 = 10.0;

fn rows_of(pane: &CommanderPane) -> Vec<Option<&CommanderEntry>> {
    let mut rows: Vec<Option<&CommanderEntry>> = Vec::new();
    if pane.up {
        rows.push(None); // `..`
    }
    rows.extend(pane.entries.iter().map(Some));
    rows
}

/// Draw a [`Widget::Commander`]. Anything else is ignored.
pub(crate) fn render(ui: &mut egui::Ui, widget: &Widget, clicked: &mut Option<Invoke>) {
    let Widget::Commander {
        id,
        left,
        right,
        on_action,
        status,
        rows,
        fill,
    } = widget
    else {
        return;
    };
    let ctx = ui.ctx().clone();
    let key = egui::Id::new(("limen_commander", id.as_str()));
    let focus_id = key.with("focus");
    let mut st: State = ctx.data(|d| d.get_temp(key)).unwrap_or_default();
    let panes = [left, right];

    for (i, pane) in panes.iter().enumerate() {
        if st.path[i] != pane.path {
            // Somewhere new: nothing marked, the cursor on what the module
            // names (the folder come up out of) or else at the top.
            st.path[i] = pane.path.clone();
            st.cursor[i] = rows_of(pane)
                .iter()
                .position(|r| r.is_some_and(|e| !pane.select.is_empty() && e.name == pane.select))
                .unwrap_or(0);
            st.marked[i].clear();
        }
        let n = rows_of(pane).len();
        st.cursor[i] = st.cursor[i].min(n.saturating_sub(1));
        st.marked[i].retain(|m| pane.entries.iter().any(|e| &e.name == m));
    }

    // What the user asked for this frame: an op, on which pane, about what.
    let mut ask: Option<(&'static str, usize, Vec<String>)> = None;
    let mut moved = false;
    let focused = ctx.memory(|m| m.has_focus(focus_id));
    if focused {
        // Tab and the arrows belong to the commander while it has the keyboard,
        // not to egui's moving the focus about.
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                focus_id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            )
        });
        let a = st.active;
        let pane = panes[a];
        let n = rows_of(pane).len();
        let page = 10;
        ctx.input_mut(|i| {
            use egui::{Key, Modifiers as M};
            let cur = st.cursor[a];
            let last = n.saturating_sub(1);
            let mut to = None;
            if i.consume_key(M::NONE, Key::ArrowDown) {
                to = Some(cur + 1);
            }
            if i.consume_key(M::NONE, Key::ArrowUp) {
                to = Some(cur.saturating_sub(1));
            }
            if i.consume_key(M::NONE, Key::PageDown) {
                to = Some(cur + page);
            }
            if i.consume_key(M::NONE, Key::PageUp) {
                to = Some(cur.saturating_sub(page));
            }
            if i.consume_key(M::NONE, Key::Home) {
                to = Some(0);
            }
            if i.consume_key(M::NONE, Key::End) {
                to = Some(last);
            }
            if let Some(to) = to {
                st.cursor[a] = to.min(last);
                moved = true;
            }
            if i.consume_key(M::NONE, Key::Tab) {
                st.active ^= 1;
            }
            if i.consume_key(M::NONE, Key::Enter) {
                ask = open_at(pane, st.cursor[a], a);
            }
            if i.consume_key(M::NONE, Key::Backspace) && pane.up {
                ask = Some(("up", a, Vec::new()));
            }
            if (i.consume_key(M::NONE, Key::Insert) || i.consume_key(M::NONE, Key::Space))
                && !pane.busy
            {
                if let Some(Some(e)) = rows_of(pane).get(st.cursor[a]) {
                    let name = e.name.clone();
                    if let Some(at) = st.marked[a].iter().position(|m| *m == name) {
                        st.marked[a].remove(at);
                    } else {
                        st.marked[a].push(name);
                    }
                }
                st.cursor[a] = (st.cursor[a] + 1).min(n.saturating_sub(1));
                moved = true;
            }
            if i.consume_key(M::NONE, Key::F5) {
                ask = Some(("copy", a, chosen(&st, pane, a)));
            }
            if i.consume_key(M::NONE, Key::F8) || i.consume_key(M::NONE, Key::Delete) {
                ask = Some(("delete", a, chosen(&st, pane, a)));
            }
            if i.consume_key(M::COMMAND, Key::R) {
                ask = Some(("refresh", a, Vec::new()));
            }
        });
    }

    // The two panes, side by side, as tall as the window allows when filling.
    let width = ui.available_width();
    let pane_w = ((width - GAP) / 2.0).max(160.0);
    let bar_h = 34.0;
    let status_h = if status.is_empty() { 0.0 } else { 22.0 };
    let head_h = 46.0;
    let wanted = if *fill {
        ui.available_height() - bar_h - status_h - 6.0
    } else {
        head_h + ROW_H * if *rows == 0 { 20 } else { *rows } as f32
    };
    let height = if wanted.is_finite() { wanted.max(head_h + ROW_H * 5.0) } else { 400.0 };
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    // The whole commander is one place the keyboard can be; it asks for it
    // the first time it is shown, as a file manager window would.
    let whole = ui.interact(area, focus_id, egui::Sense::focusable_noninteractive());
    if !st.seen {
        st.seen = true;
        whole.request_focus();
    }

    for (i, pane) in panes.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            area.min + egui::vec2(i as f32 * (pane_w + GAP), 0.0),
            egui::vec2(pane_w, height),
        );
        let mut child = ui.child_ui_with_id_source(
            rect,
            egui::Layout::top_down(egui::Align::Min),
            (id.as_str(), "pane", i),
            None,
        );
        let active = st.active == i && focused;
        let got = draw_pane(&mut child, pane, i, &mut st, active, moved);
        if let Some((op, at)) = got {
            st.active = i;
            ctx.memory_mut(|m| m.request_focus(focus_id));
            if op == "open" {
                ask = open_at(pane, at, i);
            }
        }
    }

    // The function keys, for a mouse — MC's bottom line.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let a = st.active;
        if outline_button(ui, &tr("commander.f5"), egui::Vec2::ZERO).clicked() {
            ask = Some(("copy", a, chosen(&st, panes[a], a)));
        }
        if outline_button(ui, &tr("commander.f8"), egui::Vec2::ZERO).clicked() {
            ask = Some(("delete", a, chosen(&st, panes[a], a)));
        }
        if outline_button(ui, &tr("commander.refresh"), egui::Vec2::ZERO).clicked() {
            ask = Some(("refresh", a, Vec::new()));
        }
        ui.label(egui::RichText::new(tr("commander.keys")).weak().small());
    });
    if !status.is_empty() {
        ui.label(egui::RichText::new(status).weak());
    }

    if let Some(invoke) = ask.and_then(|(op, at, names)| request(op, at, names, panes, on_action)) {
        *clicked = Some(invoke);
    }
    ctx.data_mut(|d| d.insert_temp(key, st));
}

/// The names an operation is about: the marked ones, or else the one under the
/// cursor — never `..`.
fn chosen(st: &State, pane: &CommanderPane, at: usize) -> Vec<String> {
    if !st.marked[at].is_empty() {
        return st.marked[at].clone();
    }
    match rows_of(pane).get(st.cursor[at]) {
        Some(Some(e)) => vec![e.name.clone()],
        _ => Vec::new(),
    }
}

/// Enter (or a double-click) on a row: `..` goes up, a folder opens, a file
/// does nothing — there is nothing to open it with here.
fn open_at(pane: &CommanderPane, row: usize, at: usize) -> Option<(&'static str, usize, Vec<String>)> {
    match rows_of(pane).get(row)? {
        None => Some(("up", at, Vec::new())),
        Some(e) if e.dir => Some(("open", at, vec![e.name.clone()])),
        Some(_) => None,
    }
}

/// An op as a call to the module — copy and delete asked about first.
fn request(
    op: &'static str,
    at: usize,
    names: Vec<String>,
    panes: [&CommanderPane; 2],
    on_action: &AutoAction,
) -> Option<Invoke> {
    let pane = panes[at];
    if pane.busy || ((op == "copy" || op == "delete") && names.is_empty()) {
        return None;
    }
    let mut args = on_action.args.clone();
    args.insert("op".into(), Value::String(op.into()));
    args.insert("pane".into(), Value::String(if at == 0 { "left" } else { "right" }.into()));
    args.insert("path".into(), Value::String(pane.path.clone()));
    args.insert(
        "names".into(),
        Value::Array(names.iter().cloned().map(Value::String).collect()),
    );
    let listed = names.join("\n");
    let confirm = match op {
        "copy" => Some(Confirm {
            title: tr("commander.copy_title")
                .replace("{n}", &names.len().to_string())
                .replace("{to}", &panes[1 - at].path),
            subject: listed,
            confirm_label: tr("commander.copy"),
            cancel_label: tr("commander.cancel"),
        }),
        "delete" => Some(Confirm {
            title: tr("commander.delete_title")
                .replace("{n}", &names.len().to_string())
                .replace("{from}", &pane.path),
            subject: listed,
            confirm_label: tr("commander.delete"),
            cancel_label: tr("commander.cancel"),
        }),
        _ => None,
    };
    Some(Invoke {
        action: Action {
            capability: on_action.capability.clone(),
            method: on_action.method.clone(),
        },
        args,
        open_in_tab: false,
        dismiss: false,
        confirm,
    })
}

/// One pane: its title and path, the column heads, and the rows. What was
/// clicked: a row (`select`) or a double-click (`open`), and which.
fn draw_pane(
    ui: &mut egui::Ui,
    pane: &CommanderPane,
    at: usize,
    st: &mut State,
    active: bool,
    moved: bool,
) -> Option<(&'static str, usize)> {
    let mut got = None;
    let frame_rect = ui.max_rect();
    ui.painter().rect(
        frame_rect,
        egui::Rounding::ZERO,
        color::BG_ELEVATED,
        egui::Stroke::new(1.0_f32, if active { color::ACCENT } else { color::BORDER }),
    );
    let inner = frame_rect.shrink(6.0);
    let mut ui = ui.child_ui(inner, egui::Layout::top_down(egui::Align::Min), None);
    ui.set_clip_rect(inner.intersect(ui.clip_rect()));
    ui.spacing_mut().item_spacing.y = 0.0;

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&pane.title).strong().color(if active {
            color::ACCENT
        } else {
            color::TEXT
        }));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(&pane.path).weak());
    });
    ui.add_space(4.0);
    let w = ui.available_width();
    let head = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover()).0;
    let small = egui::TextStyle::Small.resolve(ui.style());
    let p = ui.painter();
    p.text(head.left_center(), egui::Align2::LEFT_CENTER, tr("commander.name"), small.clone(), color::TEXT_MUTED);
    p.text(
        egui::pos2(head.right() - DATE_W - 8.0, head.center().y),
        egui::Align2::RIGHT_CENTER,
        tr("commander.size"),
        small.clone(),
        color::TEXT_MUTED,
    );
    p.text(
        egui::pos2(head.right() - DATE_W, head.center().y),
        egui::Align2::LEFT_CENTER,
        tr("commander.modified"),
        small,
        color::TEXT_MUTED,
    );

    if pane.busy {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tr("commander.loading")).weak());
        return None;
    }
    if !pane.error.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(format!("! {}", pane.error)).color(color::ERROR));
    }
    let body = egui::TextStyle::Body.resolve(ui.style());
    egui::ScrollArea::vertical()
        .id_source(("limen_commander_rows", at))
        .auto_shrink([false, false])
        .show(&mut ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (r, row) in rows_of(pane).into_iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::click());
                let under = st.cursor[at] == r;
                if under && moved && active {
                    ui.scroll_to_rect(rect, None);
                }
                let marked = row.is_some_and(|e| st.marked[at].contains(&e.name));
                let painter = ui.painter();
                if under {
                    painter.rect_filled(
                        rect,
                        egui::Rounding::ZERO,
                        if active { color::BG_HOVER } else { color::BG_WIDGET },
                    );
                } else if resp.hovered() {
                    painter.rect_filled(rect, egui::Rounding::ZERO, with_alpha(color::BG_WIDGET, 0.6));
                }
                let (name, size, date, dir) = match row {
                    None => ("..".to_string(), tr("commander.up"), String::new(), true),
                    Some(e) => (
                        e.name.clone(),
                        if e.dir { tr("commander.dir") } else { e.size.clone() },
                        e.modified.clone(),
                        e.dir,
                    ),
                };
                let ink = if marked {
                    color::WARNING
                } else if dir {
                    color::ACCENT_BRIGHT
                } else {
                    color::TEXT
                };
                let shown = if dir && row.is_some() { format!("/{name}") } else { name };
                let name_room = rect.width() - SIZE_W - DATE_W - 16.0;
                let mut job = egui::text::LayoutJob::simple_singleline(shown, body.clone(), ink);
                job.wrap = egui::text::TextWrapping {
                    max_width: name_room.max(40.0),
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let galley = ui.fonts(|f| f.layout_job(job));
                painter.galley(
                    egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    ink,
                );
                painter.text(
                    egui::pos2(rect.right() - DATE_W - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    size,
                    body.clone(),
                    color::TEXT_MUTED,
                );
                painter.text(
                    egui::pos2(rect.right() - DATE_W, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    date,
                    body.clone(),
                    color::TEXT_MUTED,
                );
                if resp.double_clicked() {
                    st.cursor[at] = r;
                    got = Some(("open", r));
                } else if resp.clicked() {
                    st.cursor[at] = r;
                    got = Some(("select", r));
                }
            }
        });
    got
}
