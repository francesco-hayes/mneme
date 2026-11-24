//! What a session reports: [`SessionStatus`] while it runs and
//! [`SessionSummary`] when it is over.

use std::path::PathBuf;
use std::time::Duration;

use mneme_audio::{AudioError, SourceStatus};
use mneme_core::{Segment, Speaker};
use mneme_transcribe::TranscribeError;

use crate::SinkError;

/// Where a session is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// No source has come up yet. A permission prompt may be open.
    Starting,
    /// At least one source has started.
    Recording,
    /// Capture is over or ending, and what is still queued is being
    /// transcribed. [`SessionStatus::drain`] tells how far along that is.
    Stopping,
    /// The transcript is complete and the summary is ready.
    /// [`Session::stop`](crate::Session::stop) returns at once.
    Done,
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// [`Session::stop`](crate::Session::stop) or
    /// [`request_stop`](crate::Session::request_stop) was called (or the
    /// session was dropped).
    StopRequested,
    /// Every source failed to start or ended by itself, so there was nothing
    /// left to record and the session finished the transcript without being
    /// asked.
    SourcesEnded,
    /// The capture thread panicked. What had been queued until then was
    /// still transcribed; [`SessionSummary::internal_errors`] says what
    /// happened.
    CaptureFailed,
}

/// How many chunks of speech went where.
///
/// Every chunk the chunkers cut is in exactly one place:
/// `cut == transcribed + failed + dropped`, plus what is still waiting or in
/// work while the session runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkCounts {
    /// Chunks cut by the two chunkers.
    pub cut: u64,
    /// Chunks the transcriber returned a result for, with or without text.
    pub transcribed: u64,
    /// Chunks the transcriber failed on twice. Each has a
    /// [`NOT_TRANSCRIBED`](crate::NOT_TRANSCRIBED) line in the transcript.
    pub failed: u64,
    /// Chunks that were never given to the transcriber. Each one is in
    /// [`SessionSummary::dropped`] with the reason.
    pub dropped: u64,
}

/// How far the transcription of the remaining audio has come, once capture
/// is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainProgress {
    /// Audio that was not transcribed yet when capture ended.
    pub total: Duration,
    /// Audio still not transcribed, the chunk in work included.
    pub remaining: Duration,
    /// Chunks still waiting, not counting the one in work.
    pub chunks_remaining: usize,
    /// How long the drain has been running. It stops counting when the
    /// session is done.
    pub running_for: Duration,
    /// [`Session::abandon_backlog`](crate::Session::abandon_backlog) was
    /// called.
    pub abandoned: bool,
}

/// A snapshot of a running session, from [`Session::status`](crate::Session::status).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatus {
    /// Where the session is in its life.
    pub state: SessionState,
    /// Why it ended; `None` until capture is over.
    pub end: Option<EndReason>,
    /// Meeting time: time since the sources' shared start instant. It stands
    /// still once capture is over.
    pub elapsed: Duration,
    /// Lines written to the sink so far.
    pub lines: u64,
    /// Audio waiting for the transcriber: how far behind it is. The chunk it
    /// is working on is not counted; see [`transcribing`](Self::transcribing).
    pub backlog: Duration,
    /// Chunks waiting, not counting the one in work.
    pub queued_chunks: usize,
    /// Length of the chunk the transcriber is working on, zero when it is
    /// idle.
    pub transcribing: Duration,
    /// Where the chunks went so far.
    pub chunks: ChunkCounts,
    /// Each source's phase, errors and frame counts, in the order the
    /// sources were given.
    pub sources: Vec<SourceStatus>,
    /// Set once capture is over.
    pub drain: Option<DrainProgress>,
    /// Lines the sink refused so far.
    pub sink_errors: u64,
    /// The latest failure of the transcriber or the sink, or the latest
    /// dropped chunk, as text.
    pub last_error: Option<String>,
    /// Set if the session can no longer produce a transcript because its
    /// transcription thread died. Capture goes on, but every chunk cut from
    /// now on is dropped (and listed in the summary): stop the session.
    pub fatal: Option<String>,
    /// Why the kept audio (`MNEME_KEEP_AUDIO`) is incomplete, if it is.
    pub audio_error: Option<String>,
}

impl SessionStatus {
    /// The status of the source labeled `speaker`.
    pub fn source(&self, speaker: Speaker) -> Option<&SourceStatus> {
        self.sources.iter().find(|s| s.speaker == speaker)
    }

    /// Audio cut into chunks and not transcribed yet: the backlog plus the
    /// chunk in work.
    pub fn untranscribed(&self) -> Duration {
        self.backlog + self.transcribing
    }
}

/// A chunk the transcriber failed on twice (it is tried once more after a
/// failure). Its audio is gone; a line reading
/// [`NOT_TRANSCRIBED`](crate::NOT_TRANSCRIBED) marks the gap in the
/// transcript, and the session went on with the next chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionFailure {
    /// Whose speech it was.
    pub speaker: Speaker,
    /// Meeting time at which the chunk starts.
    pub start_ms: u64,
    /// Length of the chunk.
    pub duration_ms: u64,
    /// What the transcriber returned the second time. A transcriber that
    /// panicked reads as `TranscribeError::Engine` with the panic message.
    pub error: TranscribeError,
}

