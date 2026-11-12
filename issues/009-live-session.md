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
