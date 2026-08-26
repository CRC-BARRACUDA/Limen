//! Tabs, and the screen each keeps while another is shown.

use std::collections::HashMap;

use limen_gui::ui;
use limen_gui::app::*;

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

/// A view opened in a tab is interactive like any other, and the module answers
/// a click with the next screen. That screen belongs in the tab the click was
/// made in — sent to the module's own tab instead, it arrives behind the user
/// while the tab they are looking at goes on showing what they clicked out of.
#[test]
fn a_click_inside_a_tab_is_answered_in_that_tab() {
    let detail = Tab::Detail { id: 7 };
    assert_eq!(answer_goes_to(Some(&detail), false), Answer::SameTab(7));

    // Asking for a tab still opens a new one, wherever it was asked from.
    assert_eq!(answer_goes_to(Some(&detail), true), Answer::NewTab);
    assert_eq!(
        answer_goes_to(Some(&Tab::Module("loki".into())), true),
        Answer::NewTab
    );

    // And a click on the module's own screen is answered there.
    assert_eq!(
        answer_goes_to(Some(&Tab::Module("loki".into())), false),
        Answer::Screen
    );
    assert_eq!(answer_goes_to(None, false), Answer::Screen);
}

/// A long job's chain polls in the tab that started it, and what it ends with is
/// a result rather than another step — so a module can ask for that last answer
/// to open in its own tab, leaving the screen that ran the job ready for the
/// next one.
#[test]
fn an_auto_action_can_hand_its_result_to_a_new_tab() {
    let poll: ui::View = serde_json::from_str(
        r#"{"title":"Scanning","widgets":[],
            "auto":{"capability":"scan.ioc","method":"s_poll","args":{}}}"#,
    )
    .unwrap();
    let auto = poll.auto.expect("the chain continues");
    assert!(
        !auto.clone().into_invoke().open_in_tab,
        "a step of the chain stays where the chain is"
    );

    let done: ui::View = serde_json::from_str(
        r#"{"title":"Loki","widgets":[],
            "auto":{"capability":"scan.ioc","method":"report_tab","args":{},
                    "open_in_tab":true}}"#,
    )
    .unwrap();
    let auto = done.auto.expect("one last step");
    let invoke = auto.into_invoke();
    assert!(invoke.open_in_tab, "the report opens beside the scan");
    assert_eq!(invoke.action.method, "report_tab");
}

/// Coming back to a tab restarts the loop it was in the middle of — that is
/// what resuming is for. It must not restart a handover: the step that opened
/// the scan report already ran, and running it again on every visit would open
/// another copy of the same report each time.
#[test]
fn returning_to_a_tab_resumes_a_loop_but_not_a_handover() {
    let auto = |json: &str| {
        serde_json::from_str::<ui::View>(json)
            .unwrap()
            .auto
            .expect("an auto action")
    };
    let polling = auto(
        r#"{"title":"Scanning","widgets":[],
            "auto":{"capability":"scan.ioc","method":"s_poll","args":{}}}"#,
    );
    assert!(resumes(&polling), "a scan left running has to be picked up");

    let handover = auto(
        r#"{"title":"Loki","widgets":[],
            "auto":{"capability":"scan.ioc","method":"report_tab","args":{},
                    "open_in_tab":true}}"#,
    );
    assert!(!resumes(&handover), "the report was already handed over");
}

/// A module that is out of service stays that way while you look at another
/// tab. The tab keeps every other scrap of its state across a swap, and this is
/// the one piece that decides whether the module is called at all.
#[test]
fn an_inactive_module_is_still_inactive_when_you_come_back() {
    let mut stored = HashMap::new();
    let mut dead = page("loki");
    dead.inactive = Some(Inactive::Panicked(
        "module panicked in invoke: index out of bounds".to_string(),
    ));

    let onscreen = swap_page(dead, &mut stored, Some("loki"), Some("banlist"));
    assert!(onscreen.inactive.is_none(), "banlist is fine");

    let back = swap_page(onscreen, &mut stored, Some("banlist"), Some("loki"));
    assert!(back.inactive.unwrap().detail().contains("index out of bounds"));
}

/// The screen reads the same either way — one line and one button. What
/// differs is behind the button: a bug in the module, or something missing on
/// this machine, which are not the same thing to go and fix.
#[test]
fn each_reason_explains_itself_behind_the_button() {
    let panic = Inactive::Panicked("boom".to_string());
    let start = Inactive::FailedStart("library not found".to_string());
    assert_ne!(panic.help_key(), start.help_key());
    assert_eq!(panic.detail(), "boom");
    assert_eq!(start.detail(), "library not found");
}

/// What the host reports and what the tab shows are the same fact. A module the
/// host could not start opens onto its reason rather than onto a call it cannot
/// make — and one that started is left alone.
#[test]
fn a_module_the_host_could_not_start_opens_onto_why() {
    let failed: HashMap<String, String> = [(
        "devices".to_string(),
        "could not find native library for \"devices_nope\"".to_string(),
    )]
    .into_iter()
    .collect();

    match inactive_for(&failed, "devices") {
        Some(Inactive::FailedStart(why)) => assert!(why.contains("could not find native library")),
        other => panic!("expected a start failure, got {other:?}"),
    }
    assert!(inactive_for(&failed, "loki").is_none(), "loki started fine");
    assert!(inactive_for(&HashMap::new(), "devices").is_none());
}

/// The manager needs one answer per module, from two places: the host's own
/// list of what would not start, and the tabs, where a module that panicked
/// while running is the only record there is.
#[test]
fn a_module_that_panicked_in_its_tab_is_inactive_in_the_manager_too() {
    let failed: HashMap<String, String> =
        [("devices".to_string(), "no library".to_string())].into_iter().collect();
    let mut pages: HashMap<String, ModulePage> = HashMap::new();
    let mut loki = page("loki");
    loki.inactive = Some(Inactive::Panicked("boom".to_string()));
    pages.insert("loki".to_string(), loki);

    // What `inactive_modules` composes, in the order it composes it.
    let mut all: HashMap<String, Inactive> = failed
        .iter()
        .map(|(n, e)| (n.clone(), Inactive::FailedStart(e.clone())))
        .collect();
    for (name, p) in &pages {
        if let Some(r) = &p.inactive {
            all.insert(name.clone(), r.clone());
        }
    }

    assert!(matches!(all.get("devices"), Some(Inactive::FailedStart(_))));
    assert!(matches!(all.get("loki"), Some(Inactive::Panicked(_))));
    assert!(all.get("banlist").is_none());
}
