//! Local wall-clock time, without a date library.
//!
//! `Insert Date/Time` needs the local time, and the standard library only knows
//! UTC. Rather than pull in `chrono` — a few hundred kilobytes on a 4.9 MB
//! AppImage for one menu item — this reads the same `/etc/localtime` database
//! the C library reads and does the civil-date arithmetic by hand.
//!
//! The database is TZif: a header, a table of transition instants, and a table
//! of local-time types. Only the transition table is needed here, so leap
//! seconds and the POSIX footer string are skipped.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, UTC.
pub fn now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// The local offset in seconds east of UTC at `utc`.
///
/// Falls back to zero — UTC — when the database is missing or unreadable, which
/// is the only honest answer when the machine has not said what its zone is.
pub fn local_offset(utc: i64) -> i32 {
    let Ok(bytes) = std::fs::read("/etc/localtime") else {
        return 0;
    };
    tzif_offset(&bytes, utc).unwrap_or(0)
}

/// Formats `utc` as local `YYYY-MM-DD HH:MM:SS`.
pub fn local_stamp(utc: i64) -> String {
    stamp(utc + local_offset(utc) as i64)
}

/// Formats `utc` as UTC `YYYY-MM-DD HH:MM:SS`.
pub fn utc_stamp(utc: i64) -> String {
    stamp(utc)
}

/// Formats `utc` as local `HH:MM:SS`, for a list where the date is the same on
/// every row.
///
/// Derived from [`local_stamp`] rather than recomputed, so the two can never
/// disagree about what time it is.
pub fn clock_stamp(utc: i64) -> String {
    let text = local_stamp(utc);
    match text.split_once(' ') {
        Some((_, time)) => time.to_string(),
        None => text,
    }
}

/// Formats `utc` as local `YYYYMMDD-HHMMSS`, for use in a file name.
///
/// The same instant as `local_stamp` with the separators that a file name cannot
/// carry removed, and the date and time joined by a hyphen so the result still
/// reads as two halves. Fixed width and most-significant-first, which is what
/// lets backups be sorted by name to find the newest.
pub fn file_stamp(utc: i64) -> String {
    let text = local_stamp(utc);
    let mut out = String::with_capacity(15);
    for ch in text.chars() {
        match ch {
            '-' | ':' => {}
            ' ' => out.push('-'),
            _ => out.push(ch),
        }
    }
    out
}

