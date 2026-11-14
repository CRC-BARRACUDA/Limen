//! A list of sections down the left, and the open section beside it.
//!
//! For a screen that is several pages about one thing — a host's system, its
//! network, its apps — where a row of buttons across the top runs out of width
//! long before it runs out of sections, and where which section is open is the
//! thing most worth seeing at a glance.
//!
//! Picking a section is the module's business: it is called with the item's
//! `key`, and answers with the same sidebar, that item selected, and that
//! section's body. Because the sidebar is the same widget on both screens, only
//! the body changes — and the body is drawn the way a screen is, so what arrives
//! and leaves in it animates like anything else.
//!
//! Folding the list away is the host's alone: no call is made, nothing waits
//! on the module, and the fold is remembered by the sidebar's `id`, so it stays
//! folded while the sections change underneath it.

use crate::*;

/// How wide the list is when it is open.
const OPEN: f32 = 188.0;
/// What is left of it folded: the strip the toggle sits in.
const FOLDED: f32 = 28.0;
/// Between the list and the section, with the rule down the middle of it.
const GAP: f32 = 16.0;
const ITEM_H: f32 = 30.0;
const TOGGLE_H: f32 = 24.0;
/// How long folding takes. As short as a menu: it is the body's width that is
/// being asked for, and waiting for it is waiting for nothing.
const FOLD_SECS: f32 = 0.22;

/// Where whether a sidebar is folded is kept.
///
/// By `id` alone rather than under the `Ui` drawing it, so the fold survives
/// anything the screen around it does — a section change answered with a whole
/// new screen, the same panel opened again in another tab. Folding the list is
/// a preference about the panel, not about one screen of it.
pub fn sidebar_folded_id(id: &str) -> egui::Id {
    egui::Id::new(("limen_sidebar_folded", id))
}

/// Draw a [`Widget::Sidebar`]. Anything else is ignored.
pub(crate) fn render(
    ui: &mut egui::Ui,
    widget: &Widget,
    inputs: &mut HashMap<String, String>,
    busy: Option<&Action>,
    clicked: &mut Option<Invoke>,
) {
    let Widget::Sidebar {
        id,
        items,
        selected,
        action,
        args,
        body,
    } = widget
    else {
        return;
    };
    let fold_id = sidebar_folded_id(id);
    let folded: bool = ui.ctx().data(|d| d.get_temp(fold_id)).unwrap_or(false);
    // 1 open, 0 folded. Eased at both ends: it is reversible motion, and a
    // fold that set off at full speed would read as the list being snatched.
    let open = smoothstep(anim_bool_ctx(ui.ctx(), fold_id.with("t"), !folded, FOLD_SECS));
    let width = FOLDED + (OPEN - FOLDED) * open;

    let origin = ui.available_rect_before_wrap().min;
    let room_w = ui.available_width();
    let room_h = ui.available_height();

    // The section first: how tall it is decides how far the rule runs.
    let body_rect = egui::Rect::from_min_size(
        egui::pos2(origin.x + width + GAP, origin.y),
        egui::vec2((room_w - width - GAP).max(80.0), room_h),
    );
    // Each section is its own page. Keyed by the section, so that two of them
    // are never taken for one screen that changed: the flow would read the new
    // section's heading as the old heading with new words, and leave it in
    // place — and morph one section's table into another's.
    let mut section = ui.child_ui_with_id_source(
        body_rect,
        egui::Layout::top_down(egui::Align::Min),
        ("limen_sidebar_body", id.as_str(), selected.as_str()),
        None,
    );
    // And it enters from when it was picked, which keying alone does not give:
    // a section visited before is already known to the flow, and would come
    // back without an entrance at all.
    let now = ui.input(|i| i.time);
    let entered_id = fold_id.with("entered");
    let entered_at = match ui.ctx().data(|d| d.get_temp::<(String, f64)>(entered_id)) {
        Some((key, at)) if key == *selected => at,
        _ => {
            ui.ctx()
                .data_mut(|d| d.insert_temp(entered_id, (selected.clone(), now)));
            now
        }
    };
    // On first sight the section joins the screen's entrance rather than
    // snapping in while everything around the sidebar fades up.
    let screen = ui.data(|d| d.get_temp::<f64>(view_reveal_id())).unwrap_or(0.0);
    if let Some(invoke) = render_list(&mut section, body, inputs, busy, screen.max(entered_at)) {
        *clicked = Some(invoke);
    }
    let body_h = section.min_rect().height();

    let list_rect = egui::Rect::from_min_size(origin, egui::vec2(width, room_h));
    let mut list = ui.child_ui_with_id_source(
        list_rect,
        egui::Layout::top_down(egui::Align::Min),
        ("limen_sidebar_list", id.as_str()),
        None,
    );
    // Nothing of a folding list reaches into the section beside it, and egui
    // tests a hit against the clip rect, so nothing outside it can be clicked.
    list.set_clip_rect(list_rect.intersect(ui.clip_rect()));

    if fold_toggle(&mut list, open, folded).clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(fold_id, !folded));
    }
    list.add_space(6.0);

    // Folded all the way, the items are not there to be clicked at all.
    if open > 0.001 {
        list.set_opacity(open);
        for item in items {
            if section_item(&mut list, item, item.key == *selected).clicked() {
                let mut args = args.clone();
                args.insert("key".into(), Value::String(item.key.clone()));
                *clicked = Some(Invoke {
                    action: action.clone(),
                    args,
                    open_in_tab: false,
                    dismiss: false,
                    confirm: None,
                });
            }
        }
    }
    let list_h = list.min_rect().height();

    let height = body_h.max(list_h);
    let x = origin.x + width + GAP * 0.5;
    ui.painter().vline(
        x,
        origin.y..=origin.y + height,
        egui::Stroke::new(1.0_f32, color::BORDER),
    );
    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
        origin,
        egui::vec2(room_w, height),
    ));
}

