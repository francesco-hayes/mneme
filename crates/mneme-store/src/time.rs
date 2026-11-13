//! When a meeting started: local wall-clock time plus its UTC offset.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Datelike, FixedOffset, Local, SecondsFormat, Timelike};

/// The moment a meeting started, as the wall clock showed it where the
/// meeting was recorded, together with that clock's UTC offset.
///
/// Keeping the offset is what lets a folder be named from local time
/// (`2026-10-27 1430 …`) while the frontmatter still pins an exact instant.
/// The value is whole seconds; anything finer is dropped on construction so
/// what is in memory equals what is read back from disk.
///
/// Nothing in this crate reads the system clock or time zone except
/// [`StartTime::now_local`]. Everything else takes a `StartTime`, so tests
/// pass fixed values and get the same result under any `TZ`.
///
/// Equality and ordering compare the instant, not the offset: 14:30 at
/// `+01:00` equals 13:30 at `+00:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StartTime(DateTime<FixedOffset>);

impl StartTime {
    /// The current time on the system clock, in the system time zone.
    pub fn now_local() -> Self {
        Self::from_chrono(Local::now().fixed_offset())
    }

    /// Parses an RFC 3339 date-time with an offset, such as
    /// `2026-10-27T14:30:00+01:00` or `2026-10-27T13:30:00Z`. This is the
    /// format of the `date` frontmatter key.
    pub fn parse_rfc3339(s: &str) -> Result<Self, ParseStartTimeError> {
        DateTime::parse_from_rfc3339(s)
            .map(Self::from_chrono)
            .map_err(|_| ParseStartTimeError {
                input: s.to_owned(),
            })
    }

    /// Formats as RFC 3339 with a numeric offset and whole seconds:
    /// `2026-10-27T14:30:00+01:00`. UTC is written `+00:00`.
    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Secs, false)
    }

    /// Seconds since the Unix epoch. Use it to sort meetings by start.
    pub fn unix_seconds(&self) -> i64 {
        self.0.timestamp()
    }

    /// The UTC offset in seconds, positive east of Greenwich.
    pub fn utc_offset_seconds(&self) -> i32 {
        self.0.offset().local_minus_utc()
    }

    /// The date and time part of a folder name: `2026-10-27 1430`.
    pub(crate) fn folder_prefix(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}{:02}",
            self.0.year(),
            self.0.month(),
            self.0.day(),
            self.0.hour(),
            self.0.minute()
        )
    }

    /// The title a meeting gets until the user names it:
    /// `Meeting, Oct 27 2:30 PM`.
    pub(crate) fn default_title(&self) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let (is_pm, hour) = self.0.hour12();
        format!(
            "Meeting, {} {} {}:{:02} {}",
            MONTHS[self.0.month0() as usize],
            self.0.day(),
            hour,
            self.0.minute(),
            if is_pm { "PM" } else { "AM" }
        )
    }

    fn from_chrono(value: DateTime<FixedOffset>) -> Self {
        Self(value.with_nanosecond(0).unwrap_or(value))
    }
}

/// Writes the same text as [`StartTime::to_rfc3339`].
impl fmt::Display for StartTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl FromStr for StartTime {
    type Err = ParseStartTimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_rfc3339(s)
    }
}

/// Returned when a string is not an RFC 3339 date-time with a UTC offset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid start time {input:?}: expected a date and time with a UTC offset, \
     like 2026-10-27T14:30:00+01:00"
)]
pub struct ParseStartTimeError {
    input: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> StartTime {
        s.parse().unwrap()
    }

    #[test]
    fn round_trips_through_rfc3339_keeping_the_offset() {
        for text in [
            "2026-10-27T14:30:00+01:00",
            "2026-10-27T09:30:00-07:00",
            "2026-03-01T00:05:59+05:30",
            "2026-12-31T23:59:59+00:00",
            "2026-06-15T12:00:00+13:45",
        ] {
            assert_eq!(at(text).to_rfc3339(), text);
            assert_eq!(at(text).to_string(), text);
        }
    }

    #[test]
    fn utc_is_accepted_as_z_and_written_as_a_numeric_offset() {
        let start = at("2026-10-27T13:30:00Z");
        assert_eq!(start.to_rfc3339(), "2026-10-27T13:30:00+00:00");
        assert_eq!(start.utc_offset_seconds(), 0);
    }

    #[test]
    fn fractions_of_a_second_are_dropped() {
        let start = at("2026-10-27T14:30:12.987654+01:00");
        assert_eq!(start.to_rfc3339(), "2026-10-27T14:30:12+01:00");
        assert_eq!(start, at("2026-10-27T14:30:12+01:00"));
    }

    #[test]
    fn rejects_text_without_an_offset_or_a_time() {
        for text in [
            "",
            "2026-10-27",
            "2026-10-27T14:30:00",
            "14:30",
            "Oct 27 2026 2:30 PM",
            "2026-13-01T00:00:00+00:00",
            "2026-10-27T25:00:00+00:00",
        ] {
            let err = text.parse::<StartTime>().unwrap_err();
            assert!(err.to_string().contains("UTC offset"), "{text:?}: {err}");
        }
    }

    #[test]
    fn equality_and_order_follow_the_instant() {
        let paris = at("2026-10-27T14:30:00+01:00");
        let utc = at("2026-10-27T13:30:00+00:00");
        let later = at("2026-10-27T06:31:00-07:00");

        assert_eq!(paris, utc);
        assert_eq!(paris.unix_seconds(), utc.unix_seconds());
        assert!(later > paris);
        assert_eq!(paris.utc_offset_seconds(), 3600);
        assert_eq!(later.utc_offset_seconds(), -7 * 3600);
    }

    #[test]
    fn folder_prefix_uses_the_local_wall_clock() {
        assert_eq!(
            at("2026-10-27T14:30:59+01:00").folder_prefix(),
            "2026-10-27 1430"
        );
        // Same instant, other side of midnight: the local date wins.
        assert_eq!(
            at("2026-10-27T23:05:00-07:00").folder_prefix(),
            "2026-10-27 2305"
        );
        assert_eq!(
            at("2026-10-28T06:05:00+00:00").folder_prefix(),
            "2026-10-28 0605"
        );
        assert_eq!(
            at("2026-01-02T03:04:00+00:00").folder_prefix(),
            "2026-01-02 0304"
        );
    }

    #[test]
    fn default_title_is_month_day_and_twelve_hour_time() {
        for (text, title) in [
            ("2026-10-27T14:30:00+01:00", "Meeting, Oct 27 2:30 PM"),
            ("2026-10-27T09:05:00-07:00", "Meeting, Oct 27 9:05 AM"),
            ("2026-01-01T00:00:00+00:00", "Meeting, Jan 1 12:00 AM"),
            ("2026-06-09T12:00:00+02:00", "Meeting, Jun 9 12:00 PM"),
            ("2026-12-31T23:59:00+09:00", "Meeting, Dec 31 11:59 PM"),
        ] {
            assert_eq!(at(text).default_title(), title);
        }
    }

    #[test]
    fn now_local_reads_the_system_clock_and_survives_a_round_trip() {
        let before = chrono::Utc::now().timestamp();
        let now = StartTime::now_local();
        let after = chrono::Utc::now().timestamp();

        assert!((before..=after).contains(&now.unix_seconds()));
        assert_eq!(now.to_rfc3339().parse::<StartTime>().unwrap(), now);
        // Whatever the zone, the text carries an explicit numeric offset.
        let text = now.to_rfc3339();
        let offset = &text[text.len() - 6..];
        assert!(offset.starts_with(['+', '-']), "{text}");
    }
}
