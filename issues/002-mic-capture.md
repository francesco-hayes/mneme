# 002: Mic capture to 16 kHz mono, WAV writer

Status: done · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 001 · Owns: `crates/mneme-audio/`

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

- [x] Unit tests: downmix; resampling 44.1 kHz and 48 kHz to 16 kHz keeps a test tone's frequency and length within tolerance; timestamps increase with no gaps; WAV round-trip.
- [x] The fake source drives a sink end to end in a test.
- [x] A live mic test exists, marked ignored.
- [x] `cargo test -p mneme-audio` passes; `cargo clippy -p mneme-audio` is clean.

## Manual checks

- [ ] Run the ignored live test or a small example; speak; the WAV plays back clean at the right speed.
- [ ] Plug in or switch the input device mid-recording; capture continues.

## Result

Implemented on 2026-11-12. All four acceptance criteria pass. The two manual checks are open: nothing in this issue was run against the real microphone.

### What was built

All in `crates/mneme-audio/`. Public API (re-exported from the crate root unless a module path is shown):

| Item | Summary |
| --- | --- |
| `trait AudioSource: Send` | `speaker()`, `start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError>`, `stop(&mut self)`, `is_running(&self) -> bool`, `take_errors(&mut self) -> Vec<AudioError>`. Object-safe (`Box<dyn AudioSource>`). A source drops its sender when it stops, fails or runs out, so the receiver ends once every source is done. |
| `FrameSender`, `FrameReceiver`, `frame_channel()` | `std::sync::mpsc` channel of `mneme_core::AudioFrame`. Clone one sender into both sources for a single two-speaker stream. |
| `Framer` | The meeting clock. `new(speaker, start, sink, sample_rate, channels)`, `push_interleaved`, `push_planar`, `push_mono` (each takes the buffer and the `Instant` its first sample was captured), `set_input_format(rate, channels)` after a re-open, `finish()`, `position_ms()`. Emits frames of `FRAME_SAMPLES` (320 samples, `FRAME_MS` = 20 ms); only the last frame before a gap or the end is shorter. |
| `convert` module | `ToF32` (i8/i16/i32/i64, u8/u16/u32/u64, f32/f64), `i24_to_f32`, `int_to_f32`, `downmix_interleaved`, `downmix_planar`, `downmix_to_mono`, `StreamResampler` (`new(rate)`, `process`, `finish`, `expected_output_len`), `resample_to_16k`, `to_16k_mono`. No device needed. |
| `MicSource` | Default input device through cpal, `Speaker::Me`. `MicSource::new()` plus the trait. |
| `WavWriter` | `create(path)`, `write(&[f32])`, `write_silence(n)`, `samples_written()`, `duration_ms()`, `flush()`, `finalize()`. 16 kHz mono 16-bit PCM, streamed through a buffered file. Also `wav::f32_to_i16` and `read_wav_16k_mono(path)` (any WAV to 16 kHz mono `f32`). |
| `PermissionStatus`, `microphone_permission_status()`, `request_microphone_permission()` | AVFoundation authorization status and request. The request blocks until the user answers. |
| `FakeSource`, `Pacing` | `from_samples(speaker, Vec<f32>)`, `from_wav(speaker, path)`, `with_pacing(Pacing::Fast \| Pacing::RealTime)`, `starting_at(offset)`, `failing_after(played, error)`, `failing_to_start(error)`, `duration()`, `wait_until_finished()`. Timestamps are deterministic in both pacings. |
| `AudioError` | `Clone + PartialEq`, `#[non_exhaustive]`: `NoInputDevice`, `PermissionDenied`, `Device`, `UnsupportedFormat`, `Stream`, `Resample`, `AlreadyRunning`, `SinkClosed`, `Wav`, `Other`. |

Crate-internal, for issue 003 to reuse: `capture::{Backend, OpenStream, StreamEvents, Capture, Timing}`. `Capture` is the thread that owns a stream and a `Framer` and re-opens the stream when the default device changes, the stream reports an error, or no audio arrives for 3 s. A source only supplies a `Backend` (how to open its kind of stream and how to name the current default device). `MicSource` is a cpal `Backend` plus a permission check.

How the pieces behave:

