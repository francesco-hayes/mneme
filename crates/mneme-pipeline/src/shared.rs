//! What the session handle, the capture thread and the transcription thread
//! share.
//!
//! Two locks, both held only for moments and never across a call into a
//! source, the transcriber or the sink:
//!
//! - `inner`: everything the status and the summary are made of;
//! - `queue`: the chunks waiting for the transcriber.
//!
//! Where both are needed they are taken in that order, `inner` first.

use std::any::Any;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mneme_audio::SourceStatus;
use mneme_transcribe::{Chunk, Language};

use crate::queue::ChunkQueue;
use crate::status::{
    ChunkCounts, DrainProgress, DrainSummary, DropReason, DroppedChunk, EndReason, KeptAudio,
    SessionState, SessionStatus, SessionSummary, SinkFailure, TranscriptionFailure,
};
use crate::{SinkError, TranscriptSink};

/// Locks a mutex whether or not a holder panicked. Everything behind the
/// session's locks is plain data that stays valid, and a panic on one thread
/// must never turn into a hang or a second panic on another.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The text of a panic, for an error message.
pub(crate) fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// Ways for the crate's own tests to break a thread on purpose. Always
/// `Hooks::default()` outside them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Hooks {
    /// The capture thread panics when it receives this frame (counted from
    /// 1).
    pub(crate) capture_panics_at_frame: Option<u64>,
    /// The transcription thread panics, outside everything that catches
    /// panics, when it takes this chunk (counted from 1).
    pub(crate) worker_panics_at_chunk: Option<u64>,
}

/// The moment capture ended and what was left to transcribe then.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrainStart {
    pub(crate) at: Instant,
    pub(crate) audio_ms: u64,
    pub(crate) chunks: usize,
}

/// Everything the status and the summary are made of.
pub(crate) struct Inner {
    pub(crate) state: SessionState,
    pub(crate) end: Option<EndReason>,
    /// Meeting time at which capture stopped; the clock stands still there.
    pub(crate) final_duration: Option<Duration>,
    pub(crate) sources: Vec<SourceStatus>,
    pub(crate) lines: u64,
    pub(crate) counts: ChunkCounts,
    pub(crate) transcription_errors: Vec<TranscriptionFailure>,
    pub(crate) sink_errors: Vec<SinkFailure>,
    pub(crate) dropped: Vec<DroppedChunk>,
    pub(crate) last_error: Option<String>,
    pub(crate) audio_error: Option<String>,
    pub(crate) internal_errors: Vec<String>,
    /// Set once the transcription thread is known to have died.
    pub(crate) fatal: Option<String>,
    /// When stopping began: the stop request, or the sources ending.
    pub(crate) stopping_since: Option<Instant>,
    pub(crate) drain: Option<DrainStart>,
    /// When the transcription thread was through with its last chunk.
    pub(crate) drained_at: Option<Instant>,
    /// What the sink's `finalize` returned, once it has been called.
    pub(crate) finalize: Option<Result<(), SinkError>>,
    /// Where audio is being kept, if it is. Only for the summary of a
    /// session whose capture thread died before it could finish the files.
    pub(crate) audio_dir: Option<PathBuf>,
    pub(crate) audio: Option<KeptAudio>,
    pub(crate) summary: Option<SessionSummary>,
}

pub(crate) struct Shared {
    /// The sources' shared start instant: the zero of meeting time.
    pub(crate) started_at: Instant,
    pub(crate) language: Language,
    pub(crate) max_backlog_ms: u64,
    pub(crate) stop: AtomicBool,
    pub(crate) abandon: AtomicBool,
    pub(crate) inner: Mutex<Inner>,
    /// Signalled, with `inner`, when the summary is published.
    pub(crate) done: Condvar,
    pub(crate) queue: Mutex<ChunkQueue>,
    /// Signalled, with `queue`, when a chunk is pushed, the queue is closed
    /// or the backlog is abandoned.
    pub(crate) queue_changed: Condvar,
}

