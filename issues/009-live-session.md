# 009: Live session: capture to transcript

Status: done · PRD: [03](../prds/03-live-pipeline.md) · Lane: pipeline · Depends on: 004, 006, 011 · Owns: `crates/mneme-pipeline/`

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

- From 004: use `mneme_audio::Recorder` for "start both on one clock, one stream of frames, stop both, per-source failures": `let (recorder, frames) = Recorder::start(vec![mic, system]);` with the mic first. `frames` is a std `mpsc` receiver; read it on a blocking thread. Poll `recorder.status()`: when a source's phase becomes `Ended` or `FailedToStart`, drain, then flush that speaker's chunker. At stop: `let report = recorder.stop();`, drain `frames.iter()` (it ends), flush both chunkers. `report.errors()` and `report.duration` feed the summary. `Recorder::start` never blocks, but `stop()` waits for devices to close. A fake source that runs out reads as `Ended` and `is_failure()` is true for it, so compare phases in tests. For `MNEME_KEEP_AUDIO` use `RecordingWriter` (`write` every frame, `finish(report.duration)`). Frame channels are unbounded (about 64 kB per second per source if the consumer stalls).
- The real mic and tap have each run once or never on hardware; the session must treat a failed or late source as a normal case.

## Acceptance criteria

All with fake sources and a fake transcriber unless noted:

- [x] Start, feed two speakers, stop: lines from both speakers, timestamps correct, sink finalized once.
- [x] A slow fake transcriber builds a backlog; `stop()` returns only after every chunk is transcribed.
- [x] One source errors mid-session; the other speaker's lines keep arriving; the error is in the summary.
- [x] A transcriber error on one chunk does not end the session.
- [x] No WAV files written by default; both written with the env var set.
- [x] End-to-end test with the real engine and the speech WAV on both channels, skipped when the model is absent.
- [x] `cargo test -p mneme-pipeline` and clippy pass.

## Manual checks

- [ ] A 30 minute real call gives a full, ordered transcript at stop, with Me and Them correct (with headphones).

## Result

