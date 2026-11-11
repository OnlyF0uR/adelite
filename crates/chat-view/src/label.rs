//! The small texts worked out from numbers: times and days as the user's
//! clock has them, a name's initials, a file's size.

use adelite_ui::platform::{LocalTime, local_time};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// What "now" is while a frame is drawn: asked once, used by every label.
#[derive(Debug, Clone, Copy, Default)]
pub struct Today {
    /// Now, in Unix seconds.
    pub now: i64,
    /// Today's date ([`LocalTime::date`]).
    pub date: i32,
    /// Yesterday's.
    pub yesterday: i32,
}

impl Today {
    /// Today, when `now` is the time in Unix seconds.
    #[must_use]
    pub fn at(now: i64) -> Self {
        Self { now, date: local_time(now).date(), yesterday: local_time(now - 86_400).date() }
    }
}

/// A month's name; empty for a number that is not one.
fn month(number: i32) -> &'static str {
    usize::try_from(number - 1).ok().and_then(|index| MONTHS.get(index)).copied().unwrap_or_default()
}

/// The first three letters of a month's name.
fn short_month(number: i32) -> &'static str {
    month(number).get(..3).unwrap_or_default()
}

/// A calendar day between the messages of two days: "Today", "12 March",
/// "12 March 2024" in another year.
#[must_use]
pub fn day(date: i32, today: Today) -> String {
    let (year, number, day) = (date / 10_000, date / 100 % 100, date % 100);
    if date == today.date {
        "Today".to_owned()
    } else if date == today.yesterday {
        "Yesterday".to_owned()
    } else if year == today.date / 10_000 {
        format!("{day} {}", month(number))
    } else {
        format!("{day} {} {year}", month(number))
    }
}

/// When a chat's latest message was sent, as its row says it: the time
/// today, the weekday this week, the date before that.
#[must_use]
pub fn list_time(unix: i64, today: Today) -> String {
    let at = local_time(unix);
    if at.date() == today.date {
        clock(at)
    } else if (0..6 * 86_400).contains(&(today.now - unix)) {
        WEEKDAYS.get(usize::from(at.weekday)).copied().unwrap_or_default().to_owned()
    } else if at.year == today.date / 10_000 {
        format!("{} {}", at.day, short_month(i32::from(at.month)))
    } else {
        format!("{} {} {}", at.day, short_month(i32::from(at.month)), at.year)
    }
}

/// When someone was last here, as the line under their name says it.
#[must_use]
pub fn last_seen(unix: i64, today: Today) -> String {
    let at = local_time(unix);
    if at.date() == today.date {
        format!("last seen at {}", clock(at))
    } else if at.date() == today.yesterday {
        format!("last seen yesterday at {}", clock(at))
    } else {
        format!("last seen {} {}", at.day, short_month(i32::from(at.month)))
    }
}

/// `14:05`.
fn clock(at: LocalTime) -> String {
    format!("{:02}:{:02}", at.hour, at.minute)
}

/// The letters a face without a picture wears: the first of the first two
/// words of `name`, in capitals.
#[must_use]
pub fn initials(name: &str) -> String {
    let letters = name.split_whitespace().filter_map(|word| word.chars().find(|c| c.is_alphanumeric()));
    letters.take(2).flat_map(char::to_uppercase).collect()
}

/// How long something runs, as a player says it: `0:07`, `3:15`, `1:02:09`.
#[must_use]
pub fn duration(seconds: u32) -> String {
    let (hours, minutes, seconds) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// A count as it is said in passing: `812`, `1.2K`, `34K`, `5.6M`.
#[must_use]
pub fn count(number: u32) -> String {
    let short = |value: f64, unit: &str| {
        if value < 10.0 { format!("{value:.1}{unit}") } else { format!("{value:.0}{unit}") }
    };
    match number {
        0..1_000 => number.to_string(),
        1_000..1_000_000 => short(f64::from(number) / 1e3, "K"),
        _ => short(f64::from(number) / 1e6, "M"),
    }
}

/// A file's size as people say it: `812 B`, `2.4 KB`, `13 MB`.
#[must_use]
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    let unit = UNITS.get(unit).copied().unwrap_or_default();
    if size < 10.0 { format!("{size:.1} {unit}") } else { format!("{size:.0} {unit}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_named_from_today() {
        let today = Today { now: 0, date: 20261110, yesterday: 20261109 };
        assert_eq!(day(20261110, today), "Today");
        assert_eq!(day(20261109, today), "Yesterday");
        assert_eq!(day(20260312, today), "12 March");
        assert_eq!(day(20241231, today), "31 December 2024");
        assert_eq!(day(20269901, today), "1 ", "a month that is none has no name, and no panic");
    }

    #[test]
    fn list_times_shorten_with_age() {
        // Noon UTC is the same calendar day in every time zone but the far Pacific's.
        let noon = 1_794_268_800 + 43_200;
        let today = Today::at(noon);
        assert_eq!(list_time(noon, today).len(), 5, "the time today");
        assert!(WEEKDAYS.contains(&list_time(noon - 3 * 86_400, today).as_str()), "the weekday this week");
        assert!(list_time(noon - 40 * 86_400, today).ends_with("Oct"), "the date this year");
        assert!(list_time(noon - 800 * 86_400, today).ends_with("2024"), "with the year before it");
        assert!(last_seen(noon, today).starts_with("last seen at "));
        assert!(last_seen(noon - 86_400, today).starts_with("last seen yesterday at "));
    }

    #[test]
    fn names_and_sizes_shorten() {
        assert_eq!(
            (initials("Mara Lind"), initials("design team chat"), initials("élan")),
            ("ML".into(), "DT".into(), "É".into())
        );
        assert_eq!((initials(""), initials("🎉 Party")), (String::new(), "P".into()));
        assert_eq!([duration(7), duration(195), duration(3_729)], ["0:07", "3:15", "1:02:09"]);
        assert_eq!(
            [count(0), count(812), count(1_240), count(34_400), count(5_600_000)],
            ["0", "812", "1.2K", "34K", "5.6M"]
        );
        assert_eq!(
            [file_size(0), file_size(812), file_size(2_500), file_size(13 << 20), file_size(u64::MAX)],
            ["0 B", "812 B", "2.4 KB", "13 MB", "16777216 TB"]
        );
    }
}
