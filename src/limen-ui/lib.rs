//! Limen's widget toolkit.
//!
//! The painted controls, the pop-up windows, the calendar, and the renderer that
//! turns a module's view into a screen. A crate of its own so the boundary is
//! enforced by the compiler rather than by intent: nothing in here may reach
//! back into the application, which is what kept a five-thousand-line file from
//! being read as one.
//!
//! The GUI core: a declarative view protocol + a single standardized renderer.
//!
//! Modules can't call egui directly (they're separate processes, in any
//! language), so instead each module *describes* the UI it wants by answering a
//! `ui` request with a [`View`] — a small widget tree in JSON. This core renders
//! that tree with one consistent (Zed One Dark) style and routes button clicks
//! back to the module's capabilities. So every module "draws its own window"
//! while the look-and-feel stays uniform and is defined in exactly one place.
//!
//! The same primitives are showcased in the debug-only `demo-ui` gallery, which
//! is where the styles are standardized.

pub mod cursor;

use std::sync::{OnceLock, RwLock};

/// How this crate asks for a translated string.
///
/// The catalogs, the chosen language and the fallback rules belong to the
/// application, not to a widget: a toolkit that owned them could not be used by
/// anything that disagreed about any of it. So the host installs a lookup at
/// startup and the widgets call through it.
type Translator = fn(&str) -> String;

static TRANSLATOR: OnceLock<RwLock<Option<Translator>>> = OnceLock::new();

fn translator() -> &'static RwLock<Option<Translator>> {
    TRANSLATOR.get_or_init(|| RwLock::new(None))
}

/// Install the lookup this crate's widgets use for their own strings — the
/// month names in the calendar, the words on the pop-up buttons.
pub fn set_translator(f: Translator) {
    if let Ok(mut slot) = translator().write() {
        *slot = Some(f);
    }
}

/// A translated string, or the key itself when nothing is installed.
///
/// Returning the key rather than panicking keeps the toolkit usable without a
/// host — in a test, or in a tool that only wants the widgets — and an
/// untranslated key on screen is a visible, reportable fault rather than a
/// crash.
fn tr(key: &str) -> String {
    match translator().read().ok().and_then(|s| *s) {
        Some(f) => f(key),
        None => key.to_string(),
    }
}


use std::collections::HashMap;

use eframe::egui;
use limen_proto::date::{self, Date};

use serde::Deserialize;
use serde_json::Value;

// --------------------------------------------------------------------------- //
// Barracuda "EVE-Frontier" amber-HUD palette
//
// Mirrors the platform ui-kit design tokens (Libraries/ui-kit `src/tokens.ts` +
// `theme.css`): a warm amber accent over deep warm-black backgrounds, orange CTA,
// and warm brown-grey neutrals (not a cool slate). Keep these in sync with the
// kit so Limen and the web platform read as one product.
// --------------------------------------------------------------------------- //

// The toolkit is one crate but not one file. Each of these is a piece of it;
// the re-exports below keep `ui::thing` meaning what it has always meant, so
// splitting the file changed no call site anywhere else.
mod anim;
mod datefield;
mod filefield;
mod kit;
mod markdown;
mod menu;
mod overlay;
mod table;
mod theme;
pub mod toast;
mod typing;
mod view;
mod widgets;

pub use anim::*;
pub use datefield::*;
pub use filefield::*;
pub use kit::*;
pub use markdown::*;
pub use menu::*;
pub use overlay::*;
pub use table::*;
pub use theme::*;
pub use typing::*;
pub use view::*;
pub use widgets::*;

