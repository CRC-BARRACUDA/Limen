//! Which display server the app decides on, before it opens a window.

use limen_gui::session::{Backend, choose};

/// A session running both — which every Wayland desktop with XWayland does —
/// takes the native one. This is the case the decision exists for: it is also
/// the one where getting it wrong is invisible until a window stops answering.
#[test]
fn a_session_offering_both_takes_its_own() {
    assert_eq!(
        choose(None, Some("wayland-0"), Some(":0")),
        Some(Backend::Wayland)
    );
}

/// And a session offering one takes that one, override or not.
#[test]
fn a_session_offering_one_takes_what_there_is() {
    assert_eq!(choose(None, Some("wayland-0"), None), Some(Backend::Wayland));
    assert_eq!(choose(None, None, Some(":0")), Some(Backend::X11));
    assert_eq!(choose(None, None, None), None, "nothing to decide");
}

/// The override is for the trade the default makes: file drag-and-drop, which
/// winit implements on X11 only.
#[test]
fn the_override_takes_the_other_side_of_the_trade() {
    let both = |ask| choose(ask, Some("wayland-0"), Some(":0"));
    assert_eq!(both(Some("x11")), Some(Backend::X11));
    assert_eq!(both(Some("wayland")), Some(Backend::Wayland));
    // Spelling it as anybody would.
    assert_eq!(both(Some("X11")), Some(Backend::X11));
    assert_eq!(both(Some(" Wayland ")), Some(Backend::Wayland));
    assert_eq!(both(Some("nonsense")), Some(Backend::Wayland), "ignored");
}

/// An override naming a session that is not running is dropped. Honouring it
/// would turn a preference into an application with no window — the one outcome
/// worse than either backend.
#[test]
fn an_override_for_a_session_that_is_not_there_is_dropped() {
    assert_eq!(
        choose(Some("x11"), Some("wayland-0"), None),
        Some(Backend::Wayland),
        "asked for X11 where there is no X display"
    );
    assert_eq!(
        choose(Some("wayland"), None, Some(":0")),
        Some(Backend::X11),
        "asked for Wayland where there is no Wayland session"
    );
    assert_eq!(choose(Some("x11"), None, None), None, "asked for nothing");
}

/// Empty is unset. A stray `DISPLAY=` from a login script is not an X server,
/// and reading it as one strands a perfectly good Wayland session on a backend
/// that cannot open a window.
#[test]
fn an_empty_variable_is_not_a_session() {
    assert_eq!(choose(None, Some("wayland-0"), Some("")), Some(Backend::Wayland));
    assert_eq!(choose(None, Some(""), Some(":0")), Some(Backend::X11));
    assert_eq!(choose(None, Some(""), Some("")), None);
    assert_eq!(
        choose(Some("x11"), Some("wayland-0"), Some("")),
        Some(Backend::Wayland),
        "an override cannot conjure a display either"
    );
}
