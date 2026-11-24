# PRD 03: Live pipeline

Covers build step 3, MVP feature 2 (background transcription) and the gap "Audio retention rule". Issues: 009.

## Problem

Capture, chunking, transcription and storage exist as separate parts. A meeting needs them joined: one click starts everything, transcription keeps up in the background, and at stop the full transcript is ready right away.

## Goals

- One `Session` object that owns a recording from start to stop.
- Two channels in, labeled lines out: mic is `Me`, system audio is `Them`.
- Raw audio is dropped as soon as each chunk is transcribed.
- At stop, everything still queued is transcribed before the session reports done.

## Non-goals

- Showing the transcript live. It stays hidden during the meeting.
- Diarization beyond me vs. them.
- Echo cancellation.

## Requirements

### Session lifecycle

| Call | Effect |
| --- | --- |
| `start(model, language, meeting)` | Loads the model, starts both audio sources on a shared clock, starts chunkers and the transcription worker. Fails cleanly if the model is missing or a permission is denied. |
| running | Each transcribed segment is appended to the meeting's transcript at once (PRD 04). Elapsed time and a count of lines are observable. |
| `stop()` | Stops capture, flushes both chunkers, drains the queue, sorts the transcript by timestamp, writes duration and status. Returns when the transcript is complete. |

### Flow

```
mic source ──► chunker(Me) ──┐
                              ├─► queue ─► whisper worker ─► transcript append
system source ► chunker(Them)┘
```

- One Whisper worker serves both channels from one queue, oldest chunk first.
- Each line's timestamp is the chunk's `start_ms` plus the segment's offset inside the chunk.
- If the queue grows (model slower than real time), nothing is dropped below two hours of queued audio. The session reports how far behind it is, and stop takes as long as the backlog needs. Past two hours a new chunk is dropped and reported, to protect memory (decided 2026-11-24, see issue 009).
- An error in one channel (for example system audio permission revoked) does not stop the other. The session reports it.

### Audio retention

- Chunk samples are freed after transcription. Nothing is written to disk.
- Dev-only exception: with `MNEME_KEEP_AUDIO=1`, the session also writes `mic.wav` and `system.wav` into the meeting folder, so a bad transcript can be redone and models compared. Never on by default, never exposed in the UI.

### Testability

Sources, the transcriber and the transcript sink are traits. Tests run the whole session with fake sources that play WAV files and a fake transcriber, with no microphone, model or permissions.

## Speaker bleed

Without headphones, lines repeat and labels flip. Decided 2026-11-24: v1 recommends headphones and does no echo cancellation.

## Done when

- Tests with fake sources and a fake transcriber cover: start/stop, both labels present, timestamps in order after stop, backlog drained at stop, one channel failing, no audio files left behind.
- Manual: a 30 minute call gives a full transcript at stop.
