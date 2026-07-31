//! Where the mouse actually is, asked of the OS rather than the window system
//! abstraction.
//!
//! winit does not report the cursor during a file drag on any backend: the
//! Windows `DragOver` handler takes a `POINTL` and ignores it, the X11 one reads
//! the XDND coordinates and leaves them commented out, and Wayland emits no drag
//! events at all. Without a position, a view holding several drop targets cannot
//! tell which one the user is hovering.
//!
//! So we ask the platform straight out, but only while a drag is in flight —
//! this is not a general-purpose input path, and normal pointer handling still
//! goes through egui.
//!
//! Returns **physical pixels in screen space**, or `None` where the query is
//! unavailable (Wayland, macOS, a missing X display). Callers fall back to
//! choosing a drop target by focus.

use eframe::egui;

/// The cursor's position in physical screen pixels, if the platform will say.
pub fn screen_pos() -> Option<egui::Pos2> {
    imp::screen_pos()
}

/// The learned mapping from screen pixels to UI points.
///
/// Both the origin *and* the scale are measured, never assumed. Under XWayland
/// on a HiDPI panel the X server reports one pixel space while egui works in
/// another, and `pixels_per_point` is not the ratio between them — trusting it
/// put the cursor several fields away from where it really was.
#[derive(Clone, Copy)]
struct Calib {
    screen: egui::Pos2,
    ui: egui::Pos2,
    scale: egui::Vec2,
}

fn calib_id() -> egui::Id {
    egui::Id::new("limen_cursor_calib")
}

