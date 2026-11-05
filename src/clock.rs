//! Local time and the user id, read from the C library rather than by running
//! `date` and `id` (the agent logs on every switch).

use std::time::{SystemTime, UNIX_EPOCH};

/// A local calendar time.
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// The current local time, or `None` if the C library can't convert it.
pub fn now() -> Option<LocalTime> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let time = libc::time_t::try_from(secs).ok()?;
    // SAFETY: `tm` is plain old data, so all-zero is a valid value to start from.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live locals; `localtime_r` is the
    // thread-safe variant and writes only to `tm`.
    let result = unsafe { libc::localtime_r(&raw const time, &raw mut tm) };
    if result.is_null() {
        return None;
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: u32::try_from(tm.tm_mon + 1).ok()?,
        day: u32::try_from(tm.tm_mday).ok()?,
        hour: u32::try_from(tm.tm_hour).ok()?,
        minute: u32::try_from(tm.tm_min).ok()?,
        second: u32::try_from(tm.tm_sec).ok()?,
    })
}

/// `2026-11-03 14:05:09`, for log lines.
pub fn log_stamp() -> String {
    now().map_or_else(String::new, |t| {
        format!(
            "{}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        )
    })
}

/// `20261103-140509`, for backup file names; sorts by time.
pub fn file_stamp() -> String {
    now().map_or_else(
        || {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            secs.to_string()
        },
        |t| {
            format!(
                "{}{:02}{:02}-{:02}{:02}{:02}",
                t.year, t.month, t.day, t.hour, t.minute, t.second
            )
        },
    )
}

/// The current user's id (launchd's `gui/<uid>` domain; `pgrep -u` on Linux).
pub fn uid() -> u32 {
    // SAFETY: `getuid` takes no arguments, can't fail and has no side effects.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_have_a_fixed_shape() {
        let log = log_stamp();
        assert_eq!(log.len(), "2026-11-03 14:05:09".len(), "{log}");
        assert_eq!(&log[4..5], "-");
        assert_eq!(&log[10..11], " ");
        let file = file_stamp();
        assert_eq!(file.len(), "20261103-140509".len(), "{file}");
        assert!(file.chars().all(|c| c.is_ascii_digit() || c == '-'));
        assert!(uid() > 0 || std::env::var("USER").as_deref() == Ok("root"));
    }
}
