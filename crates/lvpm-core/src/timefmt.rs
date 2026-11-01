//! Times as text: a Unix time as RFC 3339, and how long a wait is. No time
//! crate is needed for these two.

/// `unix` seconds as UTC in RFC 3339, `2026-10-31T14:05:00Z`.
pub fn rfc3339(unix: i64) -> String {
    let (days, secs) = (unix.div_euclid(SECS_PER_DAY), unix.rem_euclid(SECS_PER_DAY));
    // Days since 1970-01-01 to a civil date: Howard Hinnant's algorithm, with
    // March as the first month of the shifted year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / SECS_PER_HOUR,
        secs % SECS_PER_HOUR / SECS_PER_MINUTE,
        secs % SECS_PER_MINUTE
    )
}

const SECS_PER_MINUTE: i64 = 60;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;

/// How long `secs` is, roughly: `in 12 min`, `in 3 h`, `in 40 days`; `now`
/// once the time has passed.
pub fn time_until(secs: i64) -> String {
    match secs {
        ..=0 => "now".to_string(),
        s if s < 90 * SECS_PER_MINUTE => {
            format!("in {} min", (s + SECS_PER_MINUTE - 1) / SECS_PER_MINUTE)
        }
        s if s < 2 * SECS_PER_DAY => format!("in {} h", s / SECS_PER_HOUR),
        s => format!("in {} days", s / SECS_PER_DAY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unix_time_is_utc_in_rfc_3339_across_leap_days_and_centuries() {
        for (unix, want) in [
            (0, "1970-01-01T00:00:00Z"),
            (946_684_799, "1999-12-31T23:59:59Z"),
            (951_868_800, "2000-03-01T00:00:00Z"),
            (1_709_208_000, "2024-02-29T12:00:00Z"),
            (1_793_455_500, "2026-10-31T14:05:00Z"),
            (4_070_908_800, "2099-01-01T00:00:00Z"),
        ] {
            assert_eq!(rfc3339(unix), want);
        }
    }

    #[test]
    fn a_wait_is_told_in_minutes_then_hours_then_days() {
        for (secs, want) in [
            (-30, "now"),
            (0, "now"),
            (1, "in 1 min"),
            (720, "in 12 min"),
            (5_399, "in 90 min"),
            (5_400, "in 1 h"),
            (172_799, "in 47 h"),
            (172_800, "in 2 days"),
        ] {
            assert_eq!(time_until(secs), want);
        }
    }
}