/// Learn the screen→UI mapping from a frame where the pointer *is* tracked.
///
/// Converting screen coordinates into UI space needs to know where the window
/// is and how its pixels relate to points. `viewport().inner_rect` is not
/// reliably populated — on X11 it is often absent entirely — but whenever egui
/// knows the pointer normally we have the same point in both spaces, and two
/// such points separated by some distance also give us the scale.
///
/// Only calibrate when nothing is being dragged: during a drag egui's pointer is
/// frozen while the OS one keeps moving, and pairing those would learn nonsense.
pub fn calibrate(ctx: &egui::Context) {
    let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if dragging {
        return;
    }
    // `hover_pos` — not `pointer_latest_pos` — because the latter keeps
    // returning the last in-window position after the cursor leaves. Pairing
    // that stale point with a live OS one teaches an offset wrong by however far
    // the mouse has travelled since, which is exactly what happens on the way to
    // another window to pick up a file.
    let Some(ui) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let Some(screen) = screen_pos() else {
        return;
    };

    let prev = ctx.data(|d| d.get_temp::<Calib>(calib_id()));
    let mut scale = prev.map_or_else(|| egui::Vec2::splat(ctx.pixels_per_point()), |c| c.scale);
    // Refine the scale from a pair far enough apart to be meaningful; short
    // moves are dominated by rounding and would only add noise.
    if let Some(p) = prev {
        let ds = screen - p.screen;
        let du = ui - p.ui;
        if ds.x.abs() > 40.0 && du.x.abs() > 4.0 {
            scale.x = lerp_f32(scale.x, ds.x / du.x, 0.5);
        }
        if ds.y.abs() > 40.0 && du.y.abs() > 4.0 {
            scale.y = lerp_f32(scale.y, ds.y / du.y, 0.5);
        }
    }
    ctx.data_mut(|d| d.insert_temp(calib_id(), Calib { screen, ui, scale }));
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Whether the cursor has actually been seen to move during this drag.
fn drag_probe_id() -> egui::Id {
    egui::Id::new("limen_drag_probe")
}

/// The cursor's position in UI space during a file drag, or `None` when the
/// platform will not say where it is.
///
/// The query can be silently stale rather than absent. A drag started from a
/// Wayland application and delivered to an X11 window arrives as XDND client
/// messages while the compositor keeps the pointer grab, so XWayland never
/// updates its core pointer and `QueryPointer` keeps returning the last position
/// the cursor held over an X11 surface — a plausible, fixed, wrong answer.
///
/// So the position is not trusted until it is observed to *change*. Until then
/// this reports `None` and callers route drops by focus instead, which is what
/// happens for the whole of such a drag. Where the platform is honest — Windows
/// via `GetCursorPos` — the first real movement switches tracking on.
pub fn drag_pos(ctx: &egui::Context) -> Option<egui::Pos2> {
    let dragging = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if !dragging {
        ctx.data_mut(|d| d.remove::<(egui::Pos2, bool)>(drag_probe_id()));
        return None;
    }
    let screen = screen_pos()?;
    // Where the query is live, take it at once. The probe below guards against a
    // *stale* answer, which Windows never gives; running it there only withholds
    // a correct position until the cursor has moved a couple of pixels, and for
    // those frames every candidate field opens by focus rather than by cursor —
    // a zone lighting up away from the pointer at the start of each drag.
    if imp::POSITION_IS_LIVE {
        return to_ui_space(ctx, screen);
    }
    let (first, moved) = ctx
        .data(|d| d.get_temp::<(egui::Pos2, bool)>(drag_probe_id()))
        .unwrap_or((screen, false));
    let moved = moved || (screen - first).length() > 2.0;
    ctx.data_mut(|d| d.insert_temp(drag_probe_id(), (first, moved)));
    moved.then(|| to_ui_space(ctx, screen)).flatten()
}

/// Translate a screen-space position into this window's UI coordinates, so it
/// can be tested against widget rects. `None` until [`calibrate`] has seen the
/// pointer at least once.
///
/// Anchored on the most recent sample rather than a window origin, so any
/// residual error in the scale stays proportional to how far the cursor has
/// moved since — not to its distance from the corner of the screen.
pub fn to_ui_space(ctx: &egui::Context, screen: egui::Pos2) -> Option<egui::Pos2> {
    let c = ctx.data(|d| d.get_temp::<Calib>(calib_id()))?;
    if c.scale.x.abs() < 0.1 || c.scale.y.abs() < 0.1 {
        return None;
    }
    let d = screen - c.screen;
    let pos = c.ui + egui::vec2(d.x / c.scale.x, d.y / c.scale.y);
    // A position far outside the window means the calibration is stale — the
    // window moved, or the pair was learned badly. Report "unknown" rather than
    // a confident wrong answer, so callers fall back to focus routing instead of
    // quietly refusing every drop.
    ctx.screen_rect().expand(64.0).contains(pos).then_some(pos)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use eframe::egui;
    use std::sync::OnceLock;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    /// One connection for the process. Opening an X connection per frame would
    /// be far too costly, and `None` (no X display — a pure Wayland session)
    /// is cached just as deliberately so we stop retrying.
    fn conn() -> Option<&'static (RustConnection, usize)> {
        static CONN: OnceLock<Option<(RustConnection, usize)>> = OnceLock::new();
        CONN.get_or_init(|| x11rb::connect(None).ok()).as_ref()
    }

    /// `QueryPointer` keeps answering under XWayland even when the compositor
    /// holds the pointer grab — a fixed, plausible, wrong position. The caller
    /// must watch for movement before believing it.
    pub const POSITION_IS_LIVE: bool = false;

    pub fn screen_pos() -> Option<egui::Pos2> {
        let (c, screen_num) = conn()?;
        let root = c.setup().roots.get(*screen_num)?.root;
        let reply = c.query_pointer(root).ok()?.reply().ok()?;
        Some(egui::pos2(reply.root_x as f32, reply.root_y as f32))
    }
}

#[cfg(windows)]
mod imp {
    use eframe::egui;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    /// `GetCursorPos` tracks the cursor throughout an OLE drag, so its answer
    /// can be used the moment a drag starts.
    pub const POSITION_IS_LIVE: bool = true;

    pub fn screen_pos() -> Option<egui::Pos2> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: `GetCursorPos` only writes the POINT we hand it, and reports
        // failure through its return value rather than leaving it uninitialised.
        let ok = unsafe { GetCursorPos(&mut p) };
        (ok != 0).then(|| egui::pos2(p.x as f32, p.y as f32))
    }
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
mod imp {
    use eframe::egui;

    /// Never consulted — there is no position to be live.
    pub const POSITION_IS_LIVE: bool = false;

    pub fn screen_pos() -> Option<egui::Pos2> {
        None
    }
}

#[cfg(test)]
mod tests {
    /// Smoke test for the platform query itself. It cannot assert a position —
    /// there may be no display, and on Wayland there is deliberately no answer —
    /// but run with `--nocapture` it shows whether the cursor can be read here at
    /// all, which is the difference between drop routing following the pointer
    /// and silently falling back to focus.
    #[test]
    fn reports_a_cursor_position_where_the_platform_allows_it() {
        match super::screen_pos() {
            Some(p) => println!("cursor query OK: {p:?}"),
            None => println!("cursor query unavailable on this platform/session"),
        }
    }
}
