# 003: System audio capture with Core Audio taps

Status: done · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 002 · Owns: `crates/mneme-audio/`

## Goal

Capture what the Mac is playing (the other side of a call) as `Speaker::Them` frames, using Core Audio process taps.

## Scope

1. **`SystemAudioSource`** implementing the `AudioSource` trait from issue 002. A global tap over all processes, excluding Mneme's own process, wrapped in an aggregate device and read with an IO proc. Tap is unmuted: the user still hears the call.
2. Read the tap's stream format and run it through the conversion module from issue 002 (downmix, resample to 16 kHz).
3. Clean teardown: destroying the tap and aggregate device on stop and on drop, including after errors. No orphan aggregate devices left in the system.
4. Follow output device changes (headphones in or out, AirPods) without ending capture and without a jump in the meeting clock.
5. **Permission helpers** for system audio capture: status and request, as far as the OS allows checking. Document what the OS actually exposes.
6. The API choice is behind the trait. Note in the module docs what a ScreenCaptureKit source would need to implement.

## Notes

- Minimum macOS is 14.4. Check current Apple documentation and the current state of Rust bindings before choosing how to call the API (existing crate versus direct bindings); record the choice and why in the Result.
- The binary that captures needs `NSAudioCaptureUsageDescription`. For a command-line binary, document how that is supplied for manual testing.
- This is the highest-risk issue in v1. If taps cannot be made to work, stop and write up exactly what failed, rather than switching API without saying so.

## Acceptance criteria

- [x] `SystemAudioSource` compiles and implements the trait; teardown is covered by a test that needs no permission where possible.
- [x] Unit tests for format handling using synthetic buffers in the formats taps deliver.
- [x] A live capture test exists, marked ignored.
- [x] `cargo test -p mneme-audio` and clippy pass.

## Manual checks

- [ ] Play audio from another app; the ignored live test or example records it cleanly.
- [ ] Mneme's own sounds are not captured.
- [ ] Switch output device mid-capture; capture continues.
- [x] After the process exits, Audio MIDI Setup shows no leftover aggregate device. (Checked 2026-11-12 by the live run below and `system_profiler SPAudioDataType` afterwards, not in Audio MIDI Setup itself.)

## Result

### Live run, 2026-11-12 (orchestrator, after the implementation)

First contact with a real tap. Built from commit `0a3639f` in a clean copy and run through the bundle wrapper:

```
MNEME_LIVE_TEST_TARGET=lib crates/mneme-audio/scripts/live-system-test.sh live_tap_session
```

Outcome: `live_tap_session_leaves_nothing_behind` passed in 2.59 s.

- The tap was created and reported `TapFormat { sample_rate: 48000, channels: 2, kind: F32, interleaved: true }`.
- The aggregate device was created and runs at 48,000 Hz (default output: built-in speakers, 48 kHz).
- The session opened with the access check on, and it was not refused.
- While open, the tap and the aggregate device were both listed. After teardown: devices 4 → 4, taps 0 → 0, no teardown errors, callback context not leaked.
- After the process exited, `system_profiler SPAudioDataType` listed only the Mac's own devices and no aggregate device.

So these are now executed, once, on macOS 27.0.1: `AudioHardwareCreateProcessTap`, reading a real tap's format, `AudioHardwareCreateAggregateDevice`, creating the IO proc, `AudioDeviceStart`, the access check, and the four teardown calls.

Still never executed: recording real audio to a WAV and listening to it, capture through silence, excluding this process's own sound, following an output device change, the permission-request helpers, and any other macOS version.


Implemented on 2026-11-12. All four acceptance criteria pass. **No real tap has been created and no system audio has been captured**: the hard limits of this run forbade it, so every Core Audio call that creates, starts, checks or destroys something has only been compiled. All four manual checks are open, and they are the first time this code meets a real tap. Read "What has never been executed" before relying on any of it.

### Binding choice

**`objc2-core-audio` 0.3.2, called directly**, with `objc2-core-audio-types` 0.3.2, `objc2-core-foundation` 0.3.2 and `objc2-foundation` 0.3.2 (plus `objc2` 0.6.5, already used). All are the newest versions on crates.io on 2026-11-12, and all were already in `Cargo.lock` because cpal depends on them, so nothing new is downloaded or compiled.

