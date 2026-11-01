//! A package's release date: the one place that knows the date formats.
//!
//! Package directories write `Package.Release Date` as RFC 2822
//! (`Thu, 19 May 2016 15:46:24 -0500`), with `GMT` for some entries and a
//! sign-less `0530` zone for others. Times are Unix seconds, so a release
//! compares with the clock directly.

const SECS_PER_MINUTE: i64 = 60;
const SECS_PER_HOUR: i64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: i64 = 24 * SECS_PER_HOUR;

// Days-from-civil calendar arithmetic (Howard Hinnant's `chrono`-compatible
// algorithms): years are split into 400-year eras of 146 097 days, counted from
// 0000-03-01 so a leap day is always the last day of its year.
const YEARS_PER_ERA: i64 = 400;
const DAYS_PER_ERA: i64 = 146_097;
const DAYS_FROM_0000_03_01_TO_1970_01_01: i64 = 719_468;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Days since 1970-01-01 of a proleptic Gregorian date; `month` is 1..=12.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(YEARS_PER_ERA);
    let year_of_era = year - era * YEARS_PER_ERA;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - DAYS_FROM_0000_03_01_TO_1970_01_01
}

/// The `(year, month, day)` of a count of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + DAYS_FROM_0000_03_01_TO_1970_01_01;
    let era = days.div_euclid(DAYS_PER_ERA);
    let day_of_era = days - era * DAYS_PER_ERA;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * YEARS_PER_ERA + i64::from(month <= 2);
    (year, month, day)
}

/// `Thu, 19 May 2016 15:46:24 -0500` as Unix seconds, or `None` when it is not
/// one of the shapes the feeds use. The weekday is not checked, and a zone
/// without a sign (`0530`) is east of UTC, as the feeds mean it.
pub fn parse(text: &str) -> Option<i64> {
    let text = text.split_once(',').map_or(text, |(_, rest)| rest);
    let mut words = text.split_whitespace();
    let day: i64 = words.next()?.parse().ok()?;
    let month = words.next()?.to_ascii_lowercase();
    let month = MONTHS.iter().position(|m| *m == month)? as i64 + 1;
    let year = words.next().filter(|y| y.len() == 4)?.parse().ok()?;
    let mut clock = words.next()?.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    let second: i64 = clock.next()?.parse().ok()?;
    let offset = zone_offset(words.next()?)?;
    if words.next().is_some() || clock.next().is_some() {
        return None;
    }
    if hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    // A day the month does not have (Feb 30) rolls into the next month.
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    Some(days * SECS_PER_DAY + hour * SECS_PER_HOUR + minute * SECS_PER_MINUTE + second - offset)
}

/// Seconds east of UTC for `GMT`, `-0500`, `+0530` or a sign-less `0530`.
fn zone_offset(zone: &str) -> Option<i64> {
    if zone == "GMT" {
        return Some(0);
    }
    let (sign, digits) = match zone.as_bytes().first()? {
        b'-' => (-1, &zone[1..]),
        b'+' => (1, &zone[1..]),
        _ => (1, zone),
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes): (i64, i64) = (digits[..2].parse().ok()?, digits[2..].parse().ok()?);
    (minutes < 60).then_some(sign * (hours * SECS_PER_HOUR + minutes * SECS_PER_MINUTE))
}

/// The UTC day of a Unix time, `YYYY-MM-DD`.
pub fn day(time: i64) -> String {
    let (year, month, day) = civil_from_days(time.div_euclid(SECS_PER_DAY));
    format!("{year:04}-{month:02}-{day:02}")
}

