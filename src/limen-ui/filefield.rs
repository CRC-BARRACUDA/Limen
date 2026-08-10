//! The path field: typing, browsing, and being dropped onto.

use crate::*;

/// Where a [`Widget::File`] parks a pending Browse request: `(widget id, wants
/// a directory)`. The app picks it up after rendering and runs the OS dialog on
/// its own thread — the dialog blocks, and the UI thread only draws.
pub fn browse_request_id() -> egui::Id {
    egui::Id::new("limen_browse_request")
}

/// Marks the frame on which a dropped path was already taken by a field, so a
/// single drop cannot land in two of them at once.
pub fn drop_claim_id() -> egui::Id {
    egui::Id::new("limen_drop_claimed")
}

/// Marks the frame on which a field already offered itself as the drop target,
/// so only one opens even when several could accept what is being dragged.
pub fn invite_claim_id() -> egui::Id {
    egui::Id::new("limen_drop_invited")
}

/// The field that was armed on the last frame a drag hovered.
///
/// A drop arrives one frame *after* the hover ends — `dropped_files` is set on
/// the same frame `hovered_files` empties — so by then there is no drag left to
/// test against. This remembers the answer from while it was still visible.
pub fn armed_field_id() -> egui::Id {
    egui::Id::new("limen_drop_armed")
}

/// The path field the caret is in: `(widget id, wants a directory)`.
///
/// winit reports *that* files are hovering but never *where* — on Wayland at
/// all, and on X11 the XDND coordinates are read and then discarded. With no
/// cursor to test against, a view holding several fields of the same kind would
/// always drop into the first. Focus is the tie-break: click the field you mean,
/// then drop.
pub fn focused_field_id() -> egui::Id {
    egui::Id::new("limen_path_focus")
}

/// What a [`Widget::File`] will take.
#[derive(Clone, Copy, PartialEq)]
pub enum Accepts {
    File,
    Dir,
    Either,
}

impl Accepts {
    /// Read the spec, falling back to the older `directory` flag when `accepts`
    /// is absent. Anything unrecognised means a file — the narrower reading, so
    /// a typo cannot quietly widen what a field will swallow.
    pub fn of(accepts: &str, directory: bool) -> Self {
        let a = accepts.trim().to_lowercase();
        if a.is_empty() {
            return if directory {
                Accepts::Dir
            } else {
                Accepts::File
            };
        }
        match (a.contains("file"), a.contains("dir")) {
            (true, true) => Accepts::Either,
            (false, true) => Accepts::Dir,
            _ => Accepts::File,
        }
    }

    /// Whether a dropped path of this kind belongs here.
    pub fn takes(self, is_dir: bool) -> bool {
        match self {
            Accepts::Either => true,
            Accepts::Dir => is_dir,
            Accepts::File => !is_dir,
        }
    }
}