Why: its generated bindings expose every symbol the tap path needs. Checked in the crate source, not from memory: `CATapDescription` with all its initializers and properties, `CATapMuteBehavior`, `AudioHardwareCreateProcessTap`, `AudioHardwareDestroyProcessTap`, `AudioHardwareCreateAggregateDevice`, `AudioHardwareDestroyAggregateDevice`, `AudioDeviceCreateIOProcID` (and the block variant), `AudioDeviceStart`/`Stop`/`DestroyIOProcID`, `kAudioTapPropertyFormat`/`UID`/`Description`, the aggregate and sub-tap dictionary keys, `kAudioHardwarePropertyTranslatePIDToProcessObject`, and the host time functions. The names and semantics were checked against the macOS SDK headers on this machine (`AudioHardwareTapping.h`, `CATapDescription.h`, `AudioHardware.h`) and Apple's article "Capturing system audio with Core Audio taps".

What else was looked at:

| Option | Finding |
| --- | --- |
| cpal 0.18.2's own loopback (`src/host/coreaudio/macos/loopback.rs`) | It exists: building an input stream on an output-only device creates a tap and a private aggregate device. Not usable here. The process list is hard-coded empty, so this process cannot be excluded. The tap is bound to one device's stream 0. It only happens when the device has no input side, so a headset or AirPods (input and output on one device) would be opened as a microphone instead. It sets the physical format or sample rate of the real output device first. Teardown statuses are discarded. |
| `coreaudio-sys` 0.2.18 | A C bindgen crate. The tap functions are declared only for Objective-C (`#ifdef __OBJC__` in the SDK header) and `CATapDescription` is an Objective-C class, so it should not have them. Not confirmed against the crate's output (docs.rs has no build of it). |
| `cidre` 0.29.0 | Has tap support (seen used by a third-party crate). A second, separate binding stack next to the objc2 one this crate, cpal and Tauri already use. Not read in depth. |
| `coreaudio` 0.6.2 | A safe HAL wrapper over `objc2-core-audio` with a `process-tap` feature. Young, one maintainer, and it hides the teardown ordering this issue has to prove. Not read in depth. |

Three implementations were read as references for how the calls fit together: Apple's sample article, AudioCap (`insidegui/AudioCap`), and Chromium's `media/audio/mac/catap_audio_input_stream.mm`. Where they disagree, Chromium was followed, as the one that ships at scale.

### What was built

All in `crates/mneme-audio/`.

Public API added (re-exported from the crate root):

| Item | Summary |
| --- | --- |
| `SystemAudioSource` | `new()`, `excluding_process(pid: u32) -> Self`, `without_access_check() -> Self`, plus the `AudioSource` trait (`speaker()` is `Speaker::Them`). `Send`, boxable as `dyn AudioSource`. |
| `system_audio_permission_status() -> Option<PermissionStatus>` | `None` means unknown. See "Permission helpers". |
| `request_system_audio_permission() -> Option<PermissionStatus>` | Blocks while the prompt is up. See "Permission helpers". |
| `AudioError::NoOutputDevice` | New variant (the enum is `#[non_exhaustive]`, so this breaks nothing). |
| Cargo feature `tcc-spi` | Off by default. Turns on the two private TCC calls. |

Nothing existing changed its signature. The 72 tests from issue 002 still pass.

Internals, in `src/system/`:

| File | What it holds |
| --- | --- |
| `mod.rs` | `SystemAudioSource`; `TapBackend`, the `capture::Backend` implementation; module docs with the design and the ScreenCaptureKit note. |
| `session.rs` | `TapSession`, the resource guard; the `Hal` trait (the 14 Core Audio operations a session needs); the fake HAL used by tests. |
| `hal.rs` | `CoreAudioHal`: the real calls. The only file with unsafe FFI to Core Audio. |
| `ioproc.rs` | The IO proc (real-time callback) and `IoContext`. |
| `format.rs` | `TapFormat::from_description`: reads the tap's `AudioStreamBasicDescription`. |

Also: `StreamEvents::data_planar` and `StreamEvents::detached` in `capture.rs`; the permission functions in `permission.rs`; `tests/live_system.rs`; `scripts/live-system-test.sh`.

How it works:

