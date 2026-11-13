//! Shared types for Mneme, defined once so audio capture, transcription,
//! storage and the live pipeline agree on them. See `prds/00-foundation.md`.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Sample rate of all audio passed between crates: 16 kHz mono `f32`, which
/// is what Whisper takes.
pub const SAMPLE_RATE: u32 = 16_000;

/// Who is speaking. Channels are the only speaker separation in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Speaker {
    /// The microphone channel.
    Me,
    /// The system audio channel.
    Them,
}

impl Speaker {
    /// The label used on disk and in the UI: `Me` or `Them`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Speaker::Me => "Me",
            Speaker::Them => "Them",
        }
    }
}

impl fmt::Display for Speaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned when a string is not exactly `Me` or `Them`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSpeakerError {
    input: String,
}

impl fmt::Display for ParseSpeakerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown speaker {:?}: expected `Me` or `Them`",
            self.input
        )
    }
}

impl Error for ParseSpeakerError {}

impl FromStr for Speaker {
    type Err = ParseSpeakerError;

    /// Parses exactly what [`Speaker`]'s `Display` writes. Case-sensitive, no
    /// trimming.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Me" => Ok(Speaker::Me),
            "Them" => Ok(Speaker::Them),
            _ => Err(ParseSpeakerError {
                input: s.to_owned(),
            }),
        }
    }
}

/// A run of audio samples from one speaker at [`SAMPLE_RATE`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioFrame {
    pub speaker: Speaker,
    /// Meeting time of the first sample: milliseconds since recording
    /// started, on one clock shared by both channels.
    pub start_ms: u64,
    /// Mono samples at [`SAMPLE_RATE`].
    pub samples: Vec<f32>,
}

/// One transcribed stretch of speech. Times are meeting time in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub speaker: Speaker,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Formats meeting time as `hh:mm:ss`, the only timestamp format on disk.
///
/// Milliseconds are truncated, not rounded. Hours are zero-padded to two
/// digits and grow past two when needed (`100:00:00`).
pub fn format_timestamp(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs / 60) % 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

/// Returned when a string is not a valid `hh:mm:ss` timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTimestampError {
    input: String,
}

impl fmt::Display for ParseTimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid timestamp {:?}: expected `hh:mm:ss`", self.input)
    }
}

impl Error for ParseTimestampError {}

/// Parses an `hh:mm:ss` timestamp into meeting time in milliseconds.
///
/// Accepts exactly what [`format_timestamp`] writes: two or more hour digits,
/// then two-digit minutes and seconds, each below 60, separated by colons.
/// Nothing else is tolerated: no whitespace, signs, fractions or missing
/// parts.
pub fn parse_timestamp(s: &str) -> Result<u64, ParseTimestampError> {
    let err = || ParseTimestampError {
        input: s.to_owned(),
    };

    let mut parts = s.split(':');
    let (Some(hours), Some(minutes), Some(seconds), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(err());
    };

    if hours.len() < 2 || minutes.len() != 2 || seconds.len() != 2 {
        return Err(err());
    }

    let hours = parse_digits(hours).ok_or_else(err)?;
    let minutes = parse_digits(minutes).ok_or_else(err)?;
    let seconds = parse_digits(seconds).ok_or_else(err)?;
    if minutes >= 60 || seconds >= 60 {
        return Err(err());
    }

    hours
        .checked_mul(3600)
        .and_then(|secs| secs.checked_add(minutes * 60 + seconds))
        .and_then(|secs| secs.checked_mul(1000))
        .ok_or_else(err)
}