impl Shared {
    pub(crate) fn new(
        started_at: Instant,
        language: Language,
        max_backlog: Duration,
        sources: Vec<SourceStatus>,
    ) -> Self {
        Shared {
            started_at,
            language,
            max_backlog_ms: u64::try_from(max_backlog.as_millis()).unwrap_or(u64::MAX),
            stop: AtomicBool::new(false),
            abandon: AtomicBool::new(false),
            inner: Mutex::new(Inner {
                state: SessionState::Starting,
                end: None,
                final_duration: None,
                sources,
                lines: 0,
                counts: ChunkCounts::default(),
                transcription_errors: Vec::new(),
                sink_errors: Vec::new(),
                dropped: Vec::new(),
                last_error: None,
                audio_error: None,
                internal_errors: Vec::new(),
                fatal: None,
                stopping_since: None,
                drain: None,
                drained_at: None,
                finalize: None,
                audio_dir: None,
                audio: None,
                summary: None,
            }),
            done: Condvar::new(),
            queue: Mutex::new(ChunkQueue::default()),
            queue_changed: Condvar::new(),
        }
    }

    pub(crate) fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// `true` once waiting chunks are to be thrown away instead of
    /// transcribed: the backlog was abandoned and the session is stopping.
    pub(crate) fn discarding(&self, queue: &ChunkQueue) -> bool {
        self.abandon.load(Ordering::SeqCst) && (self.stop_requested() || queue.is_closed())
    }

    /// Marks the session as stopping, once.
    pub(crate) fn begin_stopping(&self) {
        let mut inner = lock(&self.inner);
        if matches!(
            inner.state,
            SessionState::Starting | SessionState::Recording
        ) {
            inner.state = SessionState::Stopping;
        }
        inner.stopping_since.get_or_insert_with(Instant::now);
    }

    /// Queues a chunk for the transcriber. A chunk that cannot be queued is
    /// recorded as dropped, never forgotten.
    pub(crate) fn enqueue(&self, chunk: Chunk) {
        let mut inner = lock(&self.inner);
        let mut queue = lock(&self.queue);
        inner.counts.cut += 1;
        if inner.fatal.is_some() {
            // Nobody is left to transcribe it.
            record_dropped(&mut inner, &chunk, DropReason::WorkerDied);
            return;
        }
        if self.discarding(&queue) {
            // Cut while stopping with the backlog abandoned (the flush at
            // stop): it would only be thrown away again.
            record_dropped(&mut inner, &chunk, DropReason::DrainAbandoned);
            return;
        }
        match queue.push(chunk, self.max_backlog_ms) {
            Ok(()) => self.queue_changed.notify_all(),
            Err(chunk) => {
                record_dropped(&mut inner, &chunk, DropReason::BacklogFull);
                inner.last_error = Some(format!(
                    "the transcriber is more than {} s behind; speech at {} ms was not transcribed",
                    self.max_backlog_ms / 1000,
                    chunk.start_ms
                ));
            }
        }
    }

    /// Throws away what is waiting, if the backlog was abandoned and the
    /// session is stopping. Every chunk is listed as dropped. Called by
    /// whoever makes that condition true, so the audio is freed and the
    /// status is right at once, without waiting for the transcriber to
    /// come back from the chunk it is working on.
    pub(crate) fn drop_abandoned(&self) {
        let mut inner = lock(&self.inner);
        let mut queue = lock(&self.queue);
        if self.discarding(&queue) {
            for chunk in queue.take_all() {
                record_dropped(&mut inner, &chunk, DropReason::DrainAbandoned);
            }
        }
        // The transcription thread may be waiting for a chunk that will
        // now never come.
        self.queue_changed.notify_all();
    }

    /// Records that the transcription thread is gone and gives up on what
    /// it left behind. From here on new chunks are dropped as they are cut.
    pub(crate) fn worker_died(&self, message: &str) {
        let mut inner = lock(&self.inner);
        let mut queue = lock(&self.queue);
        if inner.fatal.is_none() {
            inner.fatal = Some(message.to_owned());
            inner.last_error = Some(message.to_owned());
        }
        if let Some(lost) = queue.abandon_work() {
            inner.counts.dropped += 1;
            inner.dropped.push(DroppedChunk {
                speaker: lost.speaker,
                start_ms: lost.start_ms,
                duration_ms: lost.duration_ms,
                reason: DropReason::WorkerDied,
            });
        }
        for chunk in queue.take_all() {
            record_dropped(&mut inner, &chunk, DropReason::WorkerDied);
        }
    }