/// The button that folds the list away and brings it back.
///
/// A painted chevron rather than a glyph — the app ships one font — and one
/// that turns as the list moves: pointing left while there is a list to fold,
/// right when there is one to bring back, and through the middle in between.
fn fold_toggle(ui: &mut egui::Ui, open: f32, folded: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(FOLDED, TOGGLE_H), egui::Sense::click());
    let a = interact(ui, &resp);
    let painter = ui.painter();
    painter.rect_filled(rect, egui::Rounding::ZERO, with_alpha(color::BG_WIDGET, a.hover));

    // -1 points left, +1 right.
    let dir = 1.0 - 2.0 * open;
    let c = rect.center();
    let (w, h) = (4.0 * dir, 5.0);
    let stroke = egui::Stroke::new(
        1.5_f32,
        lerp_color(color::TEXT_MUTED, color::ACCENT_BRIGHT, a.hover),
    );
    painter.line_segment([c + egui::vec2(-w * 0.5, -h), c + egui::vec2(w * 0.5, 0.0)], stroke);
    painter.line_segment([c + egui::vec2(w * 0.5, 0.0), c + egui::vec2(-w * 0.5, h)], stroke);

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tr(if folded { "sidebar.show" } else { "sidebar.hide" }))
}

/// One section in the list.
///
/// Drawn the width of the open list whatever the list's width is now, and left
/// to the clip: a folding list then slides its items under the rule rather than
/// re-wrapping them into a column of single letters on the way.
fn section_item(ui: &mut egui::Ui, item: &SidebarItem, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(OPEN, ITEM_H), egui::Sense::click());
    let a = interact(ui, &resp);
    let sel = anim_bool(ui, resp.id.with("sel"), selected, 0.14);
    let painter = ui.painter();

    painter.rect_filled(
        rect,
        egui::Rounding::ZERO,
        with_alpha(color::BG_WIDGET, a.hover.max(sel * 0.6)),
    );
    // The bar that says which one is open, read before the text is.
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
        egui::Rounding::ZERO,
        with_alpha(color::ACCENT, sel),
    );

    // A section with nothing to show yet — its module not installed, say — is
    // still listed, so the list does not look complete when it is not; it is
    // only quieter.
    let base = if item.muted { color::TEXT_MUTED } else { color::TEXT };
    let mut job = egui::text::LayoutJob::simple_singleline(
        item.text.clone(),
        egui::TextStyle::Body.resolve(ui.style()),
        lerp_color(base, color::ACCENT, sel),
    );
    // One line, ended with an ellipsis rather than cut off mid-letter.
    job.wrap = egui::text::TextWrapping {
        max_width: OPEN - 24.0,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.fonts(|f| f.layout_job(job));
    let at = egui::pos2(rect.min.x + 12.0, rect.center().y - galley.size().y * 0.5);
    painter.galley(at, galley, color::TEXT);

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
