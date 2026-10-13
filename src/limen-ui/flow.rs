//! Which widgets of a view are new, which have gone, and where the rest of them
//! are being pushed to.
//!
//! The same problem [`crate::rows`] solves for the rows of one table, for the
//! widgets of a whole screen. A module answers with the entire view every time,
//! so a screen that has gained a line is handed over as a different screen —
//! and redrawing it is what read as the page reloading: everything flickered,
//! and the one thing that had actually changed was the hardest to spot.
//!
//! So the renderer works out the difference itself. A widget nobody has seen
//! before opens from nothing, pushing what is below it down as it grows; one
//! that has gone closes the same way, and everything else simply travels with
//! the gap. Nothing that stayed is redrawn as if it were new.

use crate::rows::{IN, OUT};
use crate::view::Widget;
use crate::*;

/// What a widget is, for the purpose of recognising it next frame.
///
/// A field is its id — that is what the module calls it, and it survives the
/// label and the contents changing. Everything else is its kind and what it
/// says, which is all there is to go on; a label whose words change is treated
/// as a new label, and that is the right answer for the only labels that do
/// change on a settled screen: the one-line notice saying what just happened.
///
/// The kind is part of every key, so a button and a label that read the same
/// are not mistaken for one another.
pub(crate) fn key(w: &Widget) -> String {
    match w {
        Widget::Text { id, .. }
        | Widget::Select { id, .. }
        | Widget::File { id, .. }
        | Widget::DateField { id, .. }
        | Widget::Checkbox { id, .. } => format!("field:{id}"),
        Widget::Button { text, .. } => format!("button:{text}"),
        Widget::Label { text, .. } => format!("label:{text}"),
        Widget::Step { label, .. } => format!("step:{label}"),
        Widget::Separator => "separator".into(),
        Widget::Spacer => "spacer".into(),
        // Named by what it holds: a header whose version has changed is not the
        // header it was, and the screen should not be told it is unchanged.
        Widget::Chrome { children } => format!(
            "chrome:{}",
            children.iter().map(key).collect::<Vec<_>>().join(",")
        ),
        // A container is named by what it holds, so a row that gains a button
        // is the same row rather than a new one.
        Widget::Row { children } => format!(
            "row:{}",
            children.iter().map(kind_of).collect::<Vec<_>>().join(",")
        ),
        // These are named by their columns and their title: the contents are
        // expected to change — that is what a table is for — and the table
        // animates its own rows.
        Widget::Table { columns, .. } => format!("table:{}", columns.join(",")),
        Widget::Chart { title, .. } => format!("chart:{title}"),
        Widget::Donut { title, .. } => format!("donut:{title}"),
        Widget::Diagram { title, .. } => format!("diagram:{title}"),
    }
}

/// The kind alone, for naming a container by its shape.
fn kind_of(w: &Widget) -> &'static str {
    match w {
        Widget::Text { .. } => "text",
        Widget::Select { .. } => "select",
        Widget::File { .. } => "file",
        Widget::DateField { .. } => "date",
        Widget::Checkbox { .. } => "checkbox",
        Widget::Button { .. } => "button",
        Widget::Label { .. } => "label",
        Widget::Step { .. } => "step",
        Widget::Separator => "separator",
        Widget::Spacer => "spacer",
        Widget::Chrome { .. } => "chrome",
        Widget::Row { .. } => "row",
        Widget::Table { .. } => "table",
        Widget::Chart { .. } => "chart",
        Widget::Donut { .. } => "donut",
        Widget::Diagram { .. } => "diagram",
    }
}

/// Repeats get their count appended, so two separators are two widgets rather
/// than one widget seen twice.
pub(crate) fn keys(widgets: &[Widget]) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    widgets
        .iter()
        .map(|w| {
            let base = key(w);
            let n = count.entry(base.clone()).or_insert(0);
            *n += 1;
            format!("{base}\u{2}{n}")
        })
        .collect()
}

/// More than this many widgets coming or going at once — and more coming and
/// going than staying — is another screen rather than an edited one. A screen
/// is a dozen widgets, so the number is small: three lines changing is an edit,
/// and eight is a different answer to a different question.
const BUSY: usize = 3;