- **Clock.** The wall clock is read once per stream, to place its first sample in meeting time (rounded up to a whole millisecond). After that `start_ms` comes from sample counts: each frame starts exactly 20 ms after the previous one. A re-open, or a buffer arriving more than 500 ms later than the sample count predicts, re-anchors the stream; the hole in `start_ms` then matches the hole in the audio. Timestamps never go backwards.
- **Resampler.** `rubato` 5.1.0. The fixed-ratio FFT resampler for every standard rate; the sinc resampler for rates that share almost no factor with 16 kHz (tested with 44 101 Hz). Start-up delay is trimmed and the tail flushed, so N input samples give exactly `ceil(N * 16000 / rate)` output samples.
- **Mic.** Opens the device in its own current format and converts here; requesting 16 kHz from cpal would reconfigure the hardware for every app. The audio callback only downmixes and hands the buffer to the capture thread; resampling runs there. cpal 0.18.2 on macOS follows default *output* changes only, so the default input device is polled twice a second.

### Dependencies added (`crates/mneme-audio/Cargo.toml`)

`cpal 0.18.2`, `rubato 5.1.0`, `hound 3.5.1`, `thiserror 2.0.21`; on macOS only: `objc2 0.6.5`, `block2 0.6.2`, `objc2-av-foundation 0.3.2` (features `std`, `AVCaptureDevice`, `AVMediaFormat`, `block2`). Versions and signatures were checked against the crate sources cargo downloaded, not from memory. Cargo updated the root `Cargo.lock` as a result.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-audio` | 72 unit tests passed, 2 doc-tests passed, 2 live tests ignored, 0 failed |
| `cargo test -p mneme-audio --lib`, 20 runs in a row | 20 of 20 passed (the capture and real-time tests use threads and timers) |
| `cargo clippy -p mneme-audio --all-targets -- -D warnings` | clean |
| `cargo clippy -p mneme-audio` | clean |
| `cargo fmt -p mneme-audio -- --check` | clean |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p mneme-audio --no-deps` | clean |

Where each criterion is covered:

- **Downmix:** `convert::tests`: `stereo_downmix_averages_each_frame`, `downmix_handles_many_channels_and_integer_formats`, `planar_downmix_matches_interleaved`, and three edge-case tests.
- **Resampling 44.1 kHz and 48 kHz:** `resampling_44100_keeps_tone_frequency_and_length`, `resampling_48000_keeps_tone_frequency_and_length`. For each tone: output length equals `ceil(N * 16000 / rate)` exactly; frequency within 1 Hz (zero crossings); amplitude within 1 %; residual after subtracting the fitted sine below 0.5 % of full scale; time offset under one output sample. Also `resampling_other_rates_...` (8, 22.05, 24, 32, 88.2, 96, 192 kHz), `odd_rates_use_the_sinc_fallback`, `frequencies_above_the_new_nyquist_are_removed` (a 12 kHz tone at 48 kHz comes out below 0.001 RMS, which a nearest-sample resampler would fail), `streaming_in_uneven_pieces_matches_one_shot`.
- **Timestamps increase with no gaps:** `framer::tests::timestamps_count_samples_and_ignore_callback_jitter` (3 s of 48 kHz stereo in 512-frame callbacks with capture times wobbling by up to 15 ms: 150 frames, each starting exactly 20 ms after the last). Also re-open, stall, never-backwards and anchoring tests in the same module.
- **WAV round-trip:** `wav::tests::wav_round_trip_preserves_format_length_and_samples` (header fields, file size, sample count, every sample within half a 16-bit step), plus `rewriting_16_bit_audio_is_lossless`, `flush_leaves_a_valid_file_mid_recording`, `silence_pads_by_sample_count`, `reading_converts_other_rates_channels_and_formats`.
- **Fake source drives a sink end to end:** `fake::tests::fake_source_drives_a_sink_end_to_end` (samples arrive bit-identical, contiguous) and `fake_source_feeds_a_wav_writer_through_the_trait` (48 kHz stereo WAV, through `Box<dyn AudioSource>` and the channel, into `WavWriter`, read back: 16 000 samples, tone at 1 kHz). Eleven more tests cover offset, real-time pacing, stop, failure, restart and two sources on one channel.
- **Live mic test exists, ignored:** `tests/live_mic.rs`: `live_mic_records_to_wav` and `live_mic_follows_device_changes`. Both compile (clippy `--all-targets`) and show as ignored in the test run.

The device-change logic of scope item 4 is covered without hardware by `capture::tests`, which drive the capture thread with a scripted backend: `a_new_default_device_is_followed_and_the_clock_kept` (48 kHz device replaced by a 44.1 kHz one; frames continue at the right meeting time, late callbacks from the closed stream are dropped, no error reported), `a_stream_error_is_reported_and_the_stream_reopened`, `capture_survives_a_spell_with_no_device` (one error reported, retries until a device returns), `a_silent_stream_is_reopened`, `capture_ends_when_the_receiver_is_dropped`.