- **Tap.** `CATapDescription initStereoGlobalTapButExcludeProcesses:` with this process's audio object (looked up with `kAudioHardwarePropertyTranslatePIDToProcessObject`) and any PIDs given to `excluding_process`. Private, `CATapUnmuted`. Global: not bound to a device, so by the API's description audio an app sends to a device other than the default is captured too (not observed).
- **Aggregate device.** Private, no sub-devices, one tap with drift compensation, `tapautostart` off. This is Chromium's composition. Auto start is off on purpose: with it on, `AudioDeviceStart` waits for the first tapped audio, a quiet system would deliver no callbacks, and the capture thread would take that for a dead stream.
- **Format.** `kAudioTapPropertyFormat` is parsed, not assumed. Accepted: linear PCM, native-endian, float 32/64 or signed integer 16/24-packed/32, interleaved or not, any channel count, any rate up to 768 kHz. Anything else fails `start` with `UnsupportedFormat`. The rate handed to the `Framer` is the aggregate device's nominal rate (that is what IO runs at); if it differs from the tap's, the aggregate is asked to match the tap first.
- **IO proc.** `AudioDeviceCreateIOProcID` with a plain function and a context pointer (not the block variant: no `dispatch2` needed, and the context's lifetime is explicit). Each callback reads the `AudioBufferList` as it actually is (one interleaved buffer, or one buffer per channel), downmixes through the existing `downmix_interleaved` / `downmix_planar`, and sends the mono buffer to the capture thread. Sample types are byte-array newtypes with alignment 1, so no assumption is made about buffer addresses. Panics are caught before they can unwind into Core Audio.
- **Capture time.** `first_sample_at` comes from the IO proc's input timestamp (`mHostTime`, "the time at which the first frame in the data was acquired"), converted with `AudioGetCurrentHostTime` / `AudioConvertHostTimeToNanos`. If the timestamp is missing, in the future or more than a second old, the buffer's own duration is used instead.
- **Reuse.** `SystemAudioSource` is a `Backend` for the existing `Capture` thread, as issue 002 intended. Re-open, the meeting clock, stall detection and error queueing are that thread's, unchanged.

### How teardown is guaranteed

`TapSession` owns the four things a capture creates and is the `Backend::Stream` the capture thread holds. Its `Drop` releases them newest first: `AudioDeviceStop`, `AudioDeviceDestroyIOProcID`, `AudioHardwareDestroyAggregateDevice`, `AudioHardwareDestroyProcessTap`.

- **Opening is incremental.** The session object exists before the first call and records each resource as it is created. An error at any step returns early, which drops the session, which destroys exactly what exists.
- **Every path is a drop.** `stop()`, dropping the source, a re-open after a device change, a failed first start, the receiver going away, and a panic unwinding the capture thread all end in the same `Drop`.
- **A failing step does not stop the rest.** Each failure is collected and reported through the capture thread into `take_errors()` as `AudioError::Device("system audio capture was not shut down cleanly: ...")`.
- **The callback context is freed last**, and only if stop and destroy-IO-proc both succeeded. Otherwise it is leaked on purpose (a few dozen bytes), because a callback might still arrive. It is also switched off first, so a late callback does nothing. Chromium does the same.
- **Not covered:** a process that dies without unwinding (crash, `kill -9`, `panic = "abort"`). Both objects are created private to the process, and the SDK header says a private aggregate device does not outlive the process that created it. That the same holds for a private tap is an assumption.

The ordering is proven against a fake `Hal` that records every call and asserts ordering rules of its own (no tap destroyed before its aggregate device, no aggregate device before its IO proc, no IO proc while running).

### How device changes are followed

The tap is rebuilt, not kept. Twice a second the capture thread compares the default output device's UID and nominal sample rate with those the tap was built under. On a difference it drops the session (full teardown) and opens a new one; the `Framer` re-anchors, so frames after the switch carry the time they were really captured and the gap in `start_ms` equals the gap in the audio. No error is queued for this. A stream that delivers nothing for 3 s, or reports an error, is rebuilt the same way; with no output device at all, `NoOutputDevice` is queued once and it retries every second.

Why rebuild: what a running global tap does across a device or rate change is undocumented and could not be observed here. Chromium rebuilds on every default-device change, and an Apple developer forum report (thread 825780, unanswered) says a tap that went silent only recovered when both objects were recreated.

### Permission helpers: what they can and cannot determine

macOS has **no public API** to read or request the System Audio Recording permission (TCC service `kTCCServiceAudioCapture`). Apple's article only says the system prompts "the first time you start recording from an aggregate device that contains a tap". Without the permission no call fails; the tap delivers silence.

| Function | Default build | With `--features tcc-spi` |
| --- | --- | --- |
| `system_audio_permission_status()` | Always `None` (unknown). Touches nothing. | `TCCAccessPreflight`, a private function, resolved with `dlopen`/`dlsym` at run time: `Granted`, `Denied` or `NotDetermined`. Never `Restricted`. `None` if the symbol is missing. |
| `request_system_audio_permission()` | Opens a real tap session for a moment and closes it. `Some(Granted)` if the access check passes, `Some(Denied)` if it is refused, `None` if the tap could not be opened for another reason. | If the status is `NotDetermined`: `TCCAccessRequest` (private), returns the user's answer. Otherwise returns the status without prompting. |

Independent of both, **`SystemAudioSource::start` checks access itself, with public API**: after starting IO it reads the tap's description and writes it back unchanged. Chromium (`ProbeAudioTapPermissions`) and the `catap` Python package both report that macOS refuses that write without the permission. If it is refused, `start` tears everything down and returns `PermissionDenied`. `start` never consults the private API.

Limits:

- The access check has never run. If it turns out to refuse when the permission is in fact granted, system audio is unusable until it is fixed; `without_access_check()` (and `MNEME_SKIP_TAP_ACCESS_CHECK=1` in the live tests) exists to tell the two cases apart.
- Sources disagree on when the prompt appears: Apple says at start of recording, Chromium's source says at `AudioDeviceCreateIOProcID` and that the call blocks up to 60 s. The check runs after `AudioDeviceStart` so the prompt has had its chance either way. If the prompt turns out not to block, the first `start` (and the default-build `request_...`) returns "denied" while the prompt is still on screen, and works when called again after the user allowed it.
- A permission revoked while capturing, or any later silent failure of the tap, cannot be told from a quiet system. Nothing detects it.
- The private functions can change without notice and are not allowed in the Mac App Store. That is why the feature is opt-in. Whether Mneme uses it is a decision for issue 024.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-audio` | 131 unit tests passed (72 from issue 002, 59 new), 2 doc-tests passed, 7 ignored (1 + 4 live system, 2 live mic), 0 failed |
| `cargo test -p mneme-audio --features tcc-spi` | same counts; the preflight test printed `Some(NotDetermined)` |
| `cargo test -p mneme-audio --lib`, 20 runs in a row | 20 of 20 passed |
| The lib test binary, 30 runs with 12 busy-loop processes competing for the CPU | 30 of 30 passed |
| `cargo clippy -p mneme-audio --all-targets -- -D warnings` | clean |
| `cargo clippy -p mneme-audio --all-targets --features tcc-spi -- -D warnings` | clean |
| `cargo clippy -p mneme-audio` | clean |
| `cargo fmt -p mneme-audio -- --check` | clean |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p mneme-audio --no-deps` | clean |
| `scripts/live-system-test.sh --list` (both targets) | built, bundled, signed, launched through LaunchServices, listed the ignored tests, exit 0. Captures nothing. |

Where each criterion is covered:

- **Compiles and implements the trait; teardown tested without permission.** `system::tests::system_audio_source_is_a_boxable_audio_source`. Teardown: `system::session::tests`, 14 tests against the fake HAL. `dropping_a_session_tears_down_in_reverse_order`; `a_failure_at_any_step_releases_exactly_what_was_created` (failure injected at each of the six opening steps: only what existed is destroyed, newest first, nothing left); `teardown_carries_on_past_failures_and_reports_them` (each of the four teardown calls failing in turn: the other three still run, one error is reported); `the_io_context_outlives_a_proc_that_could_not_be_stopped`; `a_callback_after_a_failed_stop_is_ignored`; `teardown_happens_once`. Through the real capture thread, `system::tests::with_a_fake_hal` (11 tests): stop, drop without stop, failed start, missing permission, receiver dropped, device change, rate change, failed rebuilds and a stalled stream each end with zero taps, aggregate devices and IO procs alive.
- **Format handling with synthetic buffers.** `system::format::tests` (8): the stereo float layout taps deliver; rates 8 to 192 kHz; 1, 2 and 6 channels; non-interleaved; f64, i16, i24, i32; refusal of non-PCM, big-endian, zero channels, bad frame sizes, 24-in-32, unsigned, and absurd rates. `system::ioproc::tests` (12) call the real IO proc with hand-built `AudioBufferList`s: interleaved and planar, float and integer, mono and 5.1, buffers at odd addresses, null lists, disabled streams, ragged sizes. `tap_buffers_become_16k_mono_frames` runs one second of a 1 kHz tone through IO proc, downmix, resampler and framer in nine layouts (48, 44.1, 96, 16, 24 and 88.2 kHz; f32, f64, i16, i24, i32; 1, 2 and 6 channels; both interleavings) and checks 16 000 samples out, frames exactly 20 ms apart, frequency within 1 Hz, amplitude within 1 %, residual under 0.5 %.
- **Live capture test, ignored.** `tests/live_system.rs` (4 tests) and `system::hal::tests::live_tap_session_leaves_nothing_behind`. All compile under `clippy --all-targets` and show as ignored.
- **Tests and clippy pass.** The table above.

Run against the real system, without creating or capturing anything (`system::hal::tests`): the default output device was read (`BuiltInSpeakerDevice`, 48 000 Hz); this process's audio object was found by PID (`Some(126)`); the device and tap lists were read (4 devices, 0 taps); a read on a non-existent object returned `'!obj'` instead of crashing; the `CATapDescription` was built and read back (exclusive, mixdown, stereo, private, unmuted, the two excluded objects, no device UID, no stream); the aggregate dictionary was built and read back (five keys, one tap, private, auto start off). With `tcc-spi`, `TCCAccessPreflight` was found and returned a value.

### What has never been executed

Stated plainly, because "131 tests pass" says nothing about these:

1. `AudioHardwareCreateProcessTap`. No tap has been created by this code.
2. Reading `kAudioTapPropertyFormat` from a real tap. The format taps deliver on this machine is unknown to this code; it is expected to be 32-bit float interleaved stereo from what others report.
3. `AudioHardwareCreateAggregateDevice` with the dictionary this code builds. Whether Core Audio accepts it is unknown.
4. Reading or setting the aggregate device's nominal sample rate.
5. `AudioDeviceCreateIOProcID` with the Rust IO proc, and `AudioDeviceStart`.
6. The IO proc being called by Core Audio. Its real buffer layout, timestamps, callback size, and whether callbacks arrive while nothing plays are all unobserved.
7. The access check (get and set `kAudioTapPropertyDescription`).
8. `AudioDeviceStop`, `AudioDeviceDestroyIOProcID`, `AudioHardwareDestroyAggregateDevice`, `AudioHardwareDestroyProcessTap`. The teardown order is proven; that the real calls succeed is not.
9. `SystemAudioSource::start` against Core Audio. It has only run against the fake.
10. `request_system_audio_permission()`, on either route, and `TCCAccessRequest`. The prompt has never been seen: when it appears, whether it blocks, and what `start` returns meanwhile are taken from other people's code and documentation.
11. The five live tests, including the tone the exclusion test plays through cpal.
12. `scripts/live-system-test.sh` with a test that captures. Only `--list` ran. That the permission is attributed to the bundle, that `--env` values arrive, and that the bundle can write its WAV files are unverified.
13. Device changes on real hardware, and the rebuild time they cost.
14. Any macOS other than 27.0.1. The minimum is 14.4; Chromium notes that macOS 14 does not convert between tap and aggregate sample rates, which the rate handling tries to respect, untested.
15. The non-macOS branches (`SystemAudioSource::start` returns an error there). Not compiled.
16. Long-run behavior: drift, and the forum-reported case of a tap going silent after minutes.

### Running the manual checks

**How macOS attributes the permission.** TCC grants it to the *responsible* process's app, and reads `NSAudioCaptureUsageDescription` from that app's `Info.plist`. A binary started from a terminal is the terminal's responsibility; a bundle started through LaunchServices (`open`, Finder) is responsible for itself. (Source: Qt's "The Curious Case of the Responsible Process"; the `catap` and AudioTee projects say the same of taps.) Terminal.app on this machine has no `NSAudioCaptureUsageDescription` in its `Info.plist`. AudioTee's README says Apple's Terminal does prompt and that iTerm sometimes does not; neither was tried here.