/// One widget as it should be drawn this frame.
pub(crate) struct Step {
    /// Its place in the view, or `None` for one that has gone and is still
    /// closing — which carries the widget as it last was, because the module no
    /// longer sends it.
    pub at: Option<usize>,
    pub ghost: Option<Widget>,
    /// What egui should key this widget's own state by. The widget's place in
    /// the list will not do: inserting a line above a text box would hand that
    /// box the box-above's id, and with it the wrong cursor and the wrong
    /// scroll.
    pub token: u64,
    /// 0 → 1 as it opens, 1 → 0 as it closes, 1 once it is simply there.
    pub grow: f32,
    pub leaving: bool,
    /// How big it was last time it was drawn whole, if it has been. The size a
    /// widget wants is only known by drawing it, so the first frame of an
    /// arrival opens from nothing and measures as it goes. Both dimensions are
    /// kept because a widget in a row opens across while one on a screen opens
    /// down.
    pub full: Option<egui::Vec2>,
}

impl Step {
    pub fn settled(&self) -> bool {
        self.grow >= 1.0 && !self.leaving
    }
}

/// The widgets to draw, and when this screen last began an entrance of its own.
pub(crate) struct Flow {
    pub steps: Vec<Step>,
    pub shown: f64,
}

#[derive(Clone, Default)]
struct Roster {
    /// Last frame's widgets: the token each held, its key, and the widget
    /// itself — the widget so that one which disappears can still be drawn
    /// while it closes. Copied only when the list has actually changed.
    last: Vec<(u64, String)>,
    kept: Vec<Widget>,
    next: u64,
    seen: HashMap<u64, f64>,
    high: HashMap<u64, egui::Vec2>,
    /// Closing: token, the widget as it last was, the place it held, and when
    /// it went.
    going: Vec<(u64, Widget, usize, f64)>,
    shown: f64,
}

/// Whether two widgets of this kind, in the same gap, are the same widget
/// saying something else.
///
/// True of anything whose words are a reading: a label counting what has been
/// found, a step naming what it is doing, a table's title. Not true of a
/// button, whose words *are* its identity — a screen offering "Scan processes"
/// where it offered "Scan autoruns" is a different offer, and it should be seen
/// to change rather than quietly relabelled. Not true of a field either: a field
/// is its id, so a different key is a different field.
fn pairable(kind: &str) -> bool {
    matches!(
        kind,
        "label"
            | "step"
            | "row"
            | "table"
            | "chart"
            | "donut"
            | "diagram"
            | "chrome"
            // A rule and a gap say nothing at all, so one can always stand in
            // for another: a hairline that collapsed and reopened because the
            // list around it moved would be a flicker for no reason.
            | "separator"
            | "spacer"
    )
}

/// Where in the old list each new widget came from.
///
/// Exact matches first, in order, as a longest common subsequence — so a widget
/// inserted in the middle is read as an insertion rather than as every widget
/// after it having changed. Then, within each gap the subsequence leaves, an old
/// and a new widget of the same kind are paired up: that is a label whose words
/// have changed, or a button whose count has, and neither is a new widget.
fn align(old: &[String], new: &[String]) -> Vec<Option<usize>> {
    // Longest common subsequence, by the usual table. Views are a dozen widgets
    // long, so the square is nothing.
    let (n, m) = (old.len(), new.len());
    let mut len = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i][j] = if old[i] == new[j] {
                len[i + 1][j + 1] + 1
            } else {
                len[i + 1][j].max(len[i][j + 1])
            };
        }
    }
    let mut from: Vec<Option<usize>> = vec![None; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            from[j] = Some(i);
            i += 1;
            j += 1;
        } else if len[i + 1][j] >= len[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    // The gaps between the matches, paired up by kind.
    // The kind alone: what comes before the name a key carries, and before the
    // count that tells two of a kind apart. A key with neither — a separator —
    // is all kind.
    let kind = |k: &str| {
        k.split([':', '\u{2}'])
            .next()
            .unwrap_or_default()
            .to_string()
    };
    let mut taken: Vec<bool> = (0..n).map(|i| from.contains(&Some(i))).collect();
    let mut last_match = -1i64;
    for j in 0..m {
        if let Some(i) = from[j] {
            last_match = i as i64;
            continue;
        }
        // The gap this new widget falls in: after the last matched old widget,
        // and before the next one.
        let next_match = (j + 1..m).find_map(|k| from[k]).unwrap_or(n);
        let here = kind(&new[j]);
        let found = ((last_match + 1) as usize..next_match)
            .find(|&i| !taken[i] && kind(&old[i]) == here && pairable(&here));
        if let Some(i) = found {
            taken[i] = true;
            from[j] = Some(i);
        }
    }
    from
}

