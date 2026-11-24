//! Live session joining capture, transcription and storage (PRD 03).
//!
//! One [`Session`] runs one meeting: audio sources in, labeled transcript
//! lines out, the full transcript ready when [`Session::stop`] returns.
//!
//! ```text
//!  mneme-audio's Recorder           capture thread                 transcription thread
//!  (a thread per source)            "mneme-session"                "mneme-transcribe"
//!
//!  mic source ────┐  frames   ┌─► chunker(Me) ───┐  chunks   ┌──────────────────────────┐
//!                 ├──────────►│                  ├──────────►│ queue, oldest start first │
//!  system source ─┘  (mpsc)   └─► chunker(Them) ─┘           └────────────┬─────────────┘
//!                                 │                                       ▼
//!                                 └─► mic.wav, system.wav       transcriber.transcribe(chunk)
//!                                     (MNEME_KEEP_AUDIO=1)                 │ spans
//!                                                                          ▼
//!                                                               sink.append(line) per span
//!                                                               sink.finalize() at the end
//! ```
//!
//! # The parts
//!
//! | Piece | What it is |
//! | --- | --- |
//! | [`Session`] | The handle: `start`, `status`, `elapsed`, `stop`, `stop_or_give_up`, `request_stop`, `abandon_backlog`, `give_up_transcriber`, `wait`. `Send + Sync`. |
//! | [`SessionConfig`] | Language, the folder for kept audio, the backlog limit, chunker and recorder settings. |
//! | [`SessionStatus`] | A snapshot for the UI: state, meeting time, lines, backlog, each source's phase, drain progress. |
//! | [`SessionSummary`] | What `stop` returns: duration, line count, each source's outcome, every failure and every dropped chunk. |
//! | [`TranscriptSink`] | Where lines go. `mneme_store::TranscriptWriter` implements it. |
//!
//! Sources are [`mneme_audio::AudioSource`]s (the real microphone and system
//! audio, or fakes in tests), the transcriber is a
//! [`mneme_transcribe::Transcriber`] behind an `Arc`. Loading the model,
//! creating the meeting and building the sources are the caller's job.
//!
//! # Threads
//!
//! Plain threads and channels; no async runtime is needed or used.
//!
//! - **Capture thread.** Owns the [`mneme_audio::Recorder`] (which runs each
//!   source on a thread of its own and delivers one stream of frames) and
//!   the two chunkers. Every frame goes to its speaker's chunker; every
//!   chunk a chunker completes goes to the queue. It also notices a source
//!   that ended and flushes that speaker's chunker, and at the end it stops
//!   the recorder, flushes both chunkers, closes the queue, waits for the
//!   transcription thread and publishes the summary.
//! - **Transcription thread.** Takes chunks from the queue, the one with
//!   the earliest start first, and blocks in the transcriber for each (up
//!   to seconds). Each span becomes a line at the chunk's start plus the
//!   span's offset and is appended to the sink at once. When the queue is
//!   closed and empty it finalizes the sink.
//!
//! The caller's threads only ever set a flag, read a snapshot or wait for
//! the summary. `stop` can be called from any thread, twice, at the same
//! time, before a source has come up or while one is dying.
//!
//! # What happens when something fails
//!
//! | Failure | What the session does |
//! | --- | --- |
//! | A source fails to start, starts late or dies | The other source carries on. The dead source's chunker is flushed at once, so its last words are transcribed. Its phase and errors are in the status and the summary. |
//! | Every source has ended | The session finishes the transcript and ends by itself ([`EndReason::SourcesEnded`]); `stop` then returns the summary at once. |
//! | The transcriber fails on a chunk (or panics) | Tried once more. If it fails again, a [`NOT_TRANSCRIBED`] line marks the gap at the chunk's time, the failure is recorded with the chunk's speaker and time, and the next chunk is transcribed as usual. |
//! | A transcriber call never returns | [`Session::stop_or_give_up`] gives up on it after a limit: the chunk in work and the backlog are listed as dropped, the sink is finalized and the summary published. The stuck thread cannot be killed; see [`Session::give_up_transcriber`]. |
//! | The sink refuses a line (or panics) | Not retried, since the line may have been written. The line and the error are in the summary. |
//! | The transcriber is slower than real time | Nothing is dropped; the status shows the backlog, and `stop` takes as long as the backlog needs. |
//! | More than [`SessionConfig::max_backlog`] is waiting (two hours by default) | New chunks are dropped, counted and listed, until there is room again. |
//! | A thread of the session itself panics | `stop` still returns; the summary says what was lost. |
//!
//! # Audio retention
//!
//! A chunk's samples are freed as soon as the transcriber returns. Nothing
//! is written to disk, with one development exception: when the process
//! runs with `MNEME_KEEP_AUDIO=1` and the caller gave a folder
//! ([`SessionConfig::audio_dir`]), every frame is also written to `mic.wav`
//! and `system.wav` there.
//!
//! # Using it
//!
//! ```no_run
//! use std::sync::Arc;
//! use mneme_audio::{AudioSource, MicSource, SystemAudioSource};
//! use mneme_pipeline::{Session, SessionConfig, SessionState};
//! use mneme_store::{MeetingId, Store};
//! use mneme_transcribe::{Language, Transcriber, WhisperEngine};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let store = Store::open("/tmp/meetings")?;
//! # let id = MeetingId::new("id");
//! let engine = Arc::new(WhisperEngine::load("models/ggml-base.bin")?);
//! let sources: Vec<Box<dyn AudioSource>> =
//!     vec![Box::new(MicSource::new()), Box::new(SystemAudioSource::new())];
//! let config = SessionConfig {
//!     audio_dir: Some(store.meeting(&id)?.folder),
//!     ..SessionConfig::new(Language::auto())
//! };
//! let transcriber: Arc<dyn Transcriber> = engine.clone();
//! let session = Session::start(sources, transcriber, store.transcript_writer(&id)?, config)?;
//!
//! // About once a second, for the UI:
//! let status = session.status();
//! println!("{:?} {} s, {} lines", status.state, status.elapsed.as_secs(), status.lines);
//! if status.state == SessionState::Done { /* every source died: stop() returns at once */ }
//!
//! // When the user stops (off the UI thread: this waits for the backlog):
//! let summary = session.stop();
//! store.finish_meeting(&id, summary.duration.as_secs())?;
//! drop(session);
//! // The session no longer holds the engine. Drop it before the process
//! // exits, or whisper.cpp's Metal backend aborts.
//! drop(engine);
//! # Ok(())
//! # }
//! ```