Implemented 2026-11-12 on macOS 27.0.1, Apple Silicon, rustc 1.100.0. Everything is in `crates/mneme-pipeline/`; nothing else was edited except this file. `Cargo.lock` changed by itself (the crate's new path dependencies and `tempfile` as a dev-dependency).

### Public API (`mneme_pipeline::…`)

```rust
Session::start(
    sources: Vec<Box<dyn AudioSource>>,     // 1 or 2, different speakers, mic first
    transcriber: Arc<dyn Transcriber>,      // a loaded model
    sink: impl TranscriptSink + 'static,    // store.transcript_writer(&id)? works as it is
    config: SessionConfig,
) -> Result<Session, StartError>            // returns at once; NoSources | DuplicateSpeaker | Thread

impl Session {                              // Send + Sync, every method takes &self
    fn status(&self) -> SessionStatus;      // never waits for a source, the transcriber or the sink
    fn elapsed(&self) -> Duration;          // meeting time, the clock of segment start_ms; stands still after stop
    fn started_at(&self) -> Instant;
    fn is_done(&self) -> bool;
    fn request_stop(&self);                 // non-blocking; status reads Stopping at once
    fn abandon_backlog(&self);              // non-blocking; see "Stop"
    fn stop(&self) -> SessionSummary;       // blocks until the transcript is complete
    fn wait(&self) -> SessionSummary;       // blocks until done without asking for a stop
    fn wait_timeout(&self, Duration) -> Option<SessionSummary>;
}                                           // Drop = stop() and wait

struct SessionConfig {                      // Default; SessionConfig::new(language)
    language: Language,                     // default auto
    audio_dir: Option<PathBuf>,             // where mic.wav/system.wav go IF MNEME_KEEP_AUDIO=1
    max_backlog: Duration,                  // default 2 h of queued audio (about 460 MB)
    chunker: ChunkerConfig,
    recorder: RecorderOptions,
}

struct SessionStatus {
    state: SessionState,                    // Starting | Recording | Stopping | Done
    end: Option<EndReason>,                 // StopRequested | SourcesEnded | CaptureFailed
    elapsed: Duration, lines: u64,
    backlog: Duration,                      // audio waiting in the queue
    queued_chunks: usize, transcribing: Duration,   // the chunk in work
    chunks: ChunkCounts,                    // cut, transcribed, failed, dropped
    sources: Vec<mneme_audio::SourceStatus>,// phase and errors per source
    drain: Option<DrainProgress>,           // total, remaining, chunks_remaining, running_for, abandoned
    sink_errors: u64, last_error: Option<String>, fatal: Option<String>, audio_error: Option<String>,
}

struct SessionSummary {
    end: EndReason, duration: Duration, lines: u64, chunks: ChunkCounts,
    sources: Vec<SourceStatus>,                       // per-source outcome with all errors
    transcription_errors: Vec<TranscriptionFailure>,  // speaker, start_ms, duration_ms, error
    sink_errors: Vec<SinkFailure>,                    // the whole segment and the error
    finalized: bool, finalize_error: Option<SinkError>,
    dropped: Vec<DroppedChunk>,                       // speaker, start_ms, duration_ms, reason
    drain: DrainSummary,                              // audio, chunks, took, abandoned
    stopping_took: Duration,
    audio: Option<KeptAudio>,                         // None unless MNEME_KEEP_AUDIO=1
    internal_errors: Vec<String>,
}   // helpers: source(speaker), source_errors(), failed_sources(), transcript_is_complete()

trait TranscriptSink: Send {
    fn append(&mut self, segment: &Segment) -> Result<bool, SinkError>;  // Ok(false): nothing to write
    fn finalize(&mut self) -> Result<(), SinkError>;
}
impl TranscriptSink for mneme_store::TranscriptWriter   // and for Box<T>
```

Also exported: `KEEP_AUDIO_ENV`, `keep_audio_enabled()`, `DEFAULT_MAX_BACKLOG`, and re-exports of the audio and transcribe types that appear above.

### Threading model

Plain `std` threads, two mutexes and two condvars. No async runtime.

```text
 Recorder (mneme-audio)        capture thread "mneme-session"        transcription thread "mneme-transcribe"
 one thread per source
 mic ─────┐ frames (mpsc) ┌─► chunker(Me) ───┐ chunks  ┌──────────────────────────┐
          ├──────────────►│                  ├────────►│ queue, oldest start first │─► transcribe ─► sink.append per span
 system ──┘               └─► chunker(Them) ─┘         └──────────────────────────┘   … sink.finalize at the end
                           └─► mic.wav / system.wav (MNEME_KEEP_AUDIO=1 only)
```

- The capture thread owns the `Recorder` and both chunkers. At the end it stops the recorder, reads the frames to the end, flushes both chunkers, closes the queue, joins the transcription thread and publishes the summary.
- The transcription thread owns the transcriber reference and the sink, and drops both before it ends.
- Callers only set a flag, read a snapshot under a short lock, or wait on a condvar for the summary. `stop()` works from any thread, twice, concurrently, while sources are still starting, and while a source is dying.

### Design decisions

| Question | Choice |
| --- | --- |
| Threads or async | Std threads. The frame channel is std mpsc and the transcriber blocks; nothing here gains from a runtime, and the caller is not forced to have one. |
| What `start` takes | Boxed sources (a `Vec`, so mic-only works), `Arc<dyn Transcriber>`, a sink by value, a config holding the language. |
| Store dependency | The crate depends on `mneme-store` only for `impl TranscriptSink for TranscriptWriter`. The orphan rule means a caller could not write that impl without a wrapper type. |
| Timestamps and order | Line time is `span.into_segment(chunk.speaker, chunk.start_ms)`. The queue is a `BTreeMap` keyed by `(start_ms, arrival)`, served smallest first. Lines are appended in the order transcribed; the sink's `finalize` sorts. |
| Backlog | Below the limit nothing is dropped. `status.backlog` is the audio waiting; `transcribing` is the chunk in work. |
| Backlog ceiling | `max_backlog`, default 2 h of queued audio. At the ceiling a new chunk is not queued; it is counted in `chunks.dropped`, named in `last_error` at once and listed in `summary.dropped` with `BacklogFull`. Room returns as the transcriber catches up. `Duration::MAX` removes the limit. This is a deliberate step past the PRD's "nothing is dropped": it only acts when the transcriber has stalled or is hours behind, and it trades a reported hole for not exhausting memory. |
| Stop | As listed under the threading model. Takes as long as the backlog; `status.drain` shows remaining audio while it runs. |
| Abandoning the drain | `abandon_backlog()`: once the session is stopping, every waiting chunk is thrown away and listed (`DrainAbandoned`); the chunk in work is finished and written; the sink is still finalized. Lost: the speech in the listed chunks. A transcriber call cannot be interrupted, so `stop` can still take one chunk's time. |
| Both sources ended | The session finishes the transcript and ends by itself (`EndReason::SourcesEnded`, state `Done`). `stop()` afterwards returns the summary at once. |
| Source fails, starts late, dies | The other carries on. The dead source's chunker is flushed as soon as its phase leaves starting/running. Phase and errors are in the status and the summary. A source stuck in `start` at stop is `Abandoned` and not waited for. |
| Transcriber error | Recorded with the chunk's speaker, start and length. No retry. The session continues. A panic in the transcriber is caught and recorded the same way. |
| Sink append error | Not retried. The whole segment and the error go into `summary.sink_errors`, so the text is handed back to the caller. The session continues with the next line. `Ok(false)` is not counted as a line. |
| Sink finalize error | Not retried; `finalized: false` and `finalize_error`. Called exactly once in every case except a dead transcription thread. |
| Panics | Transcriber and sink calls run under `catch_unwind`. If the transcription thread still dies, `status.fatal` is set, chunks are dropped and listed (`WorkerDied`), and `stop` returns. If the capture thread dies, queued chunks are still transcribed, the sink is finalized, and the summary says `CaptureFailed`. Locks ignore poisoning. |
| Transcriber lifetime | The only reference is on the transcription thread, which is joined before the summary is published. When `stop()` returns, the caller's `Arc` is the last one. |
| `MNEME_KEEP_AUDIO` | Read once in `start`; only the value `1` counts. Without it nothing is written and `audio_dir` is not even created. With it, `RecordingWriter` writes every frame, flushed once a second. A write failure stops the writing and is reported; the session goes on. |

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-pipeline` | 56 passed, 0 failed, 0 ignored: 9 unit, 1 end-to-end, 17 `tests/failures.rs`, 6 `tests/keep_audio.rs`, 22 `tests/session.rs`, 1 doc-test. About 14 s. |
| The same, repeated | 7 runs in a row on the final code, all green. (5 as a loop, one before and one after.) |
| `cargo clippy -p mneme-pipeline --all-targets -- -D warnings` | Clean. |
| `cargo fmt -p mneme-pipeline -- --check` | Clean. |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p mneme-pipeline --no-deps` | Clean. |

Acceptance criteria:

1. Two speakers, timestamps, one finalize: `two_speakers_give_lines_with_chunk_start_plus_span_offset_and_one_finalize` (exact segments, spans at 100 and 350 ms into each chunk).
2. Backlog and stop: `a_slow_transcriber_builds_a_backlog_and_stop_returns_only_when_it_is_gone` (backlog seen while recording, all 8 chunks transcribed when `stop` returns, drain progress watched from another thread).
3. One source errors mid-session: `a_source_that_dies_mid_session_keeps_its_last_words_and_the_other_carries_on`; also `a_source_that_fails_to_start_is_reported_and_the_other_records`.
4. Transcriber error on one chunk: `a_transcriber_error_on_one_chunk_is_recorded_and_the_session_continues`.
5. WAV files: `tests/keep_audio.rs`. Each test runs the test binary again as a child process with the variable set or removed for that child, then checks the folder: no files by default (and the folder is not created), none for values other than `1`, both files with `1`, equal in length and matching the played samples.
6. End to end: `speech_on_both_channels_becomes_an_ordered_transcript_in_a_meeting_folder`, with `ggml-tiny.en.bin` and the real store. It wrote:
   ```text
   [00:00:00] Me: Hello, this is a test of the meeting transcription, the quick brown fox jumps over the lazy
   [00:00:03] Them: Hello, this is a test of the meeting transcription, the quick brown fox jumps over the lazy
   [00:00:05] Me: dog.
   [00:00:08] Them: dog.
   ```
   The engine is unwrapped from its `Arc` (which proves the session holds no reference) and dropped before the test ends. Without a model file it prints `SKIPPED` and passes; that path was not run, since the model is present here.
7. Test and clippy commands as in the table.

Also covered: stop twice, stop from three threads at once, stop right after start (30 rounds), stop while both sources are stuck in `start`, a source that starts late, a source that never starts, a source that ends in good health, both sources ending, no speech at all, a transcriber that returns no spans, a sink that fails once, a sink that panics, a finalize that fails, the backlog limit, abandoning before and during the drain, queue order, status while the transcriber and the sink are blocked, dropping the session without `stop`, stop racing a dying source (24 rounds), and both of the session's own threads dying.

**The tests fail when the behavior is broken.** Five changes were made in turn, the matching test run, and the change undone (sources compared with their backups afterwards). Each made a test fail: no flush when a source ends; no flush at stop; a refused line offered to the sink again; the transcription thread ending when the queue is closed with chunks still waiting; a transcriber error ending the transcription thread. No wider mutation run was done.

### Known gaps

- **No real sources.** Nothing here has run with `MicSource` or `SystemAudioSource`. The manual check is open.
- **A transcriber call that never returns hangs `stop()`** and `Drop`. `abandon_backlog` does not help, because the call in progress cannot be interrupted. `wait_timeout` lets a caller give up waiting, but the stuck thread then still holds the engine.
- **`Drop` blocks for the full drain.** A caller that wants a short exit must call `abandon_backlog()` first.
- **The backlog ceiling drops speech.** Reported, never silent, but it is a departure from the PRD. It has been tested with a 1 s limit, not at the 2 h default.
- **Dropped at the ceiling means the newest chunk.** No test or measurement says that is the better choice than dropping the oldest.
- **Failed chunks are not retried** and their audio is gone. A transient engine error loses that chunk.
- **A line refused by the sink is only in the summary.** If the caller ignores `sink_errors`, the text is lost.
- **`state` lags by up to 20 ms.** It moves to `Recording` when the capture thread next looks at the sources. A session of very short fakes can go from `Starting` straight to `Stopping`.
- **A source that stalls while reading as running** keeps its open chunk in the chunker until the next frame or stop. Nothing is lost, but those words arrive late.
- **Worker and capture thread death are tested through private hooks** (`Hooks`, crate-only fields that are always off outside the crate's own tests), not through a real fault.
- **Samples freed after transcription** is by construction (the chunk is dropped before the lines are written); no test measures memory.
- **Kept audio after a capture-thread panic** is reported with an error, and the files hold only what was flushed last. Not tested.
- **`StartError::Thread`** (a thread could not be created) is not tested.
- **Timing.** Tests with real-time fakes take up to 6.4 s each and rest on sleeps of the fake source; they were green in 7 runs on one idle machine. They have not been run under load.

### Follow-ups for 021

- **Start:** `Session::start(vec![mic, system], engine.clone() as Arc<dyn Transcriber>, store.transcript_writer(&id)?, SessionConfig { audio_dir: Some(store.meeting(&id)?.folder), ..SessionConfig::new(language) })`. Returns at once. Keep the session in app state as `Arc<Session>` or behind an `Option`.
- **Poll:** `session.status()` about once a second. Map `Starting`/`Recording` to `recording`, `Stopping` to `stopping`, `Done` to idle. `elapsed.as_secs()` is `elapsedSecs`. If `state == Done` without a stop request, every source ended: call `stop()` (it returns at once) and finish the meeting. If `fatal` is set, stop the session and tell the user. A source with phase `FailedToStart` and `PermissionDenied` first in its errors is the "mic only" case.
- **Notes:** `save_notes_at(&id, &notes, session.elapsed().as_millis() as u64)`.
- **Stop:** off the UI thread, `let summary = session.stop();` then `store.finish_meeting(&id, summary.duration.as_secs())`, then emit idle. Look at `summary.failed_sources()`, `transcription_errors`, `sink_errors`, `dropped` and `finalize_error`; if `finalized` is false, `store.finalize_transcript(&id)` sorts the file.
- **Quit while recording:** `session.abandon_backlog(); session.stop();` if the user should not wait, or plain `stop()` to keep everything. Then drop the session, then drop the engine, before the process exits.
- **Engine:** the session's reference is gone when `stop()` returns. The app may keep its own `Arc<WhisperEngine>` between meetings, but must drop it before exit.
- `Cargo.lock` needs committing.

