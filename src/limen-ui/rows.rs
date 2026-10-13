//! Which rows of a table are new, which have gone, and how far each is into
//! arriving or leaving.
//!
//! A module answers with a whole view every time, so a table that gains one row
//! arrives as a new list of rows rather than as an instruction to add one. Told
//! only that, the table could do nothing but redraw — which read as the page
//! reloading: every row appeared at once, the scroll jumped back to the top, and
//! the one row that had actually changed was the hardest thing on the screen to
//! find.
//!
//! So the table remembers what it drew last frame and works out the difference
//! itself. A row nobody has seen before grows into place, pushing what is below
//! it down; a row that has gone collapses and slides out of the way. Everything
//! else holds still, which is the point: motion means something changed there.

use crate::*;

/// How long a row takes to arrive, and to go.
///
/// Leaving is quicker than arriving: an arrival is news and is worth watching,
/// while a departure is something the person just asked for and already knows
/// about — a slow one only delays the list settling.
pub(crate) const IN: f64 = 0.22;
pub(crate) const OUT: f64 = 0.15;

/// How many rows a table may have before it is left alone.
///
/// Remembering last frame's rows costs a copy of them, and the motion itself is
/// only legible on a list somebody can take in. A thousand-row result set gets
/// the plain entrance it always had.
const TRACK_MAX: usize = 200;

/// More rows than this arriving or leaving at once is not an edit, it is a
/// different table — a search that has finished, a page turned. Those replay the
/// table's own entrance instead, because a hundred rows each sliding in on their
/// own clock is noise, not information.
pub(crate) const BULK: usize = 12;

/// One row as it should be drawn this frame.
pub(crate) struct Row {
    /// Its place in the live rows, or `None` for a row that has gone and is
    /// still collapsing — a ghost, which carries its own cells because the
    /// module no longer sends them.
    pub at: Option<usize>,
    /// What a ghost says. Live rows read their cells from the table.
    pub ghost: Option<Vec<String>>,
    /// 0 → 1 as it arrives, 1 → 0 as it goes, and 1 once it is simply there.
    /// Multiplied into the row's height, so the rows after it move by the same
    /// amount at the same time, and into its opacity.
    pub grow: f32,
    /// Which way it is going, for the sideways part of the motion.
    pub leaving: bool,
}

impl Row {
    /// Whether this row is where it belongs, and so should be drawn at full
    /// height on the table's own entrance timing.
    pub fn settled(&self) -> bool {
        self.grow >= 1.0 && !self.leaving
    }
}

/// The rows to draw, and when the table last began an entrance of its own.
pub(crate) struct Lane {
    pub rows: Vec<Row>,
    /// The clock the staggered entrance runs from — restarted when the table is
    /// replaced wholesale, left alone when it is merely edited.
    pub shown: f64,
}

/// What the table drew last frame.
#[derive(Clone, Default)]
struct Roster {
    /// Last frame's rows in order, by key, with their cells — the cells so a
    /// row that disappears can still be drawn while it collapses.
    last: Vec<(String, Vec<String>)>,
    /// When each key was first seen.
    seen: HashMap<String, f64>,
    /// On the way out: key, cells, the place it held, and when it left.
    going: Vec<(String, Vec<String>, usize, f64)>,
    /// When this table last began its staggered entrance.
    shown: f64,
}

/// Name a row so it can be recognised next frame.
///
/// The module's own row id when it gave one — that is what a row *is* as far as
/// the module is concerned, and it survives the row's text changing. Otherwise
/// the text itself, which is all there is to go on. Repeats get their count
/// appended, so two identical rows are two rows rather than one seen twice.
fn keys(rows: &[Vec<String>], row_ids: &[String]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    rows.iter()
        .enumerate()
        .map(|(r, row)| {
            let base = match row_ids.get(r) {
                Some(id) if !id.is_empty() => id.clone(),
                _ => row.join("\u{1}"),
            };
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            format!("{base}\u{2}{n}")
        })
        .collect()
}