**Route A: a bundle that carries the key (recommended).** `crates/mneme-audio/scripts/live-system-test.sh` builds the test binary, wraps it in `MnemeAudioLiveTest.app` (bundle ID `app.mneme.audio-live-test`, `NSAudioCaptureUsageDescription` set), signs it ad hoc and starts it with `open`. The bundle, its log and the WAV files go to `$TMPDIR/mneme-audio-live-test/`, because an app started this way appears to have no access to `~/Documents` (found here: `open --stdout` to a path under `~/Documents` failed with error -10810, and worked with a path outside it). Steps, each from the repository root, **untested beyond `--list`**:

```sh
# 0. Nothing is captured; checks that the harness works.
crates/mneme-audio/scripts/live-system-test.sh --list

# 1. Teardown. The first run should show the macOS prompt naming
#    "Mneme audio live test". Allow it. If the test fails with a permission
#    error while the prompt was open, run it again.
MNEME_LIVE_TEST_TARGET=lib crates/mneme-audio/scripts/live-system-test.sh live_tap_session

# 2. Manual check 1. Start music or a video in another app first.
crates/mneme-audio/scripts/live-system-test.sh live_system_audio_records_to_wav
afplay "$TMPDIR/mneme-audio-live-test/live-system.wav"

# 3. Stop all playback, then: do silent buffers keep arriving?
crates/mneme-audio/scripts/live-system-test.sh live_system_audio_runs_through_silence

# 4. Manual check 2. Keep everything else silent; a quiet tone plays.
crates/mneme-audio/scripts/live-system-test.sh live_system_audio_excludes_this_process

# 5. Manual check 3. Play audio and switch the output device during the run.
MNEME_LIVE_SECONDS=30 crates/mneme-audio/scripts/live-system-test.sh live_system_audio_follows_device_changes
```

