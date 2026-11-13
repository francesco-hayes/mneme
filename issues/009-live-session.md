# 009: Live session: capture to transcript

Status: todo · PRD: [03](../prds/03-live-pipeline.md) · Lane: pipeline · Depends on: 004, 006, 011 · Owns: `crates/mneme-pipeline/`

## Goal

One `Session` that runs a meeting: two sources in, labeled transcript lines appended as they are transcribed, full transcript ready when `stop()` returns.

## Scope

1. **`Session::start`** takes two `AudioSource`s, a `Transcriber`, a language, and a transcript sink (trait; the store's transcript writer implements it or is adapted to it). Starts both sources on one shared instant, one chunker per speaker, one worker draining a queue oldest-first.
2. Each segment becomes a line with timestamp = chunk `start_ms` + segment offset, appended at once through the sink.
3. **`Session::stop`**: stop sources, flush chunkers, drain the queue, tell the sink to finalize (sort), return a summary (duration, line count, errors).
4. **Status**: elapsed time, line count, backlog (seconds of audio waiting). Observable without blocking the worker.
5. **Failure handling**: one source failing does not stop the other; a transcription error on one chunk is recorded and the session continues; nothing is dropped when the worker falls behind.
6. **Audio retention**: samples are freed after transcription. With `MNEME_KEEP_AUDIO=1`, also write `mic.wav` and `system.wav` to a given folder. Off by default.
7. The transcriber runs off the async runtime's core threads (it blocks for seconds). Loading the model is the caller's job; the session receives a ready transcriber.

## Notes from earlier issues

- From 005: a `WhisperEngine` still alive when the process exits aborts it (a Metal assert in whisper.cpp). The session must never hold the engine in a static, and `stop()` must leave nothing that keeps it alive; the caller drops the engine after stop.
- From 005: `Transcriber::transcribe(&self, samples, &Language) -> Result<Vec<TextSpan>, TranscribeError>`; `TextSpan::into_segment(speaker, chunk_start_ms)` gives the `Segment`. `FakeTranscriber` scripts text, errors and a per-call delay.
- From 005: the first load of a model can take about 7 s while Metal compiles shaders. Loading stays the caller's job, off the UI thread.

- From 011: take the writer from `Store::transcript_writer(&id)` (it is `Send + 'static`) and move it to the worker; the sink adapter is `append(&segment)` and `finalize()`. `append` syncs to disk (about 4 ms). Do not retry an `append` that errored: the line may already be written. `finalize` and the store's `finish_meeting` can run in either order. For `MNEME_KEEP_AUDIO`, take the folder from `store.meeting(&id)`, not `writer.path()`.
- From 002: the frame channel is `std::sync::mpsc` (`frame_channel()`); clone one sender into both sources. A source drops its sender when it ends, so the receiver's iterator ends when both are done. Poll `is_running()` and `take_errors()` for failures.

- From 006: one `Chunker` per speaker (`push(&frame) -> Vec<Chunk>`, `flush() -> Option<Chunk>`). Call `flush()` at stop and also when a source ends or fails, or its last words are lost. Chunks arrive up to 28 s after their `start_ms`, so the two speakers' chunks reach the queue in completion order; the sort at stop handles it. The engine can return no spans for a chunk; that is not an error. Backlog is the sum of `chunk.duration_ms()` over the queue. Write `MNEME_KEEP_AUDIO` WAVs from frames, not chunks, because the chunker drops silence.

## Acceptance criteria

All with fake sources and a fake transcriber unless noted:

- [ ] Start, feed two speakers, stop: lines from both speakers, timestamps correct, sink finalized once.
- [ ] A slow fake transcriber builds a backlog; `stop()` returns only after every chunk is transcribed.
- [ ] One source errors mid-session; the other speaker's lines keep arriving; the error is in the summary.
- [ ] A transcriber error on one chunk does not end the session.
- [ ] No WAV files written by default; both written with the env var set.
- [ ] End-to-end test with the real engine and the speech WAV on both channels, skipped when the model is absent.
- [ ] `cargo test -p mneme-pipeline` and clippy pass.

## Manual checks

- [ ] A 30 minute real call gives a full, ordered transcript at stop, with Me and Them correct (with headphones).

## Result

_Filled in by the implementer._
