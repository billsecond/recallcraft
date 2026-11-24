//! Times: now, and how they're shown (the system's time zone).

/// Milliseconds since 1970 (UTC).
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Microseconds since 1970 (UTC).
pub fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

fn zoned(ms: u64) -> Option<jiff::Zoned> {
    let ms = i64::try_from(ms).ok()?;
    Some(jiff::Timestamp::from_millisecond(ms).ok()?.to_zoned(jiff::tz::TimeZone::system()))
}

/// A moment's local date, `2026-11-24` (empty for one that can't be).
pub fn local_date(ms: u64) -> String {
    zoned(ms).map(|z| format!("{:04}-{:02}-{:02}", z.year(), z.month(), z.day())).unwrap_or_default()
}

/// A moment's local time, `14:05`.
pub fn local_time(ms: u64) -> String {
    zoned(ms).map(|z| format!("{:02}:{:02}", z.hour(), z.minute())).unwrap_or_default()
}

/// A moment's local time with seconds, `14:05:09`.
pub fn local_time_s(ms: u64) -> String {
    zoned(ms).map(|z| format!("{:02}:{:02}:{:02}", z.hour(), z.minute(), z.second())).unwrap_or_default()
}

/// A moment as a folder or file name, `2026-11-24_14-05-09`.
pub fn local_stamp(ms: u64) -> String {
    zoned(ms)
        .map(|z| format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", z.year(), z.month(), z.day(), z.hour(), z.minute(), z.second()))
        .unwrap_or_else(|| ms.to_string())
}

/// A day heading for the call list: Today, Yesterday, `Monday 24 November`.
pub fn day_heading(ms: u64, now_ms: u64) -> String {
    let (Some(z), Some(now)) = (zoned(ms), zoned(now_ms)) else { return String::new() };
    let (d, today) = (z.date(), now.date());
    if d == today {
        return "Today".to_string();
    }
    if today.yesterday().is_ok_and(|y| y == d) {
        return "Yesterday".to_string();
    }
    let day = match z.weekday() {
        jiff::civil::Weekday::Monday => "Monday",
        jiff::civil::Weekday::Tuesday => "Tuesday",
        jiff::civil::Weekday::Wednesday => "Wednesday",
        jiff::civil::Weekday::Thursday => "Thursday",
        jiff::civil::Weekday::Friday => "Friday",
        jiff::civil::Weekday::Saturday => "Saturday",
        jiff::civil::Weekday::Sunday => "Sunday",
    };
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let month = MONTHS.get(usize::try_from(z.month()).unwrap_or(1).saturating_sub(1)).copied().unwrap_or("");
    format!("{day} {} {month}", z.day())
}

/// A length of time as a clock: `4:05`, `1:02:03`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A length of time in words: `45 s`, `12 min`, `1 h 05 min`.
pub fn duration_words(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m} min");
    }
    format!("{} h {:02} min", m / 60, m % 60)
}

/// A length of time for subtitles, `01:02:03,456` (SubRip) or `01:02:03.456` (WebVTT).
pub fn subtitle_time(ms: u64, sep: char) -> String {
    let (h, m, s, f) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}{sep}{f:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_and_words() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_000), "1:05");
        assert_eq!(clock(3_723_000), "1:02:03");
        assert_eq!(duration_words(45_000), "45 s");
        assert_eq!(duration_words(12 * 60_000), "12 min");
        assert_eq!(duration_words(65 * 60_000), "1 h 05 min");
        assert_eq!(subtitle_time(3_723_456, ','), "01:02:03,456");
    }

    #[test]
    fn local_times_are_plausible_and_junk_is_empty() {
        let n = now_ms();
        assert_eq!(local_date(n).len(), 10);
        assert_eq!(local_time(n).len(), 5);
        assert_eq!(local_stamp(n).len(), 19);
        assert_eq!(day_heading(n, n), "Today");
        assert_eq!(day_heading(n - 86_400_000, n), "Yesterday");
        assert!(!day_heading(n - 5 * 86_400_000, n).is_empty());
        assert_eq!(local_date(u64::MAX), "");
        assert_eq!(day_heading(u64::MAX, n), "");
    }
}