What each should show:

- Step 1 prints the tap's real format (`tap N delivers TapFormat { ... }`) and the aggregate device's rate, and asserts that after teardown neither object answers and neither is in the system's lists. Please record the printed format: it settles item 2 above.
- Step 2 asserts frames arrived without gaps or errors, the sample count matches elapsed time, and the recording is not silent. The WAV must sound clean and at the right speed, and the music must have kept playing from the speakers.
- Step 3 asserts continuous frames with nothing playing. If it reports gaps or a "no audio ... re-opening" error, the tap does not deliver during silence on this macOS and the stall timeout for this source needs rethinking.
- Step 4 asserts the recording is silent (RMS under 0.005) while this process plays a tone.
- Step 5 asserts capture reached the end with under 5 s missing in total. The WAV has a short silence at each switch.
- **Manual check 4:** step 1 covers it from inside the process. From outside: after any run has exited, Audio MIDI Setup must show no device named "Mneme system audio". (The device is private, so it should not appear there even while a test runs.)

**Route B: straight from Apple's Terminal.** The permission is then Terminal's:

```sh
cargo test -p mneme-audio --test live_system -- --ignored --nocapture --test-threads=1 live_system_audio_records_to_wav
```

WAV files go to `target/tmp/`. Do not use an IDE's or another tool's terminal: the request would be attributed to that app.

