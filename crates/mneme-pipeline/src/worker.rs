//! The transcription thread: takes chunks from the queue oldest first, runs
//! the transcriber on each, writes the lines, finalizes the sink at the end.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use mneme_core::Segment;
use mneme_transcribe::{Chunk, TextSpan, TranscribeError, Transcriber};

use crate::shared::{Hooks, Shared, SinkSlot, lock, panic_message};
use crate::status::{SinkFailure, TranscriptionFailure};
use crate::{NOT_TRANSCRIBED, SinkError, TranscriptSink};

/// Runs until the queue is closed and empty (or abandoned), then finalizes
/// the sink.
///
/// The transcriber is owned by this function, so it is gone when the thread
/// ends, however it ends: that is what lets the caller drop the engine
/// after `stop()`. The sink is in `slot`; if the session gave up on this
/// thread, the capture thread has taken it and finalized it, and nothing
/// here touches it again.
pub(crate) fn run(
    shared: Arc<Shared>,
    transcriber: Arc<dyn Transcriber>,
    slot: SinkSlot,
    hooks: Hooks,
) {
    let mut taken = 0_u64;
    while let Some(chunk) = next_chunk(&shared) {
        taken += 1;
        if hooks.worker_panics_at_chunk == Some(taken) {
            panic!("test hook: the transcription thread panics");
        }
        transcribe_chunk(&shared, transcriber.as_ref(), &slot, chunk);
    }
    // Before anything that could still fail: the engine must not be kept
    // alive by a sink that is slow or stuck in `finalize`.
    drop(transcriber);

    // Held through `finalize`, so the capture thread cannot take the sink
    // half-way through.
    let mut slot = lock(&slot);
    let Some(sink) = slot.as_mut() else {
        // Given up on: the capture thread finalized the sink.
        return;
    };
    lock(&shared.inner).drained_at = Some(Instant::now());
    let finalized = catch_unwind(AssertUnwindSafe(|| sink.finalize())).unwrap_or_else(|panic| {
        Err(SinkError::new(format!(
            "the sink panicked in finalize: {}",
            panic_message(panic.as_ref())
        )))
    });
    lock(&shared.inner).finalize = Some(finalized);
    // Dropped here, still under the lock.
    slot.take();
}

/// Blocks until there is a chunk to transcribe. `None` when the queue is
/// closed and nothing is left, or what is left has been abandoned.
fn next_chunk(shared: &Shared) -> Option<Chunk> {
    let mut queue = lock(&shared.queue);
    loop {
        // With the backlog abandoned nothing more is taken; whoever
        // abandoned it has emptied the queue or is about to.
        if !shared.discarding(&queue)
            && let Some(chunk) = queue.pop_oldest()
        {
            return Some(chunk);
        }
        if queue.is_closed() {
            break;
        }
        queue = shared
            .queue_changed
            .wait(queue)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    drop(queue);
    // Closed between an abandon request and its sweep of the queue: make
    // sure nothing is left unaccounted for.
    shared.drop_abandoned();
    None
}

/// One transcriber call, with the time it started kept for
/// [`Session::stop_or_give_up`](crate::Session::stop_or_give_up). Nothing
/// it does can take the thread down: a panic comes back as an error.
fn call(
    shared: &Shared,
    transcriber: &dyn Transcriber,
    chunk: &Chunk,
) -> Result<Vec<TextSpan>, TranscribeError> {
    lock(&shared.inner).call_started = Some(Instant::now());
    let result = catch_unwind(AssertUnwindSafe(|| {
        transcriber.transcribe(&chunk.samples, &shared.language)
    }));
    lock(&shared.inner).call_started = None;
    result.unwrap_or_else(|panic| {
        Err(TranscribeError::Engine {
            message: format!(
                "the transcriber panicked: {}",
                panic_message(panic.as_ref())
            ),
        })
    })
}

/// Transcribes one chunk and writes its lines. A chunk the transcriber
/// fails on (or panics on) is tried once more; if that fails too, a line
/// marking the gap ([`NOT_TRANSCRIBED`]) is written in its place and the
/// failure is recorded. Either way the next chunk gets its turn.
fn transcribe_chunk(shared: &Shared, transcriber: &dyn Transcriber, slot: &SinkSlot, chunk: Chunk) {
    let speaker = chunk.speaker;
    let start_ms = chunk.start_ms;
    let duration_ms = chunk.duration_ms();

    let mut result = call(shared, transcriber, &chunk);
    // Not when the session is giving up the audio still waiting, or this
    // thread: a second try would only make the user wait longer.
    if result.is_err() && !shared.giving_up() && !shared.discarding(&lock(&shared.queue)) {
        result = call(shared, transcriber, &chunk);
    }
    // Audio retention: the samples are freed as soon as the transcriber has
    // seen them, whatever it made of them.
    drop(chunk);

    // Held until the chunk is accounted for, so the capture thread cannot
    // give up on this thread half-way through a chunk.
    let mut slot = lock(slot);
    let Some(sink) = slot.as_mut() else {
        // The session gave up on this thread while the call ran: the chunk
        // is listed as dropped and the sink is finalized. Nothing to write.
        return;
    };
    let failure = match result {
        Ok(spans) => {
            for span in spans {
                // The line's time is the chunk's start plus the span's
                // offset inside the chunk.
                append_line(shared, sink.as_mut(), span.into_segment(speaker, start_ms));
            }
            None
        }
        Err(error) => {
            // The gap shows in the transcript, at the time of the speech
            // that is missing.
            let gap = Segment {
                speaker,
                start_ms,
                end_ms: start_ms + duration_ms,
                text: NOT_TRANSCRIBED.to_owned(),
            };
            append_line(shared, sink.as_mut(), gap);
            Some(error)
        }
    };

    let mut inner = lock(&shared.inner);
    let mut queue = lock(&shared.queue);
    match failure {
        None => inner.counts.transcribed += 1,
        Some(error) => {
            inner.counts.failed += 1;
            inner.last_error = Some(format!(
                "{speaker}'s speech at {start_ms} ms was not transcribed: {error}"
            ));
            inner.transcription_errors.push(TranscriptionFailure {
                speaker,
                start_ms,
                duration_ms,
                error,
            });
        }
    }
    queue.finish_work();
}

/// Offers one line to the sink, once.
fn append_line(shared: &Shared, sink: &mut dyn TranscriptSink, segment: Segment) {
    let result = catch_unwind(AssertUnwindSafe(|| sink.append(&segment))).unwrap_or_else(|panic| {
        Err(SinkError::new(format!(
            "the sink panicked in append: {}",
            panic_message(panic.as_ref())
        )))
    });
    let mut inner = lock(&shared.inner);
    match result {
        Ok(true) => inner.lines += 1,
        // The sink had nothing to write for it.
        Ok(false) => {}
        Err(error) => {
            // Not offered again: the line may be in the sink already.
            inner.last_error = Some(format!(
                "a transcript line at {} ms could not be written: {error}",
                segment.start_ms
            ));
            inner.sink_errors.push(SinkFailure { segment, error });
        }
    }
}