/// Work out what has changed since last frame, and hand back the widgets to
/// draw. Writes next frame's memory, so it is called once per view per frame.
pub(crate) fn flow(ui: &egui::Ui, id: egui::Id, widgets: &[Widget]) -> Flow {
    let now = ui.input(|i| i.time);
    let slot = id.with("limen_view_flow");
    let was: Option<Roster> = ui.data(|d| d.get_temp(slot));
    let first_sight = was.is_none();
    let mut roster = was.unwrap_or(Roster {
        shown: now,
        ..Default::default()
    });

    let now_keys = keys(widgets);
    let old_keys: Vec<String> = roster.last.iter().map(|(_, k)| k.clone()).collect();
    let from = align(&old_keys, &now_keys);

    let arrived = from.iter().filter(|f| f.is_none()).count();
    let departed = old_keys.len() - from.iter().filter(|f| f.is_some()).count();
    // More changed than stayed: this is another screen rather than an edited
    // one, and it gets the entrance a screen gets — one staggered sweep, not a
    // dozen widgets each opening on its own clock.
    let matched = from.iter().filter(|f| f.is_some()).count();
    let wholesale = first_sight || arrived + departed > matched.max(BUSY);

    if !animations_enabled() || wholesale {
        let shown = if first_sight { roster.shown } else { now };
        let tokens: Vec<u64> = (0..widgets.len())
            .map(|i| roster.next + i as u64)
            .collect();
        let mut keep = Roster {
            last: tokens.iter().cloned().zip(now_keys).collect(),
            kept: widgets.to_vec(),
            next: roster.next + widgets.len() as u64,
            // Long enough ago to have finished arriving: these widgets *are*
            // the entrance, and must not animate again individually on the
            // frame after it.
            seen: tokens.iter().map(|t| (*t, now - IN)).collect(),
            high: HashMap::new(),
            going: Vec::new(),
            shown,
        };
        // Heights are worth keeping across a change of screen: a module that
        // flips between two views draws the same widgets again, and a remembered
        // height is one frame of settling saved.
        keep.high = roster.high;
        ui.data_mut(|d| d.insert_temp(slot, keep));
        return Flow {
            steps: tokens
                .iter()
                .enumerate()
                .map(|(i, token)| Step {
                    at: Some(i),
                    ghost: None,
                    token: *token,
                    grow: 1.0,
                    leaving: false,
                    full: None,
                })
                .collect(),
            shown,
        };
    }

    // Tokens: carried over for a widget that was already here, fresh for one
    // that was not.
    let mut tokens = Vec::with_capacity(widgets.len());
    for f in &from {
        match f {
            Some(i) => tokens.push(roster.last[*i].0),
            None => {
                roster.next += 1;
                let token = roster.next;
                roster.seen.insert(token, now);
                tokens.push(token);
            }
        }
    }

    // Whatever is no longer here starts closing, from where it was.
    let staying: Vec<u64> = tokens.clone();
    for (i, (token, _)) in roster.last.iter().enumerate() {
        if !staying.contains(token)
            && !roster.going.iter().any(|(t, _, _, _)| t == token)
            && let Some(w) = roster.kept.get(i)
        {
            roster.going.push((*token, w.clone(), i, now));
        }
    }
    roster
        .going
        .retain(|(t, _, _, left)| !staying.contains(t) && (now - *left) < OUT);

    let mut steps: Vec<Step> = tokens
        .iter()
        .enumerate()
        .map(|(i, token)| {
            let since = now - roster.seen.get(token).copied().unwrap_or(now);
            Step {
                at: Some(i),
                ghost: None,
                token: *token,
                grow: ease_out((since / IN).clamp(0.0, 1.0) as f32),
                leaving: false,
                full: roster.high.get(token).copied(),
            }
        })
        .collect();

    // The ones on their way out, put back where they were. Later places first,
    // so inserting one does not move the next.
    let mut ghosts: Vec<&(u64, Widget, usize, f64)> = roster.going.iter().collect();
    ghosts.sort_by_key(|(_, _, at, _)| std::cmp::Reverse(*at));
    for (token, w, at, left) in ghosts {
        let at = (*at).min(steps.len());
        steps.insert(
            at,
            Step {
                at: None,
                ghost: Some(w.clone()),
                token: *token,
                grow: 1.0 - smoothstep(((now - *left) / OUT).clamp(0.0, 1.0) as f32),
                leaving: true,
                full: roster.high.get(token).copied(),
            },
        );
    }

    if steps.iter().any(|s| !s.settled()) {
        ui.ctx().request_repaint();
    }

    // Next frame's memory. The widgets themselves are copied only when the list
    // has changed — a screen standing still must not clone its own contents
    // sixty times a second.
    let changed = roster.last.len() != tokens.len()
        || roster
            .last
            .iter()
            .zip(&tokens)
            .any(|((t, _), token)| t != token);
    let mut seen = roster.seen;
    seen.retain(|t, _| tokens.contains(t) || roster.going.iter().any(|(g, _, _, _)| g == t));
    let shown = roster.shown;
    if changed {
        roster.kept = widgets.to_vec();
    }
    let keep = Roster {
        last: tokens.iter().cloned().zip(keys(widgets)).collect(),
        kept: roster.kept,
        next: roster.next,
        seen,
        high: roster.high,
        going: roster.going,
        shown,
    };
    ui.data_mut(|d| d.insert_temp(slot, keep));
    Flow { steps, shown }
}

