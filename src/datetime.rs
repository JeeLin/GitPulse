//! Calendar helpers used by the UI, with no `chrono` dependency.
//!
//! Both the dashboard preview (`src/ui/dashboard.rs`) and the history view
//! (`src/git/history.rs`) used to keep their own copy of Howard Hinnant's
//! `civil_from_days` (B4-12). They are structurally identical, so they share
//! one implementation here.

/// Days since 1970-01-01 to a proleptic Gregorian date
/// (Howard Hinnant's `civil_from_days` algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { y + 1 } else { y }, month, day)
}

/// Format a Unix timestamp as `YYYY-MM-DD HH:MM` in the given UTC offset
/// (in minutes), without `chrono`. The dashboard passes `0` so its preview
/// keeps rendering UTC while the history view hands in the commit's offset.
pub fn format_local_time(seconds: i64, offset_minutes: i32) -> String {
    if seconds <= 0 {
        return "-".to_string();
    }
    let local = seconds + i64::from(offset_minutes) * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (secs / 3600, (secs % 3600) / 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}