mod capture;
mod config;
mod queue;
mod session;
mod shared;
mod sink;
mod status;
mod worker;

// Lets the unit tests share the integration tests' helpers, which name this
// crate from outside.
#[cfg(test)]
extern crate self as mneme_pipeline;
#[cfg(test)]
mod tests;

/// The text of the line written in place of a chunk the transcriber failed
/// on twice: `[00:01:23] Me: [not transcribed]`. It cannot be mistaken for
/// speech, because the Whisper engine removes every short bracketed tag like
/// it from what it transcribes.
pub const NOT_TRANSCRIBED: &str = "[not transcribed]";

pub use config::{DEFAULT_MAX_BACKLOG, KEEP_AUDIO_ENV, SessionConfig, keep_audio_enabled};
pub use session::{Session, StartError};
pub use sink::{SinkError, TranscriptSink};
pub use status::{
    ChunkCounts, DrainProgress, DrainSummary, DropReason, DroppedChunk, EndReason, KeptAudio,
    SessionState, SessionStatus, SessionSummary, SinkFailure, TranscriptionFailure,
};

// The types of other crates that appear in this crate's API.
pub use mneme_audio::{AudioError, AudioSource, RecorderOptions, SourcePhase, SourceStatus};
pub use mneme_transcribe::{ChunkerConfig, Language, TranscribeError, Transcriber};
