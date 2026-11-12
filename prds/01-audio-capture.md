# PRD 01: Audio capture

Covers build step 1 and the gaps "System audio + mic capture" and "Me vs. them labels". Issues: 002–004.

## Problem

The mic alone only hears me. The other side of a Zoom or Meet call arrives as system audio, which needs OS-specific capture. This is the riskiest part of v1, so it is built first and without any UI.

## Goals

- Capture the microphone and system audio at the same time, as two separate channels on one clock.
- Deliver both as 16 kHz mono `f32` frames, ready for Whisper.
- Prove it with a command-line recorder that writes two WAV files.

## Non-goals

- Echo cancellation. v1 recommends headphones (see Risks).
- Choosing input devices in the UI; v1 uses the system default input.
- Windows capture (WASAPI loopback) and ScreenCaptureKit. The trait leaves room for both.

## Requirements

### Sources

| Source | Speaker | Implementation |
| --- | --- | --- |
| Microphone | `Me` | `cpal`, default input device |
| System audio | `Them` | Core Audio process tap (macOS 14.4+), all processes, excluding Mneme itself |

Both implement one trait: start with a shared start instant and a frame sink, stop, and report errors. Code that consumes audio never knows which API is behind a source.

### Frames

- Each source converts whatever the device gives (any sample rate, any channel count) to 16 kHz mono `f32`.
- Frames carry `start_ms` in meeting time, measured from the shared start instant, so the two channels line up.
- A device change mid-recording (headphones plugged in, AirPods connecting) must not end the recording. The source re-opens on the new default device and keeps the clock.

### Recorder CLI

`mneme-record --out <dir> [--seconds N]` records both channels until Ctrl-C or the time limit, then writes `mic.wav` and `system.wav` (16 kHz, mono, 16-bit). This is a development tool and the only place raw audio is written to disk outside the dev keep-audio flag in PRD 03.

### Permissions

- Mic capture needs `NSMicrophoneUsageDescription`.
- Process taps need `NSAudioCaptureUsageDescription`.
- The crate exposes a way to check each permission's status and to request it. The user-facing flow is in PRD 06.

## Risks

| Risk | Handling |
| --- | --- |
| Process taps are a new API with thin documentation | Built behind the trait. If it dead-ends, ScreenCaptureKit is the fallback. |
| Speaker bleed without headphones | Not solved in v1. First run recommends headphones. |
| Permissions cannot be granted by an automated test | Live capture tests are marked ignored and listed as manual checks. |

## Done when

- Unit tests cover resampling, channel downmix, WAV writing and the shared clock, using synthetic input.
- Manual: `mneme-record` during a real Zoom call gives two clean files, mine in `mic.wav` and the other side in `system.wav`.
- Manual: switching the output device mid-recording does not stop either file.
