//! Where the pointer is during a drag.

/// Smoke test for the platform query itself. It cannot assert a position —
/// there may be no display, and on Wayland there is deliberately no answer —
/// but run with `--nocapture` it shows whether the cursor can be read here at
/// all, which is the difference between drop routing following the pointer and
/// silently falling back to focus.
#[test]
fn reports_a_cursor_position_where_the_platform_allows_it() {
    match limen_ui::cursor::screen_pos() {
        Some(p) => println!("cursor query OK: {p:?}"),
        None => println!("cursor query unavailable on this platform/session"),
    }
}