/// A line the sink did not take. It was not offered again (it may have been
/// written before the failure), so the text is kept here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkFailure {
    /// The line, complete.
    pub segment: Segment,
    /// What the sink returned. A sink that panicked reads as an error with
    /// the panic message.
    pub error: SinkError,
}

/// Why a chunk was never transcribed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// The queue already held
    /// [`SessionConfig::max_backlog`](crate::SessionConfig::max_backlog) of
    /// audio when the chunk was cut.
    BacklogFull,
    /// [`Session::abandon_backlog`](crate::Session::abandon_backlog) was
    /// called and the chunk was still waiting when the session stopped.
    DrainAbandoned,
    /// The transcription thread died with the chunk still waiting. See
    /// [`SessionSummary::internal_errors`].
    WorkerDied,
    /// The transcriber was still working on this chunk when the session
    /// gave up on it
    /// ([`Session::give_up_transcriber`](crate::Session::give_up_transcriber)).
    TranscriberStuck,
}

/// A chunk of speech that was cut and then not transcribed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DroppedChunk {
    /// Whose speech it was.
    pub speaker: Speaker,
    /// Meeting time at which the chunk starts.
    pub start_ms: u64,
    /// Length of the chunk.
    pub duration_ms: u64,
    /// Why it was dropped.
    pub reason: DropReason,
}

/// What was left to transcribe when capture ended, and how long it took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainSummary {
    /// Audio not transcribed yet when capture ended, the chunk in work
    /// included.
    pub audio: Duration,
    /// Chunks waiting at that moment, not counting the one in work.
    pub chunks: usize,
    /// From the end of capture until the last chunk was dealt with.
    pub took: Duration,
    /// The drain was cut short by
    /// [`Session::abandon_backlog`](crate::Session::abandon_backlog) or
    /// [`Session::give_up_transcriber`](crate::Session::give_up_transcriber).
    pub abandoned: bool,
}

/// The audio files written under `MNEME_KEEP_AUDIO=1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptAudio {
    /// The folder they are in.
    pub dir: PathBuf,
    /// `mic.wav` and `system.wav`, in that order. Empty if they could not be
    /// created.
    pub files: Vec<PathBuf>,
    /// Set if creating, writing or finishing the files failed. Writing
    /// stopped at the first failure; the session itself was not affected.
    pub error: Option<String>,
}

/// What [`Session::stop`](crate::Session::stop) returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Why the session ended.
    pub end: EndReason,
    /// Meeting time at which capture was stopped: the duration of the
    /// meeting. (With sources that ended by themselves, the moment the last
    /// one was seen to have ended.)
    pub duration: Duration,
    /// Lines written to the sink.
    pub lines: u64,
    /// Where the chunks went.
    pub chunks: ChunkCounts,
    /// The final state of every source, in the order given, with all its
    /// errors. A source that failed to start, died or never came up shows
    /// here; see [`failed_sources`](Self::failed_sources).
    pub sources: Vec<SourceStatus>,
    /// Chunks the transcriber failed on, oldest failure first.
    pub transcription_errors: Vec<TranscriptionFailure>,
    /// Lines the sink did not take, with their text.
    pub sink_errors: Vec<SinkFailure>,
    /// `true` if the sink's `finalize` was called and succeeded.
    pub finalized: bool,
    /// What `finalize` returned, if it failed.
    pub finalize_error: Option<SinkError>,
    /// Chunks that were cut and never transcribed, with the reason.
    pub dropped: Vec<DroppedChunk>,
    /// What was left to transcribe when capture ended.
    pub drain: DrainSummary,
    /// From the stop request (or the moment the sources were found to have
    /// ended) until the session was done: stopping the devices, the drain
    /// and the sink's `finalize`.
    pub stopping_took: Duration,
    /// The files written under `MNEME_KEEP_AUDIO=1`; `None` when audio was
    /// not kept, which is always the case without the variable.
    pub audio: Option<KeptAudio>,
    /// Panics of the session's own threads. Empty unless something is wrong
    /// with the session itself.
    pub internal_errors: Vec<String>,
}

impl SessionSummary {
    /// The final state of the source labeled `speaker`.
    pub fn source(&self, speaker: Speaker) -> Option<&SourceStatus> {
        self.sources.iter().find(|s| s.speaker == speaker)
    }

    /// Every error of every source, with the speaker it belongs to.
    pub fn source_errors(&self) -> Vec<(Speaker, AudioError)> {
        self.sources
            .iter()
            .flat_map(|s| s.errors.iter().map(|e| (s.speaker, e.clone())))
            .collect()
    }

    /// The sources that failed to start, ended by themselves or never came
    /// up. (A fake source that has played all its input is one of them.)
    pub fn failed_sources(&self) -> Vec<&SourceStatus> {
        self.sources
            .iter()
            .filter(|s| s.phase.is_failure())
            .collect()
    }

    /// `true` if every chunk that was cut is in the transcript: nothing
    /// failed in the transcriber or the sink, nothing was dropped, the sink
    /// was finalized. Says nothing about the sources.
    pub fn transcript_is_complete(&self) -> bool {
        self.transcription_errors.is_empty()
            && self.sink_errors.is_empty()
            && self.dropped.is_empty()
            && self.finalized
            && self.internal_errors.is_empty()
    }
}
