//! Whisper engine, silence chunker, model catalog and downloads (PRD 02).
//!
//! - [`Transcriber`] is the trait the live pipeline depends on: a slice of
//!   16 kHz mono samples in, [`TextSpan`]s with offsets inside the slice out.
//! - [`WhisperEngine`] implements it with whisper.cpp on the GPU (Metal).
//! - [`FakeTranscriber`] implements it with scripted text, for tests.
//! - [`Chunker`] cuts one speaker's stream of frames into [`Chunk`]s at
//!   pauses and drops silence, so the transcriber only ever gets speech.
//!
//! ```no_run
//! use mneme_core::{AudioFrame, Speaker};
//! use mneme_transcribe::{Chunk, Chunker, ChunkerConfig, Language, Transcriber, WhisperEngine};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Once per recording.
//! let engine = WhisperEngine::load("models/ggml-tiny.en.bin")?;
//! let language: Language = "auto".parse()?;
//! engine.check_language(&language)?;
//! let mut chunker = Chunker::new(Speaker::Me, ChunkerConfig::default());
//!
//! // Once per chunk. A line's time is the chunk's start plus the offset of
//! // the text inside the chunk.
//! let transcribe = |chunk: Chunk| -> Result<(), Box<dyn std::error::Error>> {
//!     for span in engine.transcribe(&chunk.samples, &language)? {
//!         let segment = span.into_segment(chunk.speaker, chunk.start_ms);
//!         println!("{} {}: {}", segment.start_ms, segment.speaker, segment.text);
//!     }
//!     Ok(())
//! };
//!
//! // Once per frame from the microphone.
//! # let frames: Vec<AudioFrame> = Vec::new();
//! for frame in &frames {
//!     for chunk in chunker.push(frame) {
//!         transcribe(chunk)?;
//!     }
//! }
//! // When the recording stops.
//! if let Some(chunk) = chunker.flush() {
//!     transcribe(chunk)?;
//! }
//! # Ok(())
//! # }
//! ```

mod chunker;
mod fake;
mod text;
mod transcriber;
mod whisper;

pub use chunker::{Chunk, Chunker, ChunkerConfig, ChunkerStats};
pub use fake::{FakeCall, FakeTranscriber};
pub use transcriber::{
    Language, ParseLanguageError, TextSpan, TranscribeError, Transcriber, duration_ms,
};
pub use whisper::{WhisperEngine, WhisperOptions};
