//! The date field: its calendar, its clock, and how both move.

use limen_proto::date;
use eframe::egui;
use limen_ui::*;
/// A popup lives in an `Area`, which has no width of its own — anything
/// that divides `available_width` there spreads across the screen. Every
/// mode of the calendar is laid out to one pinned width instead.
#[test]
fn every_calendar_mode_fits_the_same_width() {
let gap = 2.0;
let days = 7.0 * CELL.x + 6.0 * gap;
assert_eq!(days, GRID_W, "the day grid defines the width");
let months = 3.0 * MONTH_CELL.x + 2.0 * gap;
let years = 4.0 * YEAR_CELL.x + 3.0 * gap;
assert!(
    (months - GRID_W).abs() < 0.01,
    "months are {months} wide against a grid of {GRID_W}"
);
assert!(
    (years - GRID_W).abs() < 0.01,
    "years are {years} wide against a grid of {GRID_W}"
);
// Twelve years is three rows of four, and twelve months three of four.
assert!(YEARS_SHOWN.is_multiple_of(4));
}
/// The entrance staggers across the grid and settles at one. A slot that
/// never reaches 1 leaves a cell permanently half-faded; one that starts at
/// 1 is not an entrance at all.
#[test]
fn the_calendar_fills_in_across_itself_and_settles() {
let slot = |open_t: f32, i: usize, n: usize| {
    ((open_t * 1.6) - i as f32 * (0.5 / n.max(1) as f32)).clamp(0.0, 1.0)
};
// Closed: nothing showing.
for i in 0..42 {
    assert_eq!(slot(0.0, i, 42), 0.0);
}
// Fully open: everything, including the last cell of the longest month.
for i in 0..42 {
    assert_eq!(slot(1.0, i, 42), 1.0, "cell {i} never finishes arriving");
}
// Part way: earlier cells are ahead of later ones, never behind.
let mid: Vec<f32> = (0..42).map(|i| slot(0.35, i, 42)).collect();
assert!(mid.windows(2).all(|w| w[0] >= w[1]), "{mid:?}");
assert!(mid[0] > 0.0 && mid[41] < mid[0], "nothing is staggered");
// A single cell is a degenerate grid, not a division by zero.
assert_eq!(slot(1.0, 0, 0), 1.0);
}
/// A month spans four, five or six weeks depending on how far into the week
/// it opens. The popup is sized from that, so a wrong row count clips the
/// last week off the calendar — or leaves a band of empty space under it.
#[test]
fn the_calendar_is_as_tall_as_the_month_it_shows() {
let rows = |y, m| ((calendar_height(LEVEL_DAYS, y, m) - 18.0 - 2.0 + 2.0) / (CELL.y + 2.0)).round();

// February 2021 opened on a Monday and has 28 days: four rows exactly,
// the shortest a month can be.
assert_eq!(date::weekday(2021, 2, 1), 0);
assert_eq!(rows(2021, 2), 4.0);

// August 2026 opens on a Saturday and has 31: six rows, the longest.
assert_eq!(date::weekday(2026, 8, 1), 5);
assert_eq!(rows(2026, 8), 6.0);

// February never needs six, whatever it does: at 29 days the worst
// case is opening on a Sunday, and 6 + 29 is exactly five weeks.
assert_eq!(date::days_in_month(2032, 2), 29);
assert_eq!(date::weekday(2032, 2, 1), 6);
assert_eq!(rows(2032, 2), 5.0);
for y in 2000..2060 {
    assert!(rows(y, 2) <= 5.0, "February {y} claimed six rows");
}

// Every month of a decade lands in that range, and none is taller than
// the six-row case the popup is laid out for.
let tallest = calendar_height(LEVEL_DAYS, 2026, 8);
for y in 2020..2030 {
    for m in 1..=12 {
        let r = rows(y, m);
        assert!((4.0..=6.0).contains(&r), "{y}-{m} spans {r} rows");
        assert!(calendar_height(LEVEL_DAYS, y, m) <= tallest);
    }
}

// The pickers are fixed: switching to them is always the same size.
assert_eq!(calendar_height(LEVEL_MONTHS, 2026, 8),
    calendar_height(LEVEL_MONTHS, 2021, 2));
assert_eq!(calendar_height(LEVEL_YEARS, 2026, 8),
    calendar_height(LEVEL_YEARS, 2021, 2));
}
/// Which way the calendar moves is the sort of thing that is silently
/// backwards: it still works, it just feels wrong, and nothing fails.
#[test]
fn the_calendar_moves_the_way_it_was_asked_to() {
// Sideways within the days: earlier comes from the left, later from the
// right.
assert_eq!(calendar_motion((LEVEL_DAYS, 2026, 4), (LEVEL_DAYS, 2026, 3)).0, -1.0);
assert_eq!(calendar_motion((LEVEL_DAYS, 2026, 4), (LEVEL_DAYS, 2026, 5)).0, 1.0);
// December to January is forwards, not a jump back through the month
// number — which is what comparing months alone would say.
assert_eq!(calendar_motion((LEVEL_DAYS, 2026, 12), (LEVEL_DAYS, 2027, 1)).0, 1.0);
assert_eq!(calendar_motion((LEVEL_DAYS, 2027, 1), (LEVEL_DAYS, 2026, 12)).0, -1.0);
// Paging the year picker moves the same way.
assert_eq!(calendar_motion((LEVEL_YEARS, 2026, 1), (LEVEL_YEARS, 2038, 1)).0, 1.0);
assert_eq!(calendar_motion((LEVEL_YEARS, 2026, 1), (LEVEL_YEARS, 2014, 1)).0, -1.0);

// Levels zoom rather than slide, and never both. Days is 1, the closest
// in: moving toward it grows into place, moving away settles back out.
for (from, to, closer) in [
    ((LEVEL_DAYS, 2026, 4), (LEVEL_MONTHS, 2026, 4), false),
    ((LEVEL_MONTHS, 2026, 4), (LEVEL_YEARS, 2026, 4), false),
    ((LEVEL_YEARS, 2026, 4), (LEVEL_MONTHS, 2026, 4), true),
    ((LEVEL_MONTHS, 2026, 4), (LEVEL_DAYS, 2026, 4), true),
] {
    let (dir, scale) = calendar_motion(from, to);
    assert_eq!(dir, 0.0, "{from:?} -> {to:?} slid as well as zoomed");
    if closer {
        assert!(scale < 1.0, "{from:?} -> {to:?} should grow into place");
    } else {
        assert!(scale > 1.0, "{from:?} -> {to:?} should settle back out");
    }
}

// Two levels at once is twice the movement of one — the distance is
// what makes a jump read as a jump.
let one = calendar_motion((LEVEL_MONTHS, 2026, 4), (LEVEL_DAYS, 2026, 4)).1;
let two = calendar_motion((LEVEL_YEARS, 2026, 4), (LEVEL_DAYS, 2026, 4)).1;
assert!(two < one, "years to days must move further than months to days");
assert!((1.0 - two) - 2.0 * (1.0 - one) < 0.001);
// ...and the same going outwards.
let out_one = calendar_motion((LEVEL_MONTHS, 2026, 4), (LEVEL_YEARS, 2026, 4)).1;
let out_two = calendar_motion((LEVEL_DAYS, 2026, 4), (LEVEL_YEARS, 2026, 4)).1;
assert!(out_two > out_one);

// Nothing changed: no movement at all, or a still calendar would twitch
// on every frame.
assert_eq!(
    calendar_motion((LEVEL_DAYS, 2026, 4), (LEVEL_DAYS, 2026, 4)),
    (0.0, 1.0)
);
}
/// The arrows sit on the edges of the header and the month/year pair in the
/// middle. Nested layouts shrink to their content, which is what left the
/// two arrows huddled around the middle with empty space beyond them.
#[test]
fn the_header_puts_its_arrows_on_the_edges() {
let (bar, gap, arrow) = (GRID_W, 4.0, 26.0);
let (mw, yw) = (90.0, 56.0);
let x = header_pair_x(bar, mw, yw, gap);

// The pair is centred: as much space to its left as to its right.
let right_edge = x + mw + gap + yw;
assert!((x - (bar - right_edge)).abs() < 0.01, "pair is not centred");
// And it clears both arrows, so nothing overlaps them.
assert!(x > arrow, "the month runs into the left arrow");
assert!(right_edge < bar - arrow, "the year runs into the right arrow");

// A longer month name keeps the pair centred rather than pushing the
// year off toward the arrow — "Листопад" against "May".
let long = header_pair_x(bar, 130.0, yw, gap);
assert!(long < x, "a longer name must start further left, not extend right");
let long_right = long + 130.0 + gap + yw;
assert!((long - (bar - long_right)).abs() < 0.01);
}
/// Paging slides in every mode, not only in the days: the month picker
/// steps a year at a time and the year picker a page at a time, and both
/// should move the way the arrow that drove them points.
#[test]
fn paging_slides_at_every_level() {
// Months: a year forwards and back.
assert_eq!(
    calendar_motion((LEVEL_MONTHS, 2027, 2), (LEVEL_MONTHS, 2028, 2)).0,
    1.0
);
assert_eq!(
    calendar_motion((LEVEL_MONTHS, 2027, 2), (LEVEL_MONTHS, 2026, 2)).0,
    -1.0
);
// Years: a page of twelve.
assert_eq!(
    calendar_motion((LEVEL_YEARS, 2027, 1), (LEVEL_YEARS, 2039, 1)).0,
    1.0
);
// And none of them zooms while it slides.
for (a, b) in [
    ((LEVEL_MONTHS, 2027, 2), (LEVEL_MONTHS, 2028, 2)),
    ((LEVEL_YEARS, 2027, 1), (LEVEL_YEARS, 2015, 1)),
    ((LEVEL_DAYS, 2027, 2), (LEVEL_DAYS, 2027, 3)),
] {
    assert_eq!(calendar_motion(a, b).1, 1.0, "{a:?} -> {b:?} zoomed too");
}
}
/// Every way of changing the level has to produce a movement — the header
/// labels as much as the grid. They are drawn at different points in the
/// frame, and it is easy for one of them to change the view after the last
/// thing that would have noticed.
#[test]
fn every_way_into_a_level_moves() {
// What the header labels do: toggle to that level, or back to days.
let month_button = |from: u8| {
    if from == LEVEL_MONTHS {
        LEVEL_DAYS
    } else {
        LEVEL_MONTHS
    }
};
let year_button = |from: u8| {
    if from == LEVEL_YEARS {
        LEVEL_DAYS
    } else {
        LEVEL_YEARS
    }
};
for from in [LEVEL_DAYS, LEVEL_MONTHS, LEVEL_YEARS] {
    for to in [month_button(from), year_button(from)] {
        if to == from {
            continue;
        }
        let (dir, scale) = calendar_motion((from, 2034, 5), (to, 2034, 5));
        assert_eq!(dir, 0.0);
        assert_ne!(
            scale, 1.0,
            "level {from} -> {to} arrives without a movement"
        );
        // Toward days grows in, away from days settles out.
        if to < from {
            assert!(scale < 1.0, "{from} -> {to} should grow into place");
        } else {
            assert!(scale > 1.0, "{from} -> {to} should settle back out");
        }
    }
}

// And what the grid does: a year picks its months, a month its days.
assert!(calendar_motion((LEVEL_YEARS, 2034, 5), (LEVEL_MONTHS, 2034, 5)).1 < 1.0);
assert!(calendar_motion((LEVEL_MONTHS, 2034, 5), (LEVEL_DAYS, 2034, 4)).1 < 1.0);
}
/// A field's pop-up has to sit under the veil a modal window lays down, or
/// it floats over the dimming — a bright, clickable calendar on top of "do
/// you really want to quit?".
#[test]
fn a_field_popup_is_dimmed_by_a_modal() {
// egui orders layers by this enum, low to high. The veil is Middle and
// the modal's own box is Foreground; a field's pop-up must be below
// both, and above the panels it covers.
assert!(FIELD_POPUP_ORDER < egui::Order::Middle, "not dimmed by the veil");
assert!(FIELD_POPUP_ORDER < egui::Order::Foreground, "over the modal box");
assert!(
    FIELD_POPUP_ORDER > egui::Order::Background,
    "would be hidden behind the form it belongs to"
);
}
/// The time picker is laid out on a fixed grid, not by letting each column
/// size itself to its caption — "Години" and "Хвилини" are different widths,
/// and content-sized columns drift apart until the colons sit between
/// nothing in particular.
#[test]
fn the_time_columns_are_even_and_fit() {
// The same numbers the picker draws with.
const COL: f32 = 66.0;
const SEP: f32 = 18.0;
let span = 3.0 * COL + 2.0 * SEP;
assert!(span <= TIME_W, "the row is wider than the window holding it");

let x0 = (TIME_W - span) * 0.5;
let left = |i: f32| x0 + i * (COL + SEP);
// Evenly pitched: every gap between columns is the same.
let pitch: Vec<f32> = (0..2).map(|i| left(i as f32 + 1.0) - left(i as f32)).collect();
assert!(pitch.windows(2).all(|w| (w[0] - w[1]).abs() < 0.001), "{pitch:?}");
// Centred: as much room to the left of the first column as to the right
// of the last.
let right_edge = left(2.0) + COL;
assert!((x0 - (TIME_W - right_edge)).abs() < 0.001, "the row is not centred");
// The box is centred in its column, and so is the caption — they share
// a centre line, which is the thing that was visibly wrong when the box
// was left-aligned and the caption was not.
const BOX_W: f32 = 48.0;
for i in 0..3 {
    let centre = left(i as f32) + COL * 0.5;
    let box_l = centre - BOX_W * 0.5;
    assert!((box_l + BOX_W * 0.5 - centre).abs() < 0.001);
    assert!(box_l > left(i as f32), "the box overflows its column");
}

// Each colon sits midway between its neighbours, not against either.
for i in 0..2 {
    let colon = left(i as f32) + COL + SEP * 0.5;
    let gap_l = colon - (left(i as f32) + COL);
    let gap_r = left(i as f32 + 1.0) - colon;
    assert!((gap_l - gap_r).abs() < 0.001, "colon {i} sits crooked");
}
}
