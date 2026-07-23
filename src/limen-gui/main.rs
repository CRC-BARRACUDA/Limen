//! Limen desktop GUI entry point.
//!
//! Discovers modules the same way the CLI does (configured search paths plus a
//! local `./modules` for development), then runs the egui app. All engine work
//! is on a background thread — see [`worker`].

mod app;
mod ui;
mod worker;

use std::path::PathBuf;

use eframe::egui;
use limen_core::Config;

fn main() -> eframe::Result<()> {
    let dirs = resolve_search_dirs();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([720.0, 460.0])
            .with_title("Limen"),
        ..Default::default()
    };

    eframe::run_native(
        "Limen",
        options,
        Box::new(|cc| Ok(Box::new(app::LimenApp::new(cc, dirs)))),
    )
}

/// Configured search paths, plus `./modules` when developing in-repo.
fn resolve_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Config::load().map(|c| c.search_dirs()).unwrap_or_default();
    let local = PathBuf::from("modules");
    if local.is_dir() && !dirs.contains(&local) {
        dirs.push(local);
    }
    dirs
}