/// Remember how big a widget was when drawn whole, for the next arrival or
/// departure to open and close against.
pub(crate) fn remember(ui: &egui::Ui, id: egui::Id, token: u64, size: egui::Vec2) {
    let slot = id.with("limen_view_flow");
    ui.data_mut(|d| {
        if let Some(mut r) = d.get_temp::<Roster>(slot) {
            if r.high.get(&token).is_some_and(|h| (*h - size).length() < 0.5) {
                return;
            }
            r.high.insert(token, size);
            d.insert_temp(slot, r);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::align;

    fn keys(ks: &[&str]) -> Vec<String> {
        ks.iter().map(|k| k.to_string()).collect()
    }

    /// A widget inserted in the middle is an insertion — not every widget after
    /// it having changed, which is what reading the list off by one would make
    /// of it.
    #[test]
    fn one_widget_inserted_leaves_the_others_where_they_were() {
        let old = keys(&["label:Where\u{2}1", "field:paths\u{2}1", "button:Search\u{2}1"]);
        let new = keys(&[
            "label:Where\u{2}1",
            "field:paths\u{2}1",
            "separator\u{2}1",
            "button:Search\u{2}1",
        ]);
        assert_eq!(align(&old, &new), vec![Some(0), Some(1), None, Some(2)]);
    }

    /// A label whose words change is the same label saying something else: it is
    /// paired with the one it replaced, so it does not close and reopen on every
    /// tick of a progress line.
    #[test]
    fn a_label_that_changes_its_words_is_still_that_label() {
        let old = keys(&["step:Looking\u{2}1", "label:3 found\u{2}1", "button:Stop\u{2}1"]);
        let new = keys(&["step:Looking\u{2}1", "label:9 found\u{2}1", "button:Stop\u{2}1"]);
        assert_eq!(align(&old, &new), vec![Some(0), Some(1), Some(2)]);
    }

    /// But a label that appears where there was no label is new, even when
    /// something else changed in the same gap.
    #[test]
    fn a_label_appearing_beside_a_changed_one_is_new() {
        let old = keys(&["label:Where\u{2}1", "button:Add\u{2}1"]);
        let new = keys(&["label:Added /var/log\u{2}1", "label:Where\u{2}1", "button:Add\u{2}1"]);
        // The notice is new; the two that were already there are matched, and
        // nothing is paired across them.
        assert_eq!(align(&old, &new), vec![None, Some(0), Some(1)]);
    }

    /// A button is what it says: a screen offering the other scan is a
    /// different offer, and the old button goes while the new one arrives
    /// rather than being quietly relabelled.
    #[test]
    fn a_button_whose_words_change_is_not_the_old_button() {
        let old = keys(&["label:Ready\u{2}1", "button:Scan processes\u{2}1"]);
        let new = keys(&["label:Ready\u{2}1", "button:Scan autoruns\u{2}1"]);
        assert_eq!(align(&old, &new), vec![Some(0), None]);
    }

    /// Nothing in common is nothing matched — which is what tells the renderer
    /// it is looking at another screen rather than an edited one.
    #[test]
    fn a_different_screen_matches_nothing() {
        let old = keys(&["label:A\u{2}1", "field:a\u{2}1"]);
        let new = keys(&["table:Name,Path\u{2}1", "button:Back\u{2}1"]);
        assert_eq!(align(&old, &new), vec![None, None]);
    }

    /// Two widgets that read the same are two widgets: the repeat count is part
    /// of the key, so removing the first does not silently promote the second.
    #[test]
    fn repeats_are_told_apart() {
        let old = keys(&["separator\u{2}1", "label:A\u{2}1", "separator\u{2}2"]);
        let new = keys(&["label:A\u{2}1", "separator\u{2}1"]);
        // The label is matched outright; the one separator left is paired with
        // the one that is still available *after* it, not with the one before.
        assert_eq!(align(&old, &new), vec![Some(1), Some(2)]);
    }
}
