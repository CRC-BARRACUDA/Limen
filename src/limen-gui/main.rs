// Don't spawn a console window alongside the GUI on Windows release builds.
// (Debug keeps the console so the stderr host/module logs stay visible.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Limen desktop GUI entry point.
//!
//! Discovers modules the same way the CLI does (configured search paths plus a
//! local `./modules` for development), then runs the egui app. All engine work
//! is on a background thread — see [`worker`].

mod app;
mod i18n;
mod ui;
mod worker;

use std::path::PathBuf;

use eframe::egui;
use limen_core::Config;

fn main() -> eframe::Result<()> {
    let dirs = resolve_search_dirs();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([980.0, 640.0])
        .with_min_inner_size([720.0, 460.0])
        // Client-side decorations: Limen draws its own title bar + window
        // controls (see `app`/`ui`), for a themed frame instead of the OS one.
        .with_decorations(false)
        // Transparent framebuffer so the startup splash shows only the floating
        // icons over the desktop; the app itself paints opaque panels over it.
        .with_transparent(true)
        // Created hidden — the app centres it and reveals it after the first
        // frame is painted (avoiding the dark/inactive flash of an empty window),
        // then maximizes once the splash ends.
        .with_visible(false)
        .with_title("Limen");
    // The window/taskbar/Alt-Tab icon while the app is running (the embedded
    // .exe icon only covers the file itself). Decoding is best-effort.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../resources/icon.png"))
    {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let result = eframe::run_native(
        "Limen",
        options,
        Box::new(|cc| Ok(Box::new(app::LimenApp::new(cc, dirs)))),
    );
    if let Err(err) = &result {
        report_startup_failure(err);
    }
    result
}

/// Tell the user why the window never appeared.
///
/// A release build has no console (`windows_subsystem = "windows"` above), so a
/// `run_native` failure is otherwise completely silent: double-clicking
/// `Limen.exe` looks like nothing happened at all. The common cause is a machine
/// with no 3D driver — `eframe`'s renderer needs OpenGL 2.0+, and Windows falls
/// back to a GDI-generic OpenGL 1.1 when no driver is installed — so point at the
/// fix rather than leaving the user with a dead double-click.
fn report_startup_failure(err: &eframe::Error) {
    let graphics = matches!(
        err,
        eframe::Error::OpenGL(_) | eframe::Error::Glutin(_) | eframe::Error::NoGlutinConfigs(..)
    );
    let body = if graphics {
        format!(
            "Limen could not start: no usable OpenGL 2.0 driver was found.\n\n\
             This usually means a virtual machine, a remote session, or a PC without 3D \
             acceleration. The Limen window is drawn with OpenGL, so it cannot open without one.\n\n\
             How to fix it — see \"Running without a 3D accelerator\":\n\
             https://github.com/{repo}#running-without-a-3d-accelerator\n\n\
             limen-cli needs no GPU and works normally.\n\n\
             Details: {err}",
            repo = limen_core::update::APP_REPO,
        )
    } else {
        format!("Limen could not start.\n\nDetails: {err}")
    };
    eprintln!("{body}");
    alert("Limen — startup failed", &body);
}

/// Show a native modal error box.
#[cfg(windows)]
fn alert(title: &str, body: &str) {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    const MB_ICONERROR: u32 = 0x0000_0010;
    const MB_SETFOREGROUND: u32 = 0x0001_0000;
    let (body, title) = (wide(body), wide(title));
    // SAFETY: both pointers are NUL-terminated UTF-16 buffers that outlive the
    // call, and a null owner handle means "no parent window" — which is the case
    // here, since the window is exactly what failed to open.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// Elsewhere the console message is the report — every desktop target Limen
/// builds for has a terminal attached.
#[cfg(not(windows))]
fn alert(_title: &str, _body: &str) {}

/// Configured search paths, plus `./modules` when developing in-repo.
fn resolve_search_dirs() -> Vec<PathBuf> {
    // Make sure the portable modules dir exists so there's a place to drop modules.
    limen_core::paths::ensure_dirs();
    let mut dirs = Config::load().map(|c| c.search_dirs()).unwrap_or_default();
    let local = PathBuf::from("modules");
    if local.is_dir() && !dirs.contains(&local) {
        dirs.push(local);
    }
    dirs
}
