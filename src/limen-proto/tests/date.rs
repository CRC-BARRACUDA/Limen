//! Dates, without a date library.

use limen_proto::date::*;

/// Round-tripping every day for four centuries covers every leap rule there
/// is: the ordinary one, the hundred-year exception, and the four-hundred
/// year exception to that.
#[test]
fn every_day_for_four_centuries_survives_the_round_trip() {
    let (start, end) = (days_from_civil(1800, 1, 1), days_from_civil(2200, 1, 1));
    let mut z = start;
    let (mut y, mut m, mut d) = (1800i64, 1u32, 1u32);
    while z < end {
        assert_eq!(civil_from_days(z), (y, m, d), "at day {z}");
        assert_eq!(days_from_civil(y, m, d), z);
        // Step one civil day and one count of days, and they must agree.
        d += 1;
        if d > days_in_month(y, m) {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
        z += 1;
    }
    // The century rules, stated outright.
    assert_eq!(days_in_month(1900, 2), 28, "1900 is not a leap year");
    assert_eq!(days_in_month(2000, 2), 29, "2000 is");
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(2023, 2), 28);
}

/// The calendar lays out a month by where its first day falls, so a wrong
/// weekday shifts every date in the grid by a column.
#[test]
fn weekdays_are_where_they_should_be() {
    // The epoch was a Thursday; Monday is 0.
    assert_eq!(weekday(1970, 1, 1), 3);
    assert_eq!(weekday(2000, 1, 1), 5, "2000-01-01 was a Saturday");
    assert_eq!(weekday(2024, 2, 29), 3, "the leap day was a Thursday");
    assert_eq!(weekday(2026, 8, 9), 6, "a Sunday");
}

/// A value that obviously means a date should not be thrown away for being
/// punctuated differently — and one that is nearly right must be, because it
/// otherwise reaches a tool and comes back as a run that did nothing.
#[test]
fn a_date_is_read_generously_and_rejected_strictly() {
    let d = parse("2024-01-31").unwrap();
    assert_eq!((d.year, d.month, d.day, d.hour), (2024, 1, 31, 0));
    // Either separator, and a partial time.
    assert_eq!(parse("2024-01-31T09:30:15").unwrap().second, 15);
    assert_eq!(parse("2024-01-31 09:30").unwrap().minute, 30);
    assert_eq!(parse("  2024-01-31  ").unwrap().day, 31);
    // A leap second is a real thing to find in a log.
    assert!(parse("2016-12-31T23:59:60").is_some());

    // Not dates.
    for bad in [
        "31/01/2024",      // the format the tool rejects outright
        "2024-13-01",      // no thirteenth month
        "2024-02-30",      // not in that month
        "2023-02-29",      // not in that year
        "2024-01-31T25:00:00",
        "2024-01-31T10:61:00",
        "2024-01",         // not a day
        "yesterday",
        "",
    ] {
        assert!(parse(bad).is_none(), "{bad:?} was read as a date");
    }
}

/// Whatever was typed, one form leaves the field.
#[test]
fn a_date_leaves_in_the_form_a_tool_will_take() {
    assert_eq!(normalise("2024-01-31", true).unwrap(), "2024-01-31T00:00:00");
    assert_eq!(normalise("2024-1-3", true).unwrap(), "2024-01-03T00:00:00");
    assert_eq!(normalise("2024-01-31 9:30", true).unwrap(), "2024-01-31T09:30:00");
    assert_eq!(normalise("2024-01-31T09:30:15", false).unwrap(), "2024-01-31");
    // An empty bound is a real answer — no bound — not a mistake.
    assert_eq!(normalise("", true).unwrap(), "");
    assert_eq!(normalise("   ", true).unwrap(), "");
    assert!(normalise("31/01/2024", true).is_none());
}
