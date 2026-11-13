//! The transcription thread: takes chunks from the queue oldest first, runs
//! the transcriber on each, writes the lines, finalizes the sink at the end.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use mneme_core::Segment;
use mneme_transcribe::{Chunk, TranscribeError, Transcriber};

use crate::shared::{BoxedSink, Hooks, Shared, lock, panic_message};
use crate::status::{SinkFailure, TranscriptionFailure};
use crate::{SinkError, TranscriptSink};

/// Runs until the queue is closed and empty (or abandoned), then finalizes
/// the sink.
///
/// The transcriber and the sink are owned by this function, so they are
/// gone when the thread ends, however it ends: that is what lets the caller
/// drop the engine after `stop()`.
pub(crate) fn run(
    shared: Arc<Shared>,
    transcriber: Arc<dyn Transcriber>,
    mut sink: BoxedSink,
    hooks: Hooks,
) {
    let mut taken = 0_u64;
    while let Some(chunk) = next_chunk(&shared) {
        taken += 1;
        if hooks.worker_panics_at_chunk == Some(taken) {
            panic!("test hook: the transcription thread panics");
        }
        transcribe_chunk(&shared, transcriber.as_ref(), sink.as_mut(), chunk);
    }
    // Before anything that could still fail: the engine must not be kept
    // alive by a sink that is slow or stuck in `finalize`.
    drop(transcriber);
    lock(&shared.inner).drained_at = Some(Instant::now());

    let finalized = catch_unwind(AssertUnwindSafe(|| sink.finalize())).unwrap_or_else(|panic| {
        Err(SinkError::new(format!(
            "the sink panicked in finalize: {}",
            panic_message(panic.as_ref())
        )))
    });
    lock(&shared.inner).finalize = Some(finalized);
    drop(sink);
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

/// Transcribes one chunk and writes its lines. Nothing in here can take the
/// thread down: a transcriber or sink that fails or panics is recorded and
/// the next chunk gets its turn.
fn transcribe_chunk(
    shared: &Shared,
    transcriber: &dyn Transcriber,
    sink: &mut dyn TranscriptSink,
    chunk: Chunk,
) {
    let speaker = chunk.speaker;
    let start_ms = chunk.start_ms;
    let duration_ms = chunk.duration_ms();

    let result = catch_unwind(AssertUnwindSafe(|| {
        transcriber.transcribe(&chunk.samples, &shared.language)
    }));
    // Audio retention: the samples are freed as soon as the transcriber has
    // seen them, whatever it made of them.
    drop(chunk);

    let failure = match result {
        Ok(Ok(spans)) => {
            for span in spans {
                // The line's time is the chunk's start plus the span's
                // offset inside the chunk.
                append_line(shared, sink, span.into_segment(speaker, start_ms));
            }
            None
        }
        Ok(Err(error)) => Some(error),
        Err(panic) => Some(TranscribeError::Engine {
            message: format!(
                "the transcriber panicked: {}",
                panic_message(panic.as_ref())
            ),
        }),
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
