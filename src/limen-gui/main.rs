// Don't spawn a console window alongside the GUI on Windows release builds.
// (Debug keeps the console so the stderr host/module logs stay visible.)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Limen desktop GUI entry point: window options, startup failures, and where
//! to look for modules. Everything else is the library beside it.

use limen_gui::{app, i18n, ui};

use std::path::PathBuf;

use eframe::egui;
use limen_core::Config;

/// Run on X11 rather than Wayland, where both are available.
///
/// winit implements no file drag-and-drop on Wayland at all — the backend emits
/// neither `HoveredFile` nor `DroppedFile` — so a path field could never be
/// filled by dropping onto it there. Under X11 (including XWayland) the drop
/// arrives, so we prefer it: winit picks X11 when `WAYLAND_DISPLAY` is unset.
///
/// Only when there is an X display to fall back to. On a Wayland session with no
/// XWayland, forcing this would leave the app unable to open a window at all,
/// which is a far worse trade than losing drag-and-drop.
#[cfg(all(unix, not(target_os = "macos")))]
fn prefer_x11() {
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        // SAFETY: single-threaded here — this runs first thing in `main`, before
        // any thread is spawned and before winit reads the environment.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn prefer_x11() {}

/// Be the elevated supervisor instead of opening a window, when asked.
///
/// The supervisor is a *binary run as root by path*, so which path Limen hands
/// to `pkexec`/UAC decides what gets root. It hands over the running executable
/// — the one binary an attacker cannot swap without having already won — rather
/// than a sibling file that no digest, lockfile or trust approval covers, and
/// that on a portable install sits on the USB stick writable by every machine it
/// has ever touched. The cost of that choice is this function: the GUI binary
/// must understand `supervise` too, because it is now sometimes the supervisor.
///
/// Parsed by hand, ahead of everything. There is no clap here, and this must run
/// before the window, the translator, or anything that wants a display — an
/// elevated helper has no session to open one in.
///
/// `Some(code)` means we were the supervisor and are done; `None` means carry on
/// and be the app.
fn supervise_instead() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("supervise") {
        return None;
    }
    let (mut socket, mut cwd, mut argv) = (String::new(), None, Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" => socket = args.next().unwrap_or_default(),
            "--cwd" => cwd = args.next(),
            // Everything after `--` is the command, fixed here on the command
            // line: it is what the authorization covered, and the socket is
            // never allowed to introduce another.
            "--" => {
                argv.extend(args.by_ref());
                break;
            }
            _ => {}
        }
    }
    if socket.is_empty() || argv.is_empty() {
        eprintln!("supervise: needs --connect <socket> and a command after --");
        return Some(2);
    }
    Some(match limen_core::supervise(&socket, cwd.as_deref(), &argv) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("supervise: {e:#}");
            1
        }
    })
}

fn main() -> eframe::Result<()> {
    // Before the window, the translator, or anything needing a display.
    if let Some(code) = supervise_instead() {
        std::process::exit(code);
    }
    prefer_x11();
    // The toolkit has strings of its own — month names, the words on pop-up
    // buttons — but no catalogs and no opinion about language. It asks through
    // this, and the application answers.
    ui::set_translator(i18n::t);
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