/// Parses a non-empty run of ASCII digits. `u64::from_str` alone would also
/// accept a leading `+`.
fn parse_digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_is_16_khz() {
        assert_eq!(SAMPLE_RATE, 16_000);
    }

    #[test]
    fn speaker_displays_as_me_and_them() {
        assert_eq!(Speaker::Me.to_string(), "Me");
        assert_eq!(Speaker::Them.to_string(), "Them");
        assert_eq!(format!("{:>6}", Speaker::Me), "Me");
    }

    #[test]
    fn speaker_parses_what_it_displays() {
        for speaker in [Speaker::Me, Speaker::Them] {
            assert_eq!(speaker.to_string().parse::<Speaker>(), Ok(speaker));
        }
    }

    #[test]
    fn speaker_rejects_anything_else() {
        for input in ["", "me", "THEM", " Me", "Them ", "You"] {
            let err = input.parse::<Speaker>().unwrap_err();
            assert!(err.to_string().contains("expected `Me` or `Them`"));
        }
    }

    #[test]
    fn speaker_serializes_as_its_label() {
        assert_eq!(serde_json::to_string(&Speaker::Me).unwrap(), "\"Me\"");
        assert_eq!(serde_json::to_string(&Speaker::Them).unwrap(), "\"Them\"");
        assert_eq!(
            serde_json::from_str::<Speaker>("\"Them\"").unwrap(),
            Speaker::Them
        );
    }

    #[test]
    fn segment_round_trips_through_json() {
        let segment = Segment {
            speaker: Speaker::Them,
            start_ms: 83_000,
            end_ms: 86_500,
            text: "Let's start.".to_owned(),
        };
        let json = serde_json::to_string(&segment).unwrap();
        assert_eq!(
            json,
            r#"{"speaker":"Them","start_ms":83000,"end_ms":86500,"text":"Let's start."}"#
        );
        assert_eq!(serde_json::from_str::<Segment>(&json).unwrap(), segment);
    }

    #[test]
    fn audio_frame_round_trips_through_json() {
        let frame = AudioFrame {
            speaker: Speaker::Me,
            start_ms: 250,
            samples: vec![0.0, 0.5, -0.5],
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<AudioFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn format_pads_each_part_to_two_digits() {
        assert_eq!(format_timestamp(0), "00:00:00");
        assert_eq!(format_timestamp(83_000), "00:01:23");
        assert_eq!(format_timestamp(3_600_000), "01:00:00");
        assert_eq!(format_timestamp(86_399_000), "23:59:59");
    }

    #[test]
    fn format_truncates_milliseconds() {
        assert_eq!(format_timestamp(999), "00:00:00");
        assert_eq!(format_timestamp(1_000), "00:00:01");
        assert_eq!(format_timestamp(59_999), "00:00:59");
        assert_eq!(format_timestamp(60_000), "00:01:00");
    }

    #[test]
    fn format_handles_hours_above_99() {
        assert_eq!(format_timestamp(99 * 3_600_000 + 59_000), "99:00:59");
        assert_eq!(format_timestamp(100 * 3_600_000), "100:00:00");
        assert_eq!(
            format_timestamp(1234 * 3_600_000 + 5 * 60_000 + 6_000),
            "1234:05:06"
        );
    }

    #[test]
    fn format_does_not_overflow_at_u64_max() {
        let formatted = format_timestamp(u64::MAX);
        assert_eq!(formatted, "5124095576030:25:51");
        assert_eq!(parse_timestamp(&formatted), Ok(u64::MAX - u64::MAX % 1000));
    }

    #[test]
    fn parse_reads_what_format_writes() {
        assert_eq!(parse_timestamp("00:00:00"), Ok(0));
        assert_eq!(parse_timestamp("00:01:23"), Ok(83_000));
        assert_eq!(parse_timestamp("01:00:00"), Ok(3_600_000));
        assert_eq!(parse_timestamp("23:59:59"), Ok(86_399_000));
    }

    #[test]
    fn parse_handles_hours_above_99() {
        assert_eq!(parse_timestamp("100:00:00"), Ok(100 * 3_600_000));
        assert_eq!(
            parse_timestamp("1234:05:06"),
            Ok(1234 * 3_600_000 + 5 * 60_000 + 6_000)
        );
    }

    #[test]
    fn format_then_parse_round_trips_to_the_second() {
        for ms in [
            0,
            1,
            999,
            1_000,
            61_500,
            3_599_999,
            3_600_000,
            359_999_000,
            360_000_000,
            987_654_321_012,
        ] {
            assert_eq!(parse_timestamp(&format_timestamp(ms)), Ok(ms - ms % 1000));
        }
    }

    #[test]
    fn parse_rejects_malformed_input() {
        for input in [
            "",
            ":",
            "::",
            "00:00",
            "00:00:00:00",
            "0:00:00",
            "00:0:00",
            "00:00:0",
            "00:000:00",
            "00:00:000",
            "00:60:00",
            "00:00:60",
            "00:99:99",
            "aa:bb:cc",
            "00:0a:00",
            "-1:00:00",
            "+1:00:00",
            "00:+1:00",
            " 00:00:00",
            "00:00:00 ",
            "00: 0:00",
            "00:00:00.5",
            "00.00.00",
            "[00:00:00]",
            "٠٠:٠٠:٠٠",
        ] {
            let err = parse_timestamp(input).unwrap_err();
            assert!(
                err.to_string().contains("expected `hh:mm:ss`"),
                "unexpected message for {input:?}: {err}"
            );
        }
    }

    #[test]
    fn parse_rejects_hours_that_overflow() {
        // More hours than fit in u64 milliseconds.
        assert!(parse_timestamp("5124095576031:00:00").is_err());
        // More digits than fit in u64 at all.
        assert!(parse_timestamp("99999999999999999999999:00:00").is_err());
    }
}