/// A Unix time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(time: i64) -> String {
    let seconds = time.rem_euclid(SECS_PER_DAY);
    let (hour, minute, second) = (
        seconds / SECS_PER_HOUR,
        seconds % SECS_PER_HOUR / SECS_PER_MINUTE,
        seconds % SECS_PER_MINUTE,
    );
    format!("{}T{hour:02}:{minute:02}:{second:02}Z", day(time))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_CLOCK_AS_UTC: i64 = 1_463_672_784; // 2016-05-19 15:46:24 UTC

    #[test]
    fn a_signed_offset_is_taken_off_the_local_time() {
        assert_eq!(
            parse("Thu, 19 May 2016 15:46:24 -0500"),
            Some(LOCAL_CLOCK_AS_UTC + 5 * SECS_PER_HOUR)
        );
    }

    #[test]
    fn gmt_is_offset_zero() {
        assert_eq!(
            parse("Thu, 19 May 2016 15:46:24 GMT"),
            Some(LOCAL_CLOCK_AS_UTC)
        );
    }

    #[test]
    fn a_zone_without_a_sign_reads_as_east_of_utc() {
        let unsigned = parse("Thu, 19 May 2016 15:46:24 0530");
        assert_eq!(unsigned, Some(LOCAL_CLOCK_AS_UTC - 19_800));
        assert_eq!(unsigned, parse("Thu, 19 May 2016 15:46:24 +0530"));
        assert_ne!(unsigned, parse("Thu, 19 May 2016 15:46:24 -0530"));
    }

    #[test]
    fn unreadable_placeholders_are_none() {
        assert_eq!(parse("???, 19 ??? 2016 15:46:24 -0500"), None);
    }

    #[test]
    fn text_that_is_not_a_date_is_none() {
        for text in [
            "",
            "Thu, 19 May 2016",
            "Thu, 19 May 2016 15:46:24",
            "Thu, 19 May 2016 15:46:24 EST",
            "Thu, 19 May 2016 15:46:24 -05",
            "Thu, 19 May 2016 15:46:24 +0560",
            "Thu, 19 May 2016 15:46 -0500",
            "Thu, 19 May 2016 15:46:24:01 -0500",
            "Thu, 19 May 2016 15:46:24 -0500 extra",
            "Thu, 19 May 16 15:46:24 -0500",
            "Thu, 19 May 2016 24:00:00 -0500",
            "Thu, 19 May 2016 15:60:00 -0500",
            "Thu, 19 May 2016 15:46:60 -0500",
            "Thu, 0 May 2016 15:46:24 -0500",
            "Thu, 32 May 2016 15:46:24 -0500",
            "Thu, 30 Feb 2016 15:46:24 -0500",
            "Thu, x May 2016 15:46:24 -0500",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_missing_weekday_and_a_lowercase_month_are_accepted() {
        assert_eq!(parse("19 may 2016 15:46:24 GMT"), Some(LOCAL_CLOCK_AS_UTC));
    }

    #[test]
    fn the_last_day_of_a_month_and_a_leap_day_are_dates() {
        assert_eq!(parse("Thu, 29 Feb 2024 00:00:00 GMT"), Some(1_709_164_800));
        assert_eq!(parse("Thu, 29 Feb 2023 00:00:00 GMT"), None);
        assert_eq!(parse("Thu, 31 Dec 1999 23:59:59 GMT"), Some(946_684_799));
    }

    #[test]
    fn a_time_formats_as_its_utc_day_and_instant() {
        for (time, day_text, instant) in [
            (0, "1970-01-01", "1970-01-01T00:00:00Z"),
            (1_700_000_000, "2023-11-14", "2023-11-14T22:13:20Z"),
            (1_709_164_800, "2024-02-29", "2024-02-29T00:00:00Z"),
            (-1, "1969-12-31", "1969-12-31T23:59:59Z"),
        ] {
            assert_eq!(day(time), day_text);
            assert_eq!(rfc3339(time), instant);
        }
    }

    #[test]
    fn what_parse_reads_formats_back_to_the_same_utc_instant() {
        let time = parse("Thu, 19 May 2016 15:46:24 -0500").unwrap();
        assert_eq!(rfc3339(time), "2016-05-19T20:46:24Z");
    }
}