**If something goes wrong:**

- Permission error, or a silent recording: System Settings > Privacy & Security > Screen & System Audio Recording > System Audio Recording Only, and enable "Mneme audio live test" (or Terminal). The system's own strings suggest the app may need to be quit and started again for a change there to apply.
- Permission is shown as allowed and `start` still fails with a permission error: run with `MNEME_SKIP_TAP_ACCESS_CHECK=1`. If the recording then has audio, the access check is wrong on this macOS (item 7) and must be fixed or removed. If it is silent, the permission really is missing.
- To be asked again from scratch: `tccutil reset AudioCapture app.mneme.audio-live-test`. Rebuilding the tests changes the ad hoc signature, so macOS may ask again by itself.

### Deviations and choices not spelled out in the issue

- **The tap is rebuilt on a device change rather than kept.** The issue says "without ending capture and without a jump in the meeting clock": both hold, at the cost of a short gap in the audio at the switch (the mic source does the same).
- **The permission "status" is `Option<PermissionStatus>` and unknown by default.** The issue asked for status and request "as far as the OS allows"; publicly it allows nothing. The private route exists but is opt-in.
- **`excluding_process` was added.** Only the calling process is excluded by default, and a Tauri app's web view may play its sound from a WebKit helper process (not verified). This lets the app name such processes.
- **`without_access_check` was added**, only so that a wrong access check can be told from a missing permission during the manual checks.
- **A fix in issue 002's `FakeSource`.** While other lanes were building, 2 of 25 plain runs of the suite failed (not captured), and a run under artificial CPU load failed in `fake::tests::a_finished_source_can_be_started_again`, an issue 002 test: the worker thread drops its frame sender a moment before it clears its running flag, and a `start` in between returned `AlreadyRunning`. `FakeSource::start` now recognizes a worker that has delivered its last frame. No signature changed. After the fix: 30 of 30 under load, 20 of 20 plain. The same window exists in `Capture` for a source that ended by itself (receiver dropped) and is restarted at once; it was left alone.
- **`AudioDeviceCreateIOProcID`, not the block variant** named in the task brief. Both are in the bindings; the function variant needs no dispatch queue and makes the context's lifetime explicit.
- **No example binary.** As in issue 002, the ignored tests are the example; `mneme-record` in issue 004 is the real one.

