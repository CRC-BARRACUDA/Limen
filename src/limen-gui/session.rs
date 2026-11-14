//! Which display server to run on, decided before the window is opened.
//!
//! A Linux desktop can offer both: a Wayland session nearly always runs
//! XWayland beside it, so an application that asks for X11 gets a working
//! window either way. Which to take is a real choice, not a formality:
//!
//! * **Wayland** is the session's own protocol. Nothing is translated, and the
//!   compositor presents frames directly — so none of XWayland's presentation
//!   path is involved, which is where a frozen window comes from.
//! * **X11** is what winit implements file drag-and-drop on. On Wayland the
//!   backend emits neither `HoveredFile` nor `DroppedFile`, so a path field
//!   cannot be filled by dropping a file onto it there.
//!
//! The session's own backend is taken, because a window that stops answering is
//! worse than a drop that has a button beside it (every path field also opens a
//! file picker). `LIMEN_BACKEND=x11` takes the other side of that trade for
//! anyone who drops files more than they lose windows.

/// A display server to run on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    Wayland,
    X11,
}

impl Backend {
    /// What to call it in a log line or an error message.
    pub fn name(self) -> &'static str {
        match self {
            Backend::Wayland => "wayland",
            Backend::X11 => "x11",
        }
    }
}

/// What to run on, given the `LIMEN_BACKEND` override and the two display
/// variables. `None` means there is nothing to decide — no display at all, and
/// the error belongs to whoever tries to open a window.
///
/// Empty is the same as unset: a stray `DISPLAY=` in a login script is not an
/// X server, and treating it as one is how an app ends up with no window on a
/// perfectly good Wayland session.
pub fn choose(ask: Option<&str>, wayland: Option<&str>, x11: Option<&str>) -> Option<Backend> {
    let there = |var: Option<&str>| var.is_some_and(|v| !v.is_empty());
    let (wayland, x11) = (there(wayland), there(x11));

    // An override is honoured only where that session exists. Asking for a
    // display server that is not running is not a preference, it is a window
    // that never opens — so the ask is dropped and the session decides.
    if let Some(ask) = ask {
        match ask.trim().to_ascii_lowercase().as_str() {
            "wayland" if wayland => return Some(Backend::Wayland),
            "x11" if x11 => return Some(Backend::X11),
            _ => {}
        }
    }
    match (wayland, x11) {
        (true, _) => Some(Backend::Wayland),
        (false, true) => Some(Backend::X11),
        (false, false) => None,
    }
}

/// Read the three variables and decide. See [`choose`].
pub fn chosen() -> Option<Backend> {
    let var = |name: &str| std::env::var(name).ok();
    let (ask, wayland, socket, x11) = (
        var("LIMEN_BACKEND"),
        var("WAYLAND_DISPLAY"),
        var("WAYLAND_SOCKET"),
        var("DISPLAY"),
    );
    // Either variable names a Wayland session: `WAYLAND_SOCKET` is what a
    // compositor passes to something it launched itself.
    let wayland = wayland.filter(|v| !v.is_empty()).or(socket);
    choose(ask.as_deref(), wayland.as_deref(), x11.as_deref())
}
