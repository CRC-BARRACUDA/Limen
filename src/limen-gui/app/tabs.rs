//! Open tabs, and the screen each one is holding while another is shown.

use super::*;

/// An open tab. All tabs are closable.
#[derive(Clone, PartialEq, Debug)]
pub enum Tab {
    About,
    Modules,
    Module(String),
    Settings,
    Developer,
    Update,
    /// A device/detail view opened from a row action (keyed into `detail_tabs`).
    Detail {
        id: u64,
    },
}

impl Tab {
    pub fn title(&self) -> String {
        match self {
            Tab::About => i18n::t("tab.about"),
            Tab::Modules => i18n::t("tab.modules"),
            Tab::Module(n) => n.clone(),
            Tab::Settings => i18n::t("tab.settings"),
            Tab::Developer => i18n::t("tab.developer"),
            Tab::Update => i18n::t("tab.update"),
            // The real label comes from the stored view's title (looked up in the
            // tab bar); this is only a fallback.
            Tab::Detail { .. } => i18n::t("tab.details"),
        }
    }
}

/// Move the page on screen aside and bring another forward.
///
/// The whole rule of per-tab state in one place: what a tab was showing is kept
/// under its own name, and what the next tab was showing is handed back. A `from`
/// of `None` means the page being left belongs to no module tab and is simply
/// dropped; a `to` of `None` means the tab being opened is not a module tab and
/// starts empty — otherwise the last module's view would show behind an About
/// page.
/// Whether a restored tab picks its chain back up.
///
/// Resuming exists for a *loop* — a scan being polled, an install stepping
/// through — which a restored snapshot has stopped running. A step that opens a
/// tab is not a loop but a handover, and it has already happened: run again on
/// every return to the tab, it would open another copy of the same report each
/// time the user came back.
pub fn resumes(auto: &ui::AutoAction) -> bool {
    !auto.open_in_tab
}

/// Where the answer to a click belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A tab of its own — the click asked for one.
    NewTab,
    /// The tab the click was made in.
    SameTab(u64),
    /// The module's own screen.
    Screen,
}

/// Which of those, for a click made on `active`.
///
/// A view opened in a tab is interactive like any other — a row's details, a
/// form's buttons — and a module answers a click with the next screen. Sent to
/// the module's own tab, that screen arrived somewhere behind the user, while
/// the tab they were looking at went on showing the thing they had just clicked
/// out of.
pub fn answer_goes_to(active: Option<&Tab>, open_in_tab: bool) -> Answer {
    match (open_in_tab, active) {
        (true, _) => Answer::NewTab,
        (false, Some(Tab::Detail { id })) => Answer::SameTab(*id),
        (false, _) => Answer::Screen,
    }
}

pub fn swap_page(
    current: ModulePage,
    stored: &mut HashMap<String, ModulePage>,
    from: Option<&str>,
    to: Option<&str>,
) -> ModulePage {
    if let Some(name) = from {
        stored.insert(name.to_string(), current);
    }
    to.and_then(|name| stored.remove(name)).unwrap_or_default()
}

/// Why a module's tab shows a wall instead of a screen.
///
/// Two different failures that leave the user in the same place: there is
/// nothing here to use, and the only thing to do is read what happened. They
/// are told apart because the answer differs — one is a bug in the module, the
/// other is usually something missing on this machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Inactive {
    /// It panicked part-way through a call and the SDK caught the unwind. It is
    /// still loaded, but its state is whatever the panic left behind.
    Panicked(String),
    /// It never started: a manifest it could not read, a library that would not
    /// load, a runtime that is not installed. There was never a connection.
    FailedStart(String),
}

/// Why this module's tab has nothing to show, given what the host reported.
///
/// `Host::failed_modules` records every module that would not start, keyed by
/// name; the tab and the manager card both read their answer from it.
pub fn inactive_for(failed: &HashMap<String, String>, module: &str) -> Option<Inactive> {
    failed.get(module).map(|e| Inactive::FailedStart(e.clone()))
}

impl Inactive {
    /// The line above the detail in the pop-up. The screen itself says the same
    /// thing either way — what differs is the explanation behind the button.
    pub fn help_key(&self) -> &'static str {
        match self {
            Inactive::Panicked(_) => "module.panic_help",
            Inactive::FailedStart(_) => "module.failed_start",
        }
    }

    /// What the module (or the loader) actually said.
    pub fn detail(&self) -> &str {
        match self {
            Inactive::Panicked(d) | Inactive::FailedStart(d) => d,
        }
    }
}

/// Everything a module tab holds while it is not the one on screen.
///
/// The module page's state used to be the app's, so switching tabs threw it away
/// and rebuilt it — which meant a pop-up open in one tab could not survive a
/// glance at another. Each module tab keeps its own, and the app swaps the
/// active one in and out.
#[derive(Default)]
pub struct ModulePage {
    pub view: Option<ui::View>,
    pub view_error: Option<String>,
    /// The module cannot be talked to, and why. The tab shows this in place of
    /// a screen and stops calling it.
    pub inactive: Option<Inactive>,
    pub modal_stack: Vec<ui::View>,
    pub modal_closing: Option<ui::View>,
    pub inputs: HashMap<String, String>,
    pub output: String,
    /// The scan (or whatever) this tab is waiting on, so its spinner survives a
    /// look at another tab.
    pub busy: bool,
    pub busy_action: Option<ui::Action>,
}

/// A detail tab's content: a module-returned [`ui::View`] opened from a row
/// action (e.g. "About device"), with its own inputs and load state.
#[derive(Default)]
pub struct DetailTab {
    pub title: String,
    pub view: Option<ui::View>,
    pub error: Option<String>,
    pub inputs: HashMap<String, String>,
    pub busy: bool,
}