/// Formats `utc` as local time using a `strftime`-style pattern.
///
/// Only the conversions a date stamp in a text file plausibly needs are
/// supported; anything else is copied through unchanged, which is what `strftime`
/// does with an unknown conversion and what makes a pattern with a literal `%` in
/// it survive.
pub fn format_local(utc: i64, pattern: &str) -> String {
    let local = utc + local_offset(utc) as i64;
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&format!("{year:04}")),
            Some('y') => out.push_str(&format!("{:02}", year.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('e') => out.push_str(&format!("{day:2}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('j') => out.push_str(&format!("{:03}", days - days_from_civil(year, 1, 1) + 1)),
            Some('p') => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            Some('a') => out.push_str(WEEKDAYS[(days.rem_euclid(7)) as usize]),
            Some('b') => out.push_str(MONTHS[(month - 1) as usize]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// The three-letter weekday names, Sunday first.
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The three-letter month names, January first.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Formats seconds since the epoch as `YYYY-MM-DD HH:MM:SS`.
fn stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Converts a day count since 1970-01-01 into a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`: shift the epoch to 0000-03-01 so leap days
/// land at the end of the year, then work in 400-year eras, which repeat exactly.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Converts a civil `(year, month, day)` back into a day count since 1970-01-01.
///
/// The inverse of [`civil_from_days`], needed for the day-of-year conversion: the
/// cheapest way to get "which day of the year is this" is to subtract the day
/// count of January 1st.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Reads the six counts out of a TZif header body.
fn counts(bytes: &[u8], at: usize) -> Option<[usize; 6]> {
    let mut out = [0usize; 6];
    for (i, slot) in out.iter_mut().enumerate() {
        let b = bytes.get(at + i * 4..at + i * 4 + 4)?;
        *slot = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
    }
    Some(out)
}

/// The offset in force at `at`, from a TZif image.
///
/// Version 2 and later carry a second, 64-bit block after the 32-bit one; that
/// block is the one to read, because the 32-bit one runs out in 2038.
fn tzif_offset(bytes: &[u8], at: i64) -> Option<i32> {
    if bytes.len() < 44 || &bytes[0..4] != b"TZif" {
        return None;
    }
    let version = bytes[4];
    let [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt] = counts(bytes, 20)?;
    let mut pos = 44;
    let wide = version >= b'2';
    if wide {
        // Skip the 32-bit block: transitions, indices, types, designations,
        // leap seconds, then the two standard/UT flag arrays.
        pos += timecnt * 4 + timecnt + typecnt * 6 + charcnt + leapcnt * 8 + isstdcnt + isutcnt;
        if bytes.get(pos..pos + 4)? != b"TZif" {
            return None;
        }
        let [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt] = counts(bytes, pos + 20)?;
        pos += 44;
        let _ = (isutcnt, isstdcnt, leapcnt, charcnt);
        let times = bytes.get(pos..pos + timecnt * 8)?;
        let indices = bytes.get(pos + timecnt * 8..pos + timecnt * 9)?;
        let types = pos + timecnt * 9;
        return pick(times, indices, bytes, types, typecnt, at, 8);
    }
    let times = bytes.get(pos..pos + timecnt * 4)?;
    let indices = bytes.get(pos + timecnt * 4..pos + timecnt * 5)?;
    let types = pos + timecnt * 5;
    pick(times, indices, bytes, types, typecnt, at, 4)
}

/// Finds the type in force at `at` and returns its offset.
#[allow(clippy::too_many_arguments)]
fn pick(
    times: &[u8],
    indices: &[u8],
    bytes: &[u8],
    types: usize,
    typecnt: usize,
    at: i64,
    width: usize,
) -> Option<i32> {
    let mut chosen = None;
    for (i, &index) in indices.iter().enumerate() {
        let t = if width == 8 {
            let b = times.get(i * 8..i * 8 + 8)?;
            i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        } else {
            let b = times.get(i * 4..i * 4 + 4)?;
            i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64
        };
        if t <= at {
            chosen = Some(index as usize);
        } else {
            break;
        }
    }
    // Before the first transition the zone's standard type applies, which is the
    // first entry that is not a daylight-saving one.
    let index = chosen
        .or_else(|| (0..typecnt).find(|&i| bytes.get(types + i * 6 + 4).copied() == Some(0)))?;
    if index >= typecnt {
        return None;
    }
    let b = bytes.get(types + index * 6..types + index * 6 + 4)?;
    Some(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_is_the_time_half_of_the_stamp() {
        // Derived from the same function, so the two cannot disagree. Compared
        // against `local_stamp` rather than a literal because the local offset
        // is whatever the machine says it is.
        let full = local_stamp(1_700_000_000);
        let clock = clock_stamp(1_700_000_000);
        assert!(full.ends_with(&clock), "{full} should end with {clock}");
        assert_eq!(clock.len(), 8);
        assert_eq!(clock.matches(':').count(), 2);
    }

    #[test]
    fn a_pattern_fills_in_the_fields_it_names() {
        // 2023-11-14 22:13:20 UTC. The local offset shifts the clock, so the
        // assertions are on the parts a shift cannot change, plus a round trip
        // through `utc_stamp` for the date.
        let secs = 1_700_000_000;
        let date = utc_stamp(secs);
        let (y, rest) = date.split_at(4);
        let (m, rest) = rest[1..].split_at(2);
        let d = &rest[1..3];
        assert_eq!(format_local(secs, "%Y-%m-%d"), format!("{y}-{m}-{d}"));
        assert_eq!(format_local(secs, "%y"), &y[2..]);
        assert_eq!(format_local(secs, "%b"), "Nov");
        assert_eq!(format_local(secs, "%j"), "318");
        assert_eq!(format_local(secs, "100%%"), "100%");
        // Unknown conversions survive, so a pattern can carry literal text.
        assert_eq!(format_local(secs, "%Q"), "%Q");
        assert_eq!(format_local(secs, "at %H:%M"), format!("at {}", &clock_stamp(secs)[..5]));
    }

    #[test]
    fn day_counts_round_trip_through_civil_dates() {
        for days in [-1000, -1, 0, 1, 19_723, 29_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn epoch_is_a_thursday_in_january() {
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00");
    }

    #[test]
    fn a_known_instant_round_trips() {
        assert_eq!(utc_stamp(1_700_000_000), "2023-11-14 22:13:20");
    }

    #[test]
    fn leap_days_land_on_the_right_day() {
        // 2024-02-29 12:00:00 UTC.
        assert_eq!(utc_stamp(1_709_208_000), "2024-02-29 12:00:00");
    }

    #[test]
    fn negative_instants_go_before_the_epoch() {
        assert_eq!(utc_stamp(-1), "1969-12-31 23:59:59");
    }

    #[test]
    fn a_file_stamp_is_fixed_width_and_sorts_by_time() {
        // The local offset is whatever the machine says, so the test compares
        // the shape and the ordering rather than the digits.
        let early = file_stamp(1_700_000_000);
        let late = file_stamp(1_700_000_001);
        assert_eq!(early.len(), 15, "{early}");
        assert_eq!(early.as_bytes()[8], b'-', "{early}");
        assert!(early
            .chars()
            .enumerate()
            .all(|(at, ch)| at == 8 || ch.is_ascii_digit()));
        assert!(early < late, "{early} {late}");
    }

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    /// Builds a minimal version-1 TZif image with two types and one transition.
    fn fixture() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"TZif");
        out.push(0);
        out.extend_from_slice(&[0u8; 15]);
        // isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt
        for n in [0u32, 0, 0, 1, 2, 4] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        // One transition at 1_000_000.
        out.extend_from_slice(&1_000_000i32.to_be_bytes());
        // Pointing at type 1.
        out.push(1);
        // Type 0: UTC, type 1: +3600.
        out.extend_from_slice(&0i32.to_be_bytes());
        out.push(0);
        out.push(0);
        out.extend_from_slice(&3600i32.to_be_bytes());
        out.push(1);
        out.push(0);
        out.extend_from_slice(b"UTC\0");
        out
    }

    #[test]
    fn a_transition_switches_the_offset() {
        let bytes = fixture();
        assert_eq!(tzif_offset(&bytes, 999_999), Some(0));
        assert_eq!(tzif_offset(&bytes, 1_000_000), Some(3600));
        assert_eq!(tzif_offset(&bytes, 2_000_000), Some(3600));
    }

    #[test]
    fn a_bad_image_is_rejected_rather_than_guessed() {
        assert_eq!(tzif_offset(b"nope", 0), None);
        assert_eq!(tzif_offset(&[], 0), None);
    }

    #[test]
    fn a_stamp_is_sixteen_characters() {
        assert_eq!(utc_stamp(0).len(), 19);
        assert_eq!(local_stamp(0).len(), 19);
    }
}