Two tests touch the real system without capturing anything: `mic::tests::the_default_input_device_can_be_described` reads the default input device's ID and format through cpal (on this machine: `coreaudio:BuiltInMicrophoneDevice`, 48 000 Hz, 1 channel, f32), and `permission::tests::reading_the_status_does_not_panic` reads the authorization status (`NotDetermined` for the process that ran the tests). Neither builds a stream or shows a prompt.

### Not verified

- **`MicSource` has never captured audio.** Opening the cpal stream, the callback path, audio quality, playback speed and re-opening on real hardware are untested. The cpal backend is about 100 lines on top of the tested capture loop, but that is reasoning, not evidence. Both manual checks below cover it.
- `request_microphone_permission()` has never been called. It compiles against the objc2 bindings; the prompt and its result are unobserved.
- What cpal delivers while the permission prompt is on screen (silence or no callbacks) was not observed. If it is no callbacks, the source re-opens every 3 s and queues one `Stream` error until audio flows.
- Long-run drift between a device clock and the system clock was not measured.
- Only built on macOS. The permission functions have a non-macOS branch that was not compiled.

### Running the manual checks

From a terminal app that has, or can be given, microphone permission (macOS attributes the request to the terminal):

```sh
cargo test -p mneme-audio --test live_mic -- --ignored --nocapture live_mic_records_to_wav
cargo test -p mneme-audio --test live_mic -- --ignored --nocapture live_mic_follows_device_changes
```

The first records 5 s, the second 30 s (`MNEME_LIVE_SECONDS=N` changes either). Each prints the WAV path (`target/tmp/live-mic.wav`, `target/tmp/live-mic-device-change.wav`), the frame count, the peak level, any gaps and any source errors. The first also asserts no gaps, no errors, a non-silent recording, and that the sample count matches elapsed time. The second asserts that capture reaches the end and that under 5 s of audio is missing; gaps are written as silence so the file plays back in meeting time.

### Deviations and choices not spelled out in the issue

- **Frames are a fixed 20 ms.** The issue does not set a size. Fixed frames make `start_ms` advance by an exact number of milliseconds.
- **Re-anchor on a stall.** The issue says timestamps come from sample counts once a stream runs. That holds, with one addition: a buffer more than 500 ms late re-anchors the stream, so a stalled device or a sleeping machine does not leave every later timestamp behind real time.
- **Resampled audio is aligned to within one output sample (62.5 µs), not exactly.** rubato reports its delay in whole samples.
- **Downmix averages all channels.** A multi-channel interface with one microphone input comes out quieter by the channel count.
- **Buffer overruns reported by cpal are not queued as errors**, to keep `take_errors` free of noise. A long one appears as a gap in `start_ms`.
- **`MicSource::start` fails with `PermissionDenied` when the status is `Denied` or `Restricted`.** Otherwise macOS would hand over silence with no error. With `NotDetermined` it starts, and the OS prompts.
- **`AudioError` carries messages as text**, so it is `Clone` and comparable for summaries and tests.
- **No example binary.** The two ignored tests serve as the "small example"; `mneme-record` in issue 004 is the real one.

### Follow-ups

- `issues/README.md` still lists 002 as `in progress`; it is outside this issue's paths.
- The root `Cargo.lock` changed when the dependencies were added and needs committing with this work.
- **003:** implement `capture::Backend` for the tap and reuse `Capture`; it already handles re-open, the clock and error reporting. If the tap delivers non-interleaved buffers, add a planar variant of `StreamEvents::data` (the `Framer` already has `push_planar`). Compute `first_sample_at` from the IO proc's host time. Put the system-audio permission functions next to the microphone ones in `permission.rs`.
- **004:** the recorder can clone one `FrameSender` into both sources, poll `is_running()` and `take_errors()`, and pad with `WavWriter::write_silence` using the gaps in `start_ms`. `FakeSource::starting_at`, `failing_after` and `failing_to_start` exist for its tests.
- **009:** the frame channel is unbounded `std::sync::mpsc`. A consumer on an async runtime needs a bridging thread or a blocking task.
- **021 / 024:** the app bundle's `Info.plist` needs `NSMicrophoneUsageDescription`, and `request_microphone_permission()` must be called off the UI thread.
