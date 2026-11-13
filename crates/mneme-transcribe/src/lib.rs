//! Whisper engine, silence chunker, model catalog and downloads (PRD 02).
//!
//! - [`Transcriber`] is the trait the live pipeline depends on: a slice of
//!   16 kHz mono samples in, [`TextSpan`]s with offsets inside the slice out.
//! - [`WhisperEngine`] implements it with whisper.cpp on the GPU (Metal).
//! - [`FakeTranscriber`] implements it with scripted text, for tests.
//!
//! ```no_run
//! use mneme_core::Speaker;
//! use mneme_transcribe::{Language, Transcriber, WhisperEngine};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Once per recording.
//! let engine = WhisperEngine::load("models/ggml-tiny.en.bin")?;
//! let language: Language = "auto".parse()?;
//! engine.check_language(&language)?;
//!
//! // Once per chunk. `chunk_start_ms` is the chunk's meeting time.
//! let (chunk_start_ms, samples) = (12_000, vec![0.0_f32; 16_000]);
//! for span in engine.transcribe(&samples, &language)? {
//!     let segment = span.into_segment(Speaker::Me, chunk_start_ms);
//!     println!("{} {}: {}", segment.start_ms, segment.speaker, segment.text);
//! }
//! # Ok(())
//! # }
//! ```

mod fake;
mod text;
mod transcriber;
mod whisper;

pub use fake::{FakeCall, FakeTranscriber};
pub use transcriber::{
    Language, ParseLanguageError, TextSpan, TranscribeError, Transcriber, duration_ms,
};
pub use whisper::{WhisperEngine, WhisperOptions};
