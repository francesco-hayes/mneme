//! The [`Transcriber`] trait and the types every transcriber shares.
//!
//! The live pipeline and its tests depend on this trait, not on Whisper.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use mneme_core::{SAMPLE_RATE, Segment, Speaker};

/// Turns a slice of audio into text.
///
/// Implementations are shared between threads (`Send + Sync`) and take
/// `&self`, so one loaded model can sit behind an `Arc` and be reused for
/// every chunk of a recording. A call blocks until the slice is transcribed,
/// which can take seconds: call it from a worker thread, not from an async
/// runtime's core threads.
pub trait Transcriber: Send + Sync {
    /// Transcribes `samples`: mono `f32` audio at
    /// [`SAMPLE_RATE`](mneme_core::SAMPLE_RATE), nominally in `-1.0..=1.0`.
    ///
    /// Returns the speech found, in order, with offsets measured from the
    /// start of `samples`. Audio with no speech gives an empty list, and so
    /// does an empty slice.
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError>;
}

impl<T: Transcriber + ?Sized> Transcriber for &T {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        (**self).transcribe(samples, language)
    }
}

impl<T: Transcriber + ?Sized> Transcriber for Box<T> {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        (**self).transcribe(samples, language)
    }
}

impl<T: Transcriber + ?Sized> Transcriber for Arc<T> {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        (**self).transcribe(samples, language)
    }
}

/// One stretch of transcribed speech inside the slice that was transcribed.
///
/// Offsets are milliseconds from the start of that slice, not meeting time.
/// [`TextSpan::into_segment`] converts to meeting time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    /// Where the speech starts, in milliseconds from the start of the slice.
    pub start_ms: u64,
    /// Where the speech ends, in milliseconds from the start of the slice.
    /// Never before `start_ms`.
    pub end_ms: u64,
    /// The text, trimmed and never empty.
    pub text: String,
}

impl TextSpan {
    /// Places this span on the meeting clock: `chunk_start_ms` is the meeting
    /// time of the first sample of the slice that was transcribed.
    pub fn into_segment(self, speaker: Speaker, chunk_start_ms: u64) -> Segment {
        Segment {
            speaker,
            start_ms: chunk_start_ms.saturating_add(self.start_ms),
            end_ms: chunk_start_ms.saturating_add(self.end_ms),
            text: self.text,
        }
    }
}

/// Length of `sample_count` samples at [`SAMPLE_RATE`], in milliseconds,
/// rounded up so a span can always end at the last sample.
pub fn duration_ms(sample_count: usize) -> u64 {
    let samples = u64::try_from(sample_count).unwrap_or(u64::MAX);
    samples
        .saturating_mul(1000)
        .div_ceil(u64::from(SAMPLE_RATE))
}

/// The language setting: detect it from the audio, or a fixed language.
///
/// Parses from and displays as `auto` or a lowercase language code such as
/// `en` or `de`, which is the form the settings file stores. Parsing checks
/// the shape of the code only; whether the loaded model knows the language is
/// checked by the transcriber (see `WhisperEngine::check_language`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Language {
    /// `None` means detect.
    code: Option<String>,
}

impl Language {
    /// Detect the language from the audio.
    pub const fn auto() -> Self {
        Language { code: None }
    }

    /// English, the only language `*.en` models take.
    pub fn english() -> Self {
        Language {
            code: Some("en".to_owned()),
        }
    }

    /// True for the `auto` setting.
    pub fn is_auto(&self) -> bool {
        self.code.is_none()
    }

    /// The language code, or `None` for `auto`.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// `auto` or the language code.
    pub fn as_str(&self) -> &str {
        self.code.as_deref().unwrap_or("auto")
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Returned when a string is neither `auto` nor shaped like a language code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLanguageError {
    input: String,
}

impl fmt::Display for ParseLanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid language {:?}: expected `auto` or a two or three letter language code such as `en`",
            self.input
        )
    }
}

impl Error for ParseLanguageError {}

impl FromStr for Language {
    type Err = ParseLanguageError;

