# 002: Mic capture to 16 kHz mono, WAV writer

Status: todo · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 001 · Owns: `crates/mneme-audio/`

## Goal

Capture the default microphone and deliver 16 kHz mono `f32` frames on a meeting clock, plus the shared pieces every source needs.

## Scope

1. **`AudioSource` trait**: start with a shared start `Instant` and a frame sink (a channel sender of `AudioFrame` is fine), stop, and a way to surface errors that happen after start. Both the mic and the system source (issue 003) implement it.
2. **Conversion module**: downmix any channel count to mono; resample any input rate to 16 kHz with a real resampler (not nearest-sample); handle the common sample formats devices return. Pure functions, no device needed.
3. **Meeting clock**: frames carry `start_ms` since the shared start instant. Timestamps are derived from sample counts once a stream is running, so they do not drift or jitter with callback timing.
4. **`MicSource`** using `cpal` on the default input device, labeled `Speaker::Me`. If the default input device changes or the stream errors, re-open on the new default and keep the clock.
5. **WAV writer**: write 16 kHz mono 16-bit PCM from `f32` samples, streaming (does not hold the whole recording in memory).
6. **Permission helpers** for the microphone: status and request.
7. **Fake source** for tests and for other crates: plays a `Vec<f32>` or a WAV file as frames in real time or as fast as possible.

## Acceptance criteria

- [ ] Unit tests: downmix; resampling 44.1 kHz and 48 kHz to 16 kHz keeps a test tone's frequency and length within tolerance; timestamps increase with no gaps; WAV round-trip.
- [ ] The fake source drives a sink end to end in a test.
- [ ] A live mic test exists, marked ignored.
- [ ] `cargo test -p mneme-audio` passes; `cargo clippy -p mneme-audio` is clean.

## Manual checks

- [ ] Run the ignored live test or a small example; speak; the WAV plays back clean at the right speed.
- [ ] Plug in or switch the input device mid-recording; capture continues.

## Result

_Filled in by the implementer._