    pub(crate) fn status(&self) -> SessionStatus {
        let inner = lock(&self.inner);
        let queue = lock(&self.queue);
        let remaining_ms = queue.waiting_ms() + queue.in_work_ms();
        SessionStatus {
            state: inner.state,
            end: inner.end,
            elapsed: inner
                .final_duration
                .unwrap_or_else(|| self.started_at.elapsed()),
            lines: inner.lines,
            backlog: Duration::from_millis(queue.waiting_ms()),
            queued_chunks: queue.len(),
            transcribing: Duration::from_millis(queue.in_work_ms()),
            chunks: inner.counts,
            sources: inner.sources.clone(),
            drain: inner.drain.map(|start| DrainProgress {
                total: Duration::from_millis(start.audio_ms),
                remaining: Duration::from_millis(remaining_ms),
                chunks_remaining: queue.len(),
                running_for: inner
                    .drained_at
                    .map_or_else(|| start.at.elapsed(), |end| end.duration_since(start.at)),
                abandoned: self.abandon.load(Ordering::SeqCst),
            }),
            sink_errors: inner.sink_errors.len() as u64,
            last_error: inner.last_error.clone(),
            fatal: inner.fatal.clone(),
            audio_error: inner.audio_error.clone(),
        }
    }

    /// Builds the summary from what the threads recorded, marks the session
    /// done and wakes everyone waiting for it. Does nothing the second time.
    pub(crate) fn publish_summary(&self) {
        let mut inner = lock(&self.inner);
        if inner.summary.is_some() {
            return;
        }
        let now = Instant::now();
        let duration = inner
            .final_duration
            .unwrap_or_else(|| self.started_at.elapsed());
        inner.final_duration = Some(duration);
        // If the transcription thread never got to say when it was through
        // (it died), the drain ended now.
        let drained_at = *inner.drained_at.get_or_insert(now);
        let drain = inner.drain.map_or_else(DrainSummary::default, |start| {
            let end = drained_at;
            DrainSummary {
                audio: Duration::from_millis(start.audio_ms),
                chunks: start.chunks,
                took: end.saturating_duration_since(start.at),
                abandoned: inner
                    .dropped
                    .iter()
                    .any(|chunk| chunk.reason == DropReason::DrainAbandoned),
            }
        });
        let (finalized, finalize_error) = match inner.finalize.clone() {
            Some(Ok(())) => (true, None),
            Some(Err(error)) => (false, Some(error)),
            None => (false, None),
        };
        let summary = SessionSummary {
            end: inner.end.unwrap_or(EndReason::CaptureFailed),
            duration,
            lines: inner.lines,
            chunks: inner.counts,
            sources: inner.sources.clone(),
            transcription_errors: inner.transcription_errors.clone(),
            sink_errors: inner.sink_errors.clone(),
            finalized,
            finalize_error,
            dropped: inner.dropped.clone(),
            drain,
            stopping_took: inner
                .stopping_since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since)),
            audio: inner.audio.clone(),
            internal_errors: inner.internal_errors.clone(),
        };
        inner.state = SessionState::Done;
        inner.summary = Some(summary);
        self.done.notify_all();
    }
}

/// Counts a chunk as dropped and lists it. The caller holds `inner`.
pub(crate) fn record_dropped(inner: &mut Inner, chunk: &Chunk, reason: DropReason) {
    inner.counts.dropped += 1;
    inner.dropped.push(DroppedChunk {
        speaker: chunk.speaker,
        start_ms: chunk.start_ms,
        duration_ms: chunk.duration_ms(),
        reason,
    });
}

/// The sink as the transcription thread holds it.
pub(crate) type BoxedSink = Box<dyn TranscriptSink>;