### Follow-ups

- `issues/README.md` still lists 003 as `in progress`; it is outside this issue's paths.
- The root `Cargo.lock` gained four lines (the new direct dependencies of `mneme-audio`; no new packages).
- **Before 004 builds on this, run the manual checks.** If step 1 or 2 fails, 004's manual checks cannot pass either.
- **004:** `Recorder` can take `Box::new(MicSource::new())` and `Box::new(SystemAudioSource::new())` as they are. `SystemAudioSource::start` may block for as long as the permission prompt is open, so start the two sources on separate threads or accept that the mic starts first. One third-party report (unverified) says the microphone should be started *before* the tap when AirPods are in use, or they stay in their music profile and the mic delivers nothing. Starting the mic is expected to switch AirPods to their call profile and change their output sample rate, which this source would see as a device change and answer with a rebuild: expect one short gap in `system.wav` near the start in that setup. `mneme-record` is a command-line binary: it needs the same bundle treatment as the live tests (or Terminal) to get the permission, and the script here can be the model. When padding with silence, note that a silent `system.wav` with no errors means "nothing played" or "permission lost mid-run"; the two cannot be told apart.
- **021:** `SystemAudioSource::start` and `request_system_audio_permission()` block and must run off the UI thread. `PermissionDenied` from `start` is the signal to send the user to System Settings; `NoOutputDevice` in `take_errors()` is not fatal. If the app ever plays sound through the web view, find the helper process's PID and pass it to `excluding_process`, then confirm with a recording that Mneme's own sound is absent.
- **024:** the app bundle's `Info.plist` needs `NSAudioCaptureUsageDescription` next to `NSMicrophoneUsageDescription`. Decide whether to enable `tcc-spi` (in `src-tauri/Cargo.toml`: `mneme-audio = { ..., features = ["tcc-spi"] }`): with it the first-run screen can show the real status and get the user's answer; without it the status is unknown until a tap has been opened, and the request may report "denied" while the prompt is still open. The Settings pane is "Screen & System Audio Recording", section "System Audio Recording Only" (read from the system's own strings on macOS 27.0.1); the prompt reads "“App” would like access to record your system audio." A change made in Settings may only apply after the app is restarted. Chromium's source warns that if the prompt is left unanswered for 60 s, Core Audio calls in that process fail until it is restarted.
- **025:** if `tcc-spi` is enabled, the signed build calls two private functions through `dlsym`. That rules out the Mac App Store. Whether notarization objects to it was not checked.
- Not built, and worth considering once real behavior is known: a listener on the aggregate device's `kAudioDevicePropertyDeviceIsAlive` (today a dead device is noticed by the 3 s stall check); re-resolving excluded processes when the system's process list changes (today only when the tap is rebuilt); a way to notice a tap that delivers only zeros.
