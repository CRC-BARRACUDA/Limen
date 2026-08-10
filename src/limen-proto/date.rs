//! Dates, without a date library.
//!
//! Shared between the GUI — which draws a calendar out of these — and the
//! modules, which validate what a user typed before handing it to a tool. One
//! implementation, because a field that accepts what the module then rejects is
//! worse than having no field at all.

/// A calendar date, and optionally a time of day.
///
/// Kept as plain numbers rather than pulling in a date library: the whole of
/// what this application does with a date is show a month, step between months,
/// and print one back in the format a tool will accept.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Days since 1970-01-01 for a civil date, and back again.
///
/// Howard Hinnant's `days_from_civil`/`civil_from_days`: exact for every date in
/// the proleptic Gregorian calendar, which is more than a log viewer needs and
/// no more code than a wrong version would be.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// 0 = Monday … 6 = Sunday. 1970-01-01 was a Thursday.
pub fn weekday(y: i64, m: u32, d: u32) -> u32 {
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u32
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// Today, in UTC.
///
/// UTC and not local time, because the alternative is a timezone database. It
/// only decides which month the calendar opens on and what "Today" means.
pub fn today() -> Date {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    Date {
        year,
        month,
        day,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

/// Read a date a person might have typed.
///
/// Deliberately generous about the separator and about a missing time — the
/// point is that a value which obviously means a date is not thrown away for
/// being punctuated differently. Deliberately strict about everything else: a
/// date that is nearly right is worse than one that is plainly wrong, because
/// it reaches the tool and comes back as a run that did nothing.
pub fn parse(s: &str) -> Option<Date> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (day, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, t.trim()),
        None => (s, ""),
    };
    let mut date = day.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: u32 = date.next()?.parse().ok()?;
    let dom: u32 = date.next()?.parse().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if dom < 1 || dom > days_in_month(year, month) {
        return None;
    }

    let mut parts = time.split(':').filter(|p| !p.is_empty());
    let num = |p: Option<&str>, max: u32| -> Option<u32> {
        match p {
            None => Some(0),
            Some(v) => v.parse().ok().filter(|n| *n <= max),
        }
    };
    let hour = num(parts.next(), 23)?;
    let minute = num(parts.next(), 59)?;
    // 60 for a leap second, which a log can legitimately carry.
    let second = num(parts.next(), 60)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Date {
        year,
        month,
        day: dom,
        hour,
        minute,
        second,
    })
}

/// Print a date the way tools want it: `2024-01-31T00:00:00`, or just the day.
pub fn format(d: Date, with_time: bool) -> String {
    if with_time {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            d.year, d.month, d.day, d.hour, d.minute, d.second
        )
    } else {
        format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
    }
}

/// What should be sent for what was typed: the canonical form, or `None` if it
/// is not a date at all.
///
/// An empty field normalises to an empty string rather than to `None` — "no
/// bound" is an answer, and not the same as an unreadable one.
pub fn normalise(s: &str, with_time: bool) -> Option<String> {
    if s.trim().is_empty() {
        return Some(String::new());
    }
    parse(s).map(|d| format(d, with_time))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
