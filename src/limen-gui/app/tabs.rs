//! Open tabs, and the screen each one is holding while another is shown.

use super::*;

/// An open tab. All tabs are closable.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Tab {
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
    pub(crate) fn title(&self) -> String {
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
pub(crate) fn swap_page(
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

/// Everything a module tab holds while it is not the one on screen.
///
/// The module page's state used to be the app's, so switching tabs threw it away
/// and rebuilt it — which meant a pop-up open in one tab could not survive a
/// glance at another. Each module tab keeps its own, and the app swaps the
/// active one in and out.
#[derive(Default)]
pub(crate) struct ModulePage {
    pub(crate) view: Option<ui::View>,
    pub(crate) view_error: Option<String>,
    pub(crate) modal_stack: Vec<ui::View>,
    pub(crate) modal_closing: Option<ui::View>,
    pub(crate) inputs: HashMap<String, String>,
    pub(crate) output: String,
    /// The scan (or whatever) this tab is waiting on, so its spinner survives a
    /// look at another tab.
    pub(crate) busy: bool,
    pub(crate) busy_action: Option<ui::Action>,
}

/// A detail tab's content: a module-returned [`ui::View`] opened from a row
/// action (e.g. "About device"), with its own inputs and load state.
#[derive(Default)]
pub(crate) struct DetailTab {
    pub(crate) title: String,
    pub(crate) view: Option<ui::View>,
    pub(crate) error: Option<String>,
    pub(crate) inputs: HashMap<String, String>,
    pub(crate) busy: bool,
}

#[cfg(test)]
mod page_tests {
    use super::*;

    fn page(marker: &str) -> ModulePage {
        let view: ui::View =
            serde_json::from_str(&format!(r#"{{"title":"{marker}","widgets":[]}}"#)).unwrap();
        let popup: ui::View = serde_json::from_str(
            r#"{"title":"Scan settings","modal":"loki.settings","widgets":[]}"#,
        )
        .unwrap();
        ModulePage {
            view: Some(view),
            modal_stack: vec![popup],
            inputs: [("target".to_string(), format!("/srv/{marker}"))]
                .into_iter()
                .collect(),
            busy: true,
            ..Default::default()
        }
    }

    fn title(p: &ModulePage) -> String {
        p.view.as_ref().map(|v| v.title.clone()).unwrap_or_default()
    }

    /// The point of the whole thing: what a tab was showing — including an open
    /// pop-up and a half-filled form — is still there when you come back.
    #[test]
    fn a_tab_keeps_its_screen_while_another_is_shown() {
        let mut stored = HashMap::new();

        // On "loki", with a settings pop-up open. Switch to "banlist".
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        assert!(onscreen.view.is_none(), "banlist has never been shown");
        assert!(onscreen.modal_stack.is_empty());

        // Come back. Everything is as it was left.
        let back = swap_page(onscreen, &mut stored, Some("banlist"), Some("loki"));
        assert_eq!(title(&back), "loki");
        assert_eq!(back.modal_stack.len(), 1, "the pop-up survived the trip");
        assert_eq!(back.modal_stack[0].modal.as_deref(), Some("loki.settings"));
        assert_eq!(back.inputs.get("target").unwrap(), "/srv/loki");
        assert!(back.busy, "and so did what it was waiting on");
    }

    /// A non-module tab starts empty. Without this the last module's view — and
    /// its pop-up — would show behind an About page.
    #[test]
    fn a_tab_that_is_not_a_module_shows_nothing() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), None);
        assert!(onscreen.view.is_none());
        assert!(onscreen.modal_stack.is_empty());
        assert!(!onscreen.busy);
        // ...and loki's screen was not lost, only set aside.
        assert_eq!(title(stored.get("loki").unwrap()), "loki");
    }

    /// Leaving a page that belongs to no tab drops it rather than filing it
    /// under someone else's name.
    #[test]
    fn a_page_with_no_tab_is_not_kept() {
        let mut stored = HashMap::new();
        let _ = swap_page(page("about"), &mut stored, None, Some("loki"));
        assert!(stored.is_empty(), "nothing was filed: {:?}", stored.keys());
    }

    /// Two tabs do not share a form. Their inputs are the most obvious thing to
    /// leak, since the fields they name are usually identical.
    #[test]
    fn two_module_tabs_do_not_share_their_inputs() {
        let mut stored = HashMap::new();
        let mut onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        onscreen
            .inputs
            .insert("target".to_string(), "/srv/banlist".to_string());

        let loki = swap_page(onscreen, &mut stored, Some("banlist"), Some("loki"));
        assert_eq!(loki.inputs.get("target").unwrap(), "/srv/loki");
        let banlist = swap_page(loki, &mut stored, Some("loki"), Some("banlist"));
        assert_eq!(banlist.inputs.get("target").unwrap(), "/srv/banlist");
    }

    /// Closing a tab forgets it: reopening the module starts fresh rather than
    /// resurrecting a pop-up from a tab the user closed.
    #[test]
    fn a_closed_tab_leaves_nothing_behind() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), None);
        assert!(stored.contains_key("loki"));

        stored.remove("loki"); // what close_tab does
        let reopened = swap_page(onscreen, &mut stored, None, Some("loki"));
        assert!(reopened.view.is_none());
        assert!(reopened.modal_stack.is_empty());
    }

    /// A reload restarts every module, so a stored screen describes a connection
    /// that no longer exists — and a stored pop-up would sit over a module that
    /// has forgotten it was ever open.
    #[test]
    fn a_reload_clears_what_every_tab_was_showing() {
        let mut stored = HashMap::new();
        let onscreen = swap_page(page("loki"), &mut stored, Some("loki"), Some("banlist"));
        let _ = swap_page(page("banlist"), &mut stored, Some("banlist"), None);
        assert_eq!(stored.len(), 2);

        stored.clear(); // what the reload does
        let after = swap_page(onscreen, &mut stored, None, Some("loki"));
        assert!(after.view.is_none(), "nothing survives a reload");
    }
}
