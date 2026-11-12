# 006: Silence chunker

Status: todo · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 005 · Owns: `crates/mneme-transcribe/`

## Goal

Cut a continuous stream of frames for one speaker into chunks at pauses, and never hand Whisper silence.

## Scope

1. **`Chunker`**: push `AudioFrame`s for one speaker; get back zero or more finished chunks (`speaker`, `start_ms`, samples). `flush()` returns whatever is pending.
2. **Speech detection**: energy-based, with an adaptive noise floor so a noisy room does not count as speech. Keep it dependency-light; note in the Result if a model-based detector looks necessary after testing.
3. **Cut rules**: end a chunk after a pause of the configured length; force a cut at the maximum chunk length, at the quietest point in the last stretch before the limit; keep a little audio either side of speech so words are not clipped.
4. **Drop silent chunks**: a chunk with less than the minimum amount of speech is discarded.
5. Gaps in frame timestamps (a source restarting) end the current chunk; `start_ms` always reflects real meeting time.
6. A `ChunkerConfig` with documented defaults.

## Acceptance criteria

- [ ] Tests with synthetic audio (tone bursts and noise as "speech", low noise as "silence"): one chunk per burst; chunk `start_ms` matches where the burst began; pure silence yields nothing; a long unbroken burst is force-cut under the maximum; `flush()` returns the tail; a timestamp gap splits chunks.
- [ ] Test with the speech WAV from issue 005 containing an inserted long pause: two chunks, no words clipped (checked with the tiny model when present, skipped otherwise).
- [ ] No unbounded memory growth while fed silence for a long simulated time.
- [ ] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
