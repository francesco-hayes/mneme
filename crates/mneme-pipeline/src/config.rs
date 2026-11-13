//! [`SessionConfig`] and the `MNEME_KEEP_AUDIO` switch.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use mneme_audio::RecorderOptions;
use mneme_transcribe::{ChunkerConfig, Language};

/// The environment variable that makes a session keep the raw audio. Only
/// the exact value `1` turns it on.
pub const KEEP_AUDIO_ENV: &str = "MNEME_KEEP_AUDIO";

/// Default for [`SessionConfig::max_backlog`]: two hours of audio waiting,
/// which is an hour of both speakers talking without a break and about
/// 460 MB of samples.
pub const DEFAULT_MAX_BACKLOG: Duration = Duration::from_secs(2 * 60 * 60);

/// `true` if `MNEME_KEEP_AUDIO=1` is set for this process.
///
/// Read once by [`Session::start`](crate::Session::start). A development
/// switch: never on by default and not something the app offers.
pub fn keep_audio_enabled() -> bool {
    is_keep_audio_value(std::env::var_os(KEEP_AUDIO_ENV).as_deref())
}

fn is_keep_audio_value(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// Settings for [`Session::start`](crate::Session::start).
/// `SessionConfig::default()` is what the app uses, apart from `language`
/// and `audio_dir`.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionConfig {
    /// The language handed to the transcriber with every chunk. Default
    /// `auto`.
    pub language: Language,

    /// Where `mic.wav` and `system.wav` go **if** the process runs with
    /// `MNEME_KEEP_AUDIO=1`. Pass the meeting folder (from
    /// `store.meeting(&id)?.folder`). Default `None`.
    ///
    /// Without the variable the session never writes audio anywhere,
    /// whatever this is: the folder is not created and not touched. With the
    /// variable and `None` here, nothing is written either.
    pub audio_dir: Option<PathBuf>,

    /// The most audio that may wait for the transcriber, as a length of
    /// audio. Default [`DEFAULT_MAX_BACKLOG`].
    ///
    /// Below it nothing is ever dropped, however slow the transcriber. It
    /// exists for the transcriber that has stopped or is hopelessly behind:
    /// without a limit the queue grows by 64 kB per second of speech until
    /// the machine is out of memory. At the limit a new chunk is not queued;
    /// it is counted in the status and listed in the summary with its
    /// speaker and time ([`DropReason::BacklogFull`](crate::DropReason)), so
    /// the hole in the transcript is known. `Duration::MAX` removes the
    /// limit.
    pub max_backlog: Duration,

    /// Settings of the two chunkers. Default as the transcribe crate has it.
    pub chunker: ChunkerConfig,

    /// How the sources are started. Default as the audio crate has it: the
    /// first source gets a second's head start.
    pub recorder: RecorderOptions,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            language: Language::auto(),
            audio_dir: None,
            max_backlog: DEFAULT_MAX_BACKLOG,
            chunker: ChunkerConfig::default(),
            recorder: RecorderOptions::default(),
        }
    }
}

impl SessionConfig {
    /// The defaults with this language.
    pub fn new(language: Language) -> Self {
        SessionConfig {
            language,
            ..SessionConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_value_1_keeps_audio() {
        assert!(is_keep_audio_value(Some(OsStr::new("1"))));
        for other in ["", "0", "true", "yes", "11", " 1", "1 "] {
            assert!(!is_keep_audio_value(Some(OsStr::new(other))), "{other:?}");
        }
        assert!(!is_keep_audio_value(None));
    }

    #[test]
    fn the_default_keeps_no_audio_and_drops_nothing_for_two_hours() {
        let config = SessionConfig::default();
        assert_eq!(config.audio_dir, None);
        assert_eq!(config.max_backlog, Duration::from_secs(7200));
        assert!(config.language.is_auto());
        assert_eq!(
            SessionConfig::new(Language::english()).language,
            Language::english()
        );
    }
}