    /// Accepts `auto` or a two or three letter code. Surrounding whitespace
    /// and letter case are ignored.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        if normalized == "auto" {
            return Ok(Language::auto());
        }
        let is_code = (2..=3).contains(&normalized.len())
            && normalized.bytes().all(|b| b.is_ascii_lowercase());
        if is_code {
            Ok(Language {
                code: Some(normalized),
            })
        } else {
            Err(ParseLanguageError {
                input: s.to_owned(),
            })
        }
    }
}

/// Why a model could not be loaded or a slice could not be transcribed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscribeError {
    /// There is no file at the model path.
    ModelNotFound { path: PathBuf },
    /// The file exists but could not be loaded as a Whisper model.
    ModelLoad { path: PathBuf, message: String },
    /// An English-only model (`*.en`) was asked for another language.
    EnglishOnlyModel { requested: String },
    /// The model does not know this language code.
    UnsupportedLanguage { code: String },
    /// The engine failed while transcribing.
    Engine { message: String },
}

impl fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscribeError::ModelNotFound { path } => {
                write!(f, "model file not found: {}", path.display())
            }
            TranscribeError::ModelLoad { path, message } => {
                write!(f, "could not load model {}: {message}", path.display())
            }
            TranscribeError::EnglishOnlyModel { requested } => write!(
                f,
                "this model only transcribes English, but the language is set to `{requested}`; \
                 set the language to `en` or `auto`, or pick a multilingual model"
            ),
            TranscribeError::UnsupportedLanguage { code } => {
                write!(f, "`{code}` is not a language this model knows")
            }
            TranscribeError::Engine { message } => write!(f, "transcription failed: {message}"),
        }
    }
}

impl Error for TranscribeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_parses_auto_and_codes() {
        assert_eq!("auto".parse::<Language>(), Ok(Language::auto()));
        assert_eq!(" Auto ".parse::<Language>(), Ok(Language::auto()));
        assert_eq!("en".parse::<Language>(), Ok(Language::english()));
        assert_eq!("EN".parse::<Language>(), Ok(Language::english()));
        assert_eq!("de".parse::<Language>().unwrap().code(), Some("de"));
        assert_eq!("haw".parse::<Language>().unwrap().code(), Some("haw"));
    }

    #[test]
    fn language_rejects_anything_else() {
        for input in ["", " ", "e", "engl", "english", "e1", "en-US", "en_US", "é"] {
            let err = input.parse::<Language>().unwrap_err();
            assert!(
                err.to_string().contains("expected `auto`"),
                "unexpected message for {input:?}: {err}"
            );
        }
    }

    #[test]
    fn language_displays_what_it_parses() {
        for text in ["auto", "en", "de", "yue"] {
            let language: Language = text.parse().unwrap();
            assert_eq!(language.to_string(), text);
            assert_eq!(language.as_str(), text);
        }
        assert!(Language::auto().is_auto());
        assert!(!Language::english().is_auto());
        assert_eq!(Language::default(), Language::auto());
    }

    #[test]
    fn duration_rounds_up_to_the_millisecond() {
        assert_eq!(duration_ms(0), 0);
        assert_eq!(duration_ms(1), 1);
        assert_eq!(duration_ms(16), 1);
        assert_eq!(duration_ms(17), 2);
        assert_eq!(duration_ms(16_000), 1000);
        assert_eq!(duration_ms(16_000 * 30), 30_000);
    }

    #[test]
    fn span_moves_onto_the_meeting_clock() {
        let span = TextSpan {
            start_ms: 500,
            end_ms: 2_250,
            text: "Let's start.".to_owned(),
        };
        assert_eq!(
            span.into_segment(Speaker::Them, 83_000),
            Segment {
                speaker: Speaker::Them,
                start_ms: 83_500,
                end_ms: 85_250,
                text: "Let's start.".to_owned(),
            }
        );
    }

    #[test]
    fn english_only_error_says_what_to_do() {
        let message = TranscribeError::EnglishOnlyModel {
            requested: "de".to_owned(),
        }
        .to_string();
        assert!(message.contains("only transcribes English"));
        assert!(message.contains("`de`"));
    }
}
