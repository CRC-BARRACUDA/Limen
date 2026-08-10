//! The Limen desktop shell.
//!
//! A library so the binary beside it is only an entry point — and so the tests
//! can live outside the sources they test, which a `[[bin]]` cannot offer: an
//! integration test cannot import a binary crate.
//!
//! Discovers modules the same way the CLI does (configured search paths plus a
//! local `./modules` for development), then runs the egui app. All engine work
//! is on a background thread — see [`worker`].

pub mod app;
pub mod i18n;
pub mod worker;

/// The widget toolkit, under the name the rest of the crate has always used for
/// it. It is a crate of its own now; this keeps `ui::` meaning what it meant.
pub use limen_ui as ui;