/// Work out what has changed since last frame, and hand back the rows to draw.
///
/// Also writes next frame's memory, so this is called once per table per frame,
/// before anything is drawn.
pub(crate) fn lane(ui: &egui::Ui, id: egui::Id, rows: &[Vec<String>], row_ids: &[String]) -> Lane {
    let now = ui.input(|i| i.time);
    let key = id.with("limen_row_lane");
    let was: Option<Roster> = ui.data(|d| d.get_temp(key));

    // Too long to follow, and too long to copy every frame.
    if rows.len() > TRACK_MAX || !animations_enabled() {
        let shown = was.as_ref().map(|r| r.shown).unwrap_or(now);
        ui.data_mut(|d| {
            d.insert_temp(
                key,
                Roster {
                    last: Vec::new(),
                    seen: HashMap::new(),
                    going: Vec::new(),
                    shown,
                },
            )
        });
        return Lane {
            rows: still(rows.len()),
            shown,
        };
    }

    let now_keys = keys(rows, row_ids);
    let first_sight = was.is_none();
    let mut roster = was.unwrap_or(Roster {
        shown: now,
        ..Default::default()
    });

    let arrived: Vec<usize> = (0..rows.len())
        .filter(|&r| !roster.seen.contains_key(&now_keys[r]))
        .collect();
    let departed: Vec<(usize, &(String, Vec<String>))> = roster
        .last
        .iter()
        .enumerate()
        .filter(|(_, (k, _))| !now_keys.contains(k))
        .collect();

    // A different table rather than an edited one: start again, and let the
    // table's own entrance do the talking.
    if first_sight || arrived.len() > BULK || departed.len() > BULK {
        let shown = if first_sight { roster.shown } else { now };
        ui.data_mut(|d| {
            d.insert_temp(
                key,
                Roster {
                    last: snapshot(rows, &now_keys),
                    // Long enough ago to have finished arriving: these rows are
                    // the table's entrance, and must not then animate again
                    // individually on the frame after it.
                    seen: now_keys.iter().map(|k| (k.clone(), now - IN)).collect(),
                    going: Vec::new(),
                    shown,
                },
            )
        });
        return Lane {
            rows: still(rows.len()),
            shown,
        };
    }

    // Newly gone rows join the ones already collapsing, at the place they held.
    for (at, (k, cells)) in departed {
        roster.going.push((k.clone(), cells.clone(), at, now));
    }
    // And rows that came back while they were still collapsing stop going.
    roster.going.retain(|(k, _, _, left)| {
        !now_keys.contains(k) && (now - *left) < OUT
    });

    for (r, k) in now_keys.iter().enumerate() {
        let _ = r;
        roster.seen.entry(k.clone()).or_insert(now);
    }
    // A key nobody is holding any more is forgotten, or the roster grows for
    // the life of the window.
    roster
        .seen
        .retain(|k, _| now_keys.contains(k) || roster.going.iter().any(|(g, _, _, _)| g == k));

    // The live rows, each as far into its arrival as its own clock says.
    let mut out: Vec<Row> = now_keys
        .iter()
        .enumerate()
        .map(|(r, k)| {
            let since = now - roster.seen.get(k).copied().unwrap_or(now);
            let grow = ease_out((since / IN).clamp(0.0, 1.0) as f32);
            Row {
                at: Some(r),
                ghost: None,
                grow,
                leaving: false,
            }
        })
        .collect();

    // The ghosts, put back where they were so the gap closes from the right
    // place. Later indices first, so inserting one does not move the next.
    let mut ghosts: Vec<&(String, Vec<String>, usize, f64)> = roster.going.iter().collect();
    ghosts.sort_by_key(|(_, _, at, _)| std::cmp::Reverse(*at));
    for (_, cells, at, left) in ghosts {
        let grow = 1.0 - smoothstep(((now - *left) / OUT).clamp(0.0, 1.0) as f32);
        let at = (*at).min(out.len());
        out.insert(
            at,
            Row {
                at: None,
                ghost: Some(cells.clone()),
                grow,
                leaving: true,
            },
        );
    }

    if out.iter().any(|row| !row.settled()) {
        // Nothing else is driving the clock between frames.
        ui.ctx().request_repaint();
    }

    let shown = roster.shown;
    // Next frame's memory. The cells are copied only when the set of rows has
    // actually changed: a table sitting still should not allocate its own
    // contents sixty times a second.
    let keep_cells = roster.last.len() != now_keys.len()
        || roster.last.iter().zip(&now_keys).any(|((k, _), n)| k != n);
    if keep_cells {
        roster.last = snapshot(rows, &now_keys);
    }
    ui.data_mut(|d| d.insert_temp(key, roster));
    Lane { rows: out, shown }
}

/// Rows that are simply where they are — the answer for a table that is not
/// being followed, or has just been replaced.
fn still(n: usize) -> Vec<Row> {
    (0..n)
        .map(|r| Row {
            at: Some(r),
            ghost: None,
            grow: 1.0,
            leaving: false,
        })
        .collect()
}

fn snapshot(rows: &[Vec<String>], keys: &[String]) -> Vec<(String, Vec<String>)> {
    keys.iter().cloned().zip(rows.iter().cloned()).collect()
}