/// A path field: type it, drop a file or folder on it, or Browse for it.
///
/// Returns `true` when `path` changed. Drops are matched against this field's
/// rect, so a view with several path inputs routes each drop to the one under
/// the cursor.
#[allow(clippy::too_many_arguments)]
pub fn file_field(
    ui: &mut egui::Ui,
    id: &str,
    path: &mut String,
    placeholder: &str,
    browse_label: &str,
    browse_dir_label: &str,
    accepts: Accepts,
) -> bool {
    let ctx = ui.ctx().clone();
    let frame = ctx.frame_nr();
    // What is being dragged over the window: `Some(true)` a directory,
    // `Some(false)` a file, `None` nothing (or a drag whose path the platform
    // withheld, which we cannot classify and therefore do not invite).
    let dragged_is_dir = ctx.input(|i| {
        i.raw
            .hovered_files
            .iter()
            .find_map(|f| f.path.as_ref())
            .map(|p| p.is_dir())
    });
    // Every field that could take this thing opens, so all the valid targets are
    // visible at once — dragging a file over a directory field still invites
    // nothing, and vice versa.
    let suits_me = dragged_is_dir.is_some_and(|d| accepts.takes(d));
    // Where the pointer is, asked of the OS: winit reports the drag but never
    // its position. `None` where that query is unavailable — a Wayland session,
    // macOS — and the caret becomes the tie-break instead.
    let drag_pos = suits_me.then(|| cursor::drag_pos(&ctx)).flatten();
    let mut changed = false;

    let row = ui.horizontal(|ui| {
        // Lay the button out first, right-aligned, and give the field whatever
        // is left. `outline_button`'s size argument is only a *minimum*, so a
        // long label — "Choose folder…", or any translation of it — grows the
        // button; reserving a fixed width for it pushed the row off-screen.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // A field that takes either kind offers both dialogs, because the
            // OS has no single one that picks a file *or* a folder. Rendered
            // right-to-left, so the folder button sits outermost.
            if matches!(accepts, Accepts::Either)
                && !browse_dir_label.is_empty()
                && outline_button(ui, browse_dir_label, egui::Vec2::ZERO).clicked()
            {
                ctx.data_mut(|d| d.insert_temp(browse_request_id(), (id.to_string(), true)));
            }
            let wants_dir = matches!(accepts, Accepts::Dir);
            if outline_button(ui, browse_label, egui::Vec2::ZERO).clicked() {
                ctx.data_mut(|d| d.insert_temp(browse_request_id(), (id.to_string(), wants_dir)));
            }
            let field_w = ui.available_width().max(80.0);
            let before = path.clone();
            let resp = text_field(ui, path, placeholder, field_w, false);
            changed |= *path != before;

            // Remember where the caret is, so a drag with no cursor to follow
            // still knows which of several same-kind fields the user means.
            if resp.has_focus() {
                ctx.data_mut(|d| {
                    d.insert_temp(
                        focused_field_id(),
                        (id.to_string(), matches!(accepts, Accepts::Dir)),
                    )
                });
            }
        });
    });

    // A collision box around the row, tested against the cursor: enter it and
    // this field becomes the drop zone, leave and it goes back to being a text
    // box. The box follows the zone as it grows — sized from *last* frame's
    // openness — so moving down into the expanded area keeps it open instead of
    // falling straight back out of the rect that opened it.
    const GROW: f32 = 44.0;
    let r = row.response.rect;
    let was_open = ctx
        .data(|d| d.get_temp::<f32>(egui::Id::new(("dropopen", id))))
        .unwrap_or(0.0);
    let hit = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() + GROW * was_open))
        .expand(6.0);

    // Only the field the cursor is actually in opens. Without a cursor to test
    // — a Wayland session, macOS — fall back to showing every candidate and
    // arming the focused one, since otherwise nothing would open at all.
    let focused = ctx.data(|d| d.get_temp::<(String, bool)>(focused_field_id()));
    let (want_open, want_armed) = match drag_pos {
        Some(pos) => {
            let inside = suits_me && hit.contains(pos);
            (inside, inside)
        }
        None => {
            let mine = focused.as_ref().is_some_and(|(f, _)| f == id);
            let elsewhere = focused
                .as_ref()
                .is_some_and(|(f, dir)| f != id && accepts.takes(*dir));
            let taken = ctx.data(|d| d.get_temp::<u64>(invite_claim_id())) == Some(frame);
            (suits_me, suits_me && (mine || (!elsewhere && !taken)))
        }
    };
    if want_armed {
        ctx.data_mut(|d| {
            d.insert_temp(invite_claim_id(), frame);
            // The drop itself arrives a frame later, by which point
            // `hovered_files` is empty and nothing can be recomputed — so
            // remember who was armed while we still know.
            d.insert_temp(armed_field_id(), id.to_string());
        });
    }
    let open = anim_bool(ui, egui::Id::new(("dropzone", id)), want_open, 0.16);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("dropopen", id)), open));

    // While open, the field *is* the drop target: it grows downward and a cross
    // marks the middle. Painted over the row rather than swapped for it, so the
    // layout below eases apart instead of jumping.
    if open > 0.0 {
        let extra = GROW * open;
        ui.add_space(extra);
        let zone = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() + extra));
        let armed = anim_bool(ui, egui::Id::new(("dropzonelit", id)), want_armed, 0.16);

        let p = ui.painter();
        let rounding = egui::Rounding::same(3.0);
        // ACCENT is the only colour used — no tinted wash underneath it. Washing
        // one amber over another is what muddied this into brown; the zone keeps
        // the ordinary input background and speaks entirely through its outline.
        let k = (0.4 + 0.6 * armed) * open;
        // Opaque at full open, so the row underneath is covered rather than
        // showing through the zone.
        p.rect_filled(zone, rounding, with_alpha(color::BG_ELEVATED, open));
        p.rect_stroke(
            zone,
            rounding,
            egui::Stroke::new(1.0 + 0.5 * armed, with_alpha(color::ACCENT, k)),
        );
        // The cross, drawn rather than typed — no glyph to depend on.
        let c = zone.center();
        let arm = 13.0 * open;
        let stroke = egui::Stroke::new(1.5 + 0.5 * armed, with_alpha(color::ACCENT, k));
        p.line_segment([c - egui::vec2(arm, 0.0), c + egui::vec2(arm, 0.0)], stroke);
        p.line_segment([c - egui::vec2(0.0, arm), c + egui::vec2(0.0, arm)], stroke);
    }

    // The drop lands a frame after the hover ends, so it cannot be matched
    // against a live drag — the field armed on the last hovered frame takes it,
    // provided the kind still agrees.
    if let Some(dropped) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
        let armed_id = ctx.data(|d| d.get_temp::<String>(armed_field_id()));
        let taken = ctx.data(|d| d.get_temp::<u64>(drop_claim_id())) == Some(frame);
        if !taken && armed_id.as_deref() == Some(id) && accepts.takes(dropped.is_dir()) {
            *path = dropped.display().to_string();
            changed = true;
            ctx.data_mut(|d| d.insert_temp(drop_claim_id(), frame));
        }
    }
    changed
}
