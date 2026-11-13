# 004: Two-channel recorder and `mneme-record` CLI

Status: done · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 003 · Owns: `crates/mneme-audio/`

## Goal

Run the mic and system sources together on one clock, and prove build step 1 with a command-line recorder that writes two WAV files.

## Scope

1. **`Recorder`**: takes two `AudioSource`s, starts both with one shared start instant, exposes one stream of `AudioFrame`s (both speakers), stops both. If one source fails to start or dies, the other keeps going and the failure is reported.
2. **`mneme-record` binary**: `--out <dir>` and optional `--seconds N`. Records until Ctrl-C or the limit, writes `mic.wav` and `system.wav`, prints duration and any source errors. Each file is padded with silence where its source had no frames, so both files have the same length and line up in time.
3. Usage notes in the crate README: how to run it, which permissions it asks for, and how to test during a call.

## Acceptance criteria

- [x] Test with two fake sources: both files written, same length, offsets preserved (a source that starts late is padded at the front).
- [x] Test: one fake source fails mid-run; the other file is complete and the error is reported.
- [x] `cargo run -p mneme-audio --bin mneme-record -- --help` works.
- [x] `cargo test -p mneme-audio` and clippy pass.

## Manual checks

- [ ] During a real Zoom call with headphones: `mic.wav` has only me, `system.wav` has only the other side, both clean.
- [ ] The two files line up when played together.

## Result

Implemented on 2026-11-12. All four acceptance criteria pass. **No real audio has been recorded.** The hard limits of this run forbade opening the microphone or a tap, so `mneme-record` has only ever run with synthetic sources (`--fake`). The microphone and tap code from issues 002 and 003 has still never met hardware. Both manual checks are open, and they are the first real test of issues 002, 003 and 004 together.

### What was built

All in `crates/mneme-audio/`. Everything below is re-exported from the crate root; the two new modules are also public (`mneme_audio::recorder`, `mneme_audio::recording`) so their module docs are rendered.

**`Recorder`** (`src/recorder.rs`): runs a list of sources as one recording.

| Item | Summary |
| --- | --- |
| `Recorder::start(sources: Vec<Box<dyn AudioSource>>) -> (Recorder, FrameReceiver)` | Reads the start instant once, starts every source in the background against it, returns at once. The receiver carries every source's frames. |
| `Recorder::start_with(sources, RecorderOptions)` | The same with settings. |
| `RecorderOptions { head_start: Duration }` | How long a source's `start` is given before the next one is started alongside it. Default 1 s. Zero starts all at the same moment. |
| `started_at() -> Instant`, `elapsed() -> Duration` | The shared start instant and meeting time now. |
| `status() -> Vec<SourceStatus>` | Every source's state right now, in the order given. Never waits for a source. |
| `is_finished() -> bool` | No source is starting or running. |
| `stop(self) -> RecorderReport` | Stops every source and reports. Afterwards `frames.iter()` drains the receiver and ends. Dropping a recorder also stops it. |
| `SourceStatus` | `speaker`, `phase`, `errors: Vec<AudioError>` (the start error, then everything from `take_errors`, oldest first), `started_ms`, `frames`, `samples`, `first_frame_ms`, `end_ms`; `captured() -> Duration`. |
| `SourcePhase` | `Starting`, `Running`, `FailedToStart`, `Ended` (stopped delivering by itself), `Stopped` (by `stop`), `Abandoned` (still starting when the recorder stopped). `is_live()`, `is_failure()`. |
| `RecorderReport` | `duration` (meeting time at `stop`), `sources`; `source(speaker)`, `errors() -> Vec<(Speaker, AudioError)>`. |

**Aligned WAV files** (`src/recording.rs`):

| Item | Summary |
| --- | --- |
| `TrackWriter` | One speaker's frames into one WAV laid out in meeting time. `create(speaker, path)`, `limited_to(duration)`, `write_frame(&frame)`, `samples()`, `flush()`, `finish(samples) -> TrackSummary`. |
| `RecordingWriter` | The pair: `mic.wav` for `Speaker::Me`, `system.wav` for `Speaker::Them`. `create(dir)`, `limited_to(duration)`, `write(&frame)`, `flush()`, `finish(at_least: Duration) -> [TrackSummary; 2]`. Both files end at one length. |
| `TrackSummary` | `speaker`, `path`, `samples`, `captured_samples`, `padded_samples`, `dropped_samples`, `gaps`, `peak`, `first_audio_ms`, `last_audio_ms`; `duration()`, `captured()`, `padded()`, `is_silent()`. |
| `record_to_wav(sources, out_dir, &RecordOptions, stop: &AtomicBool, on_event) -> Result<Recording, AudioError>` | The recording loop behind the CLI. Ends on the stop flag, the time limit, or when every source has ended. |
| `RecordOptions { limit: Option<Duration>, recorder: RecorderOptions }`, `StopReason`, `RecordEvent`, `Recording { reason, duration, tracks, sources }` | Its settings, live events and result. `Recording::any_source_failed()`. |
| `MIC_WAV`, `SYSTEM_WAV` | The file names. |

**`mneme-record`** (`src/bin/mneme-record.rs`): `--out <dir> [--seconds <n>] [--fake]`, `-h`/`--help`. A hand-written parser (also accepts `--name=value`; `--seconds` takes decimals). It only parses arguments, installs the signal handlers, calls `record_to_wav` and prints. Exit status: 0 recorded and both sources ran to the end; 1 a source failed or the files could not be written; 2 wrong usage.

**Scripts**: `scripts/bundle.sh` (new, sourced: wraps a binary in an app bundle carrying both usage descriptions, signs it, runs it through `open`), `scripts/record.sh` (new: `mneme-record` through that bundle), `scripts/live-system-test.sh` (now uses `bundle.sh`; same behavior).

**`crates/mneme-audio/README.md`** (new): what the tool does, the two ways to run it, permissions, testing during a call, reading the summary, troubleshooting.

### How the recorder handles a start that blocks, fails or panics

- **Each source is started on a thread of its own** and stays with that thread (its supervisor) for its whole life. `Recorder::start` returns at once. A `SystemAudioSource::start` waiting on a permission prompt holds up nothing: the microphone is already delivering.
- **The clock is read once**, before any thread is spawned, and every source is given that same `Instant`. A source that comes up late stamps its first frame with the time it was captured (that is the `Framer`'s job, from issue 002), so its lateness shows as a later `start_ms`, not as a shifted clock.
- **Start order.** Sources are started in the order given. The next one is started when the previous `start` has returned, or after `head_start` (1 s) if it has not. The CLI passes the microphone first. This follows the unverified report that AirPods need the microphone opened before the tap; it costs nothing when the report is wrong, and a blocked microphone start delays system audio by at most a second.
- **A start that fails**: the error is the first entry of that source's `errors`, the phase is `FailedToStart`, the other source is untouched.
- **A source that dies mid-run**: its supervisor sees the source drop its frame sender (or `is_running()` turn false), collects its errors, and marks it `Ended`. The phase changes only after that source's last frame is in the stream.
- **A source that panics** (in `start` or later): caught on its thread, recorded as that source's failure with the panic message, and the stream can still end.
- **Stop while a start is still blocked**: `stop()` does not wait for it. The source is marked `Abandoned`, nothing it delivers later reaches the receiver, and it is stopped as soon as its `start` returns. Running sources are told to stop all at once and then waited for.
- **The consumer drops the receiver**: the sources are stopped and `AudioError::SinkClosed` is recorded for each.

How frames travel: each source sends to a private channel and its supervisor forwards to the one receiver the caller holds. This differs from the earlier note ("clone one `FrameSender` into both sources"). With a shared sender, a source stuck in `start` would hold a sender forever and the stream could never end after `stop`. The forwarding hop also gives the exact moment a source ended and per-source counts for free.

### How the two files are aligned and padded

A frame's `start_ms` is its position on the shared clock, so it is also its position in the file: sample `start_ms * 16`. Before writing a frame, `TrackWriter` writes silence up to that sample. That one rule gives:

- silence at the front for a source that started late;
- silence in the middle for a gap in `start_ms` (device change, stall);
- at the end, both files are padded to one length, which covers a source that died early or never started.

Nothing is ever shifted. A frame that would start before audio already written loses its overlapping samples (no source in this crate produces one; the summary would flag it).

Length of both files:

| Ended by | Length |
| --- | --- |
| `--seconds N` | exactly N seconds; audio stamped later is left out |
| Ctrl-C | the meeting time at which the sources were told to stop (or the end of the last frame if one reaches past it) |
| every source ending | the end of the last audio captured |

The WAV headers are brought up to date every second and once more before the sources are stopped, so a killed process or a hanging device leaves playable files.

### Ctrl-C

`signal-hook` 0.4.6 (`default-features = false`), the newest version on crates.io on 2026-11-12, read in the downloaded source. SIGINT, SIGTERM and SIGHUP set an `AtomicBool` (`signal_hook::flag::register`); the recording loop sees it within 25 ms, stops the sources and finishes both files. A second signal exits at once with status 130 (`flag::register_conditional_shutdown`, registered first, as the crate's documentation prescribes for a double Ctrl-C), in case stopping a device hangs. Chosen over `ctrlc` because setting a flag is all that is needed and it covers SIGTERM without a feature; chosen over raw `libc::signal` because that needs unsafe code here.

### The `Capture` race from issue 003

It is real, and wider than described. A capture thread that ends by itself (the frame receiver went away) queues `SinkClosed`, then closes its stream, then drops its sender, and only then reports stopped. Closing a real stream or tap can take a while, and for all of that time `MicSource::start` and `SystemAudioSource::start` answered `AlreadyRunning`, although the source's owner had already been told it was over.

Fixed in `src/capture.rs`: the capture thread sets an `ended` flag before it queues the error, and both sources' `start` now go through `capture::make_ready`, which refuses only a capture that is live and otherwise joins the old thread (waiting for it to release the stream) before starting again. Two tests: `a_capture_that_ended_by_itself_can_be_started_again_at_once` holds the window open with a scripted stream that is slow to close (it fails with `AlreadyRunning` when the fix is disabled), and `a_running_capture_refuses_a_second_start`. No public signature changed. The `Recorder` itself never restarts a source, so it did not depend on this.

### Verification

Run on macOS 27.0.1, Apple Silicon (12 cores), rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-audio` | lib: 168 passed (131 from before, 37 new), 1 ignored. `mneme-record` unit tests: 11 passed. `tests/record_cli.rs`: 5 passed. Doc-tests: 4 passed. Live tests: 6 ignored. 0 failed. |
| `cargo test -p mneme-audio --features tcc-spi` | same counts |
| The lib test binary, 30 runs in a row | 30 of 30 passed |
| The `mneme-record` unit tests, 30 runs; the CLI tests, 10 runs | 30 of 30; 10 of 10 |
| The same with 12 busy-loop processes competing for the CPU: lib 30 runs, unit tests 10, CLI tests 10 | 30 of 30; 10 of 10; 10 of 10 |
| `cargo clippy -p mneme-audio --all-targets -- -D warnings` | clean |
| `cargo clippy -p mneme-audio --all-targets --features tcc-spi -- -D warnings`, `cargo clippy -p mneme-audio` | clean |
| `cargo fmt -p mneme-audio -- --check` | clean |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p mneme-audio --no-deps` | clean |
| `shellcheck -s sh -x scripts/*.sh` | clean |
| `cargo run -p mneme-audio --bin mneme-record -- --help` | prints the usage, exit 0 |
| `mneme-record --seconds 3` (no `--out`) | "error: --out <dir> is required", exit 2 |
| `mneme-record --out <dir> --fake --seconds 1.5` | two files of 24 000 samples; summary shows 0.50 s padded for `system.wav`; exit 0 |
| `mneme-record --fake --seconds 30`, then SIGINT after 1.2 s | exit 0, "interrupted", two finished files of equal length |
| The same with SIGTERM and with SIGHUP | exit 0, two finished files of equal length |
| Two SIGINTs 5 ms apart | exit 130 at once; both files still valid WAVs (15 680 and 16 000 samples: not finished, so not equal) |
| `scripts/record.sh --help` | built, bundled, signed, started through LaunchServices, printed the help, exit 0 |
| `scripts/record.sh --fake --seconds 2` | two files of 2.00 s under `$TMPDIR/mneme-record/`, exit 0 |
| `scripts/record.sh --fake --seconds 40`, then SIGINT to the script's process group (what a terminal's Ctrl-C does) | the recorder reported "interrupted" and finished two files of equal length; the script returned 2.2 s after the signal with exit 0; no process left |
| `scripts/live-system-test.sh --list` (both targets) after the refactoring | lists the ignored tests, exit 0. Captures nothing. |

Where each criterion is covered:

- **Two fake sources, same length, offsets preserved.** `recording::tests::two_sources_give_two_files_of_one_length_with_offsets_kept`: a 1 s microphone tone from 0 and system audio that starts 500 ms late and runs 300 ms longer, through `record_to_wav`. Both files are 20 800 samples; `system.wav` is zero for exactly its first 8 000 samples and then equals the source sample for sample; `mic.wav` equals its source and is zero after 16 000. Also: `a_gap_in_a_source_becomes_silence_of_the_same_length`, `the_time_limit_gives_files_of_exactly_that_length`, `audio_stamped_past_the_limit_is_left_out`, `a_track_places_frames_by_their_timestamp`, `a_track_never_shifts_audio_to_make_room`, and through the built binary `a_timed_fake_recording_writes_two_aligned_files`.
- **One source fails mid-run.** `recording::tests::a_source_that_fails_mid_run_leaves_the_other_file_complete`: system audio dies after 250 ms with an error; `mic.wav` equals its 2 s source in full, `system.wav` has its 4 000 samples then silence to the same length, the error is in the result and was reported as an event once. `a_source_that_dies_while_the_other_records_on_in_real_time` is the same in real time, checking the death is announced while recording continues. Also `a_source_that_fails_to_start_gives_a_silent_file_of_the_same_length`, `no_source_starting_still_gives_two_valid_files`, and in `recorder::tests`: `a_source_that_dies_mid_run_is_reported_while_the_other_carries_on`, `a_source_that_fails_to_start_does_not_stop_the_other`, `a_source_that_panics_is_a_failed_source_not_a_dead_recorder`.
- **`--help` works.** Run by hand (above) and in `tests/record_cli.rs::help_prints_the_usage_and_succeeds`.
- **Tests and clippy pass.** The table above.

Other behavior with tests: a blocked start does not hold up the other source and both get the same instant (`a_start_that_blocks_does_not_hold_up_the_other_source`, `every_source_is_given_the_same_start_instant`); stop does not wait for a blocked start and the stream still ends (`stop_does_not_wait_for_a_start_that_never_returns`, `interrupting_while_a_start_is_blocked_still_finishes_both_files`); start order and its bound (`sources_start_in_order_and_the_next_waits_for_the_previous`, `the_head_start_bounds_how_long_the_next_source_waits`); sources are stopped together (`sources_are_stopped_together_not_one_after_the_other`); the stop flag finishes both files (`the_stop_flag_ends_the_recording_with_both_files_finished`); a real SIGINT to the built binary (`ctrl_c_finishes_both_files`); the summary calls out a silent `system.wav` (`a_silent_system_file_is_called_out_not_passed_off_as_success`).

**The new tests were checked to fail when the behavior is broken.** Sixteen deliberate breakages, each applied alone and reverted: no padding before a frame; no padding at the end; source errors never collected; the start error dropped; each source given its own clock; the limit not applied; overlapping audio shifted instead of dropped; the stop flag ignored; the head start unbounded; the next source not waiting its turn; a dead source reported as stopped; a panic not contained; one source stopped 400 ms after the other; the signal handler not registered; the silent-file note removed; and the `Capture` fix disabled. Each made at least one test fail. Two of them showed as a test that did not finish within the 120 s allowed rather than as a failed assertion: the uncontained panic (the stream never ends) and the unregistered handler (the signal is swallowed and the recording runs on). The fifteen in the recorder, the writer and the CLI were run again against the final code.

### What has never been executed

1. `mneme-record` without `--fake`. The branch that builds `MicSource` and `SystemAudioSource`, prints the permission status and honors `MNEME_SKIP_TAP_ACCESS_CHECK` has only been compiled.
2. `Recorder` with a real source. How long a real `start` or `stop` takes, whether a tap's `start` really blocks under a permission prompt, and what the head start does to AirPods are all unobserved.
3. `scripts/record.sh` capturing. Whether macOS shows the Microphone and System Audio Recording prompts for the "Mneme record" bundle, and grants an ad hoc signed bundle access, is unverified. Only `--help` and `--fake` runs went through it.
4. Running from Apple's Terminal (way 1 in the README). Nothing was run from Terminal.app; all runs here were from an agent's shell.
5. The summary's notes against real audio: the silent-file note is tested with constructed results, not with a tap that delivers zeros.
6. The `Capture` fix on a real device. It is tested with a scripted stream; `MicSource::start` and `SystemAudioSource::start` after a self-ended capture were not run.
7. Everything in "What has never been executed" of issue 003, and the "Not verified" list of issue 002. Nothing in this issue ran any of it. `scripts/live-system-test.sh` after its refactoring ran only with `--list`.
8. A long recording: memory use (the frame channels are unbounded), drift between the two device clocks, hours-long files.
9. Any platform other than macOS. `signal-hook` and the SIGHUP line are behind `cfg(unix)` where needed, not compiled elsewhere.

### Running the manual checks

Full instructions are in `crates/mneme-audio/README.md`. In short, **all untested**:

```sh
# 0. Check the tool and the wrapper without touching a device (these work).
cargo run -p mneme-audio --bin mneme-record -- --out /tmp/mneme-fake --fake --seconds 3
crates/mneme-audio/scripts/record.sh --fake --seconds 3

# 1. Before a call: issue 003's steps 1 and 2 (crates/mneme-audio/scripts/live-system-test.sh),
#    because a failure there is easier to read than one inside a recording.

# 2. Headphones on. Join a Zoom call. Then either, from Apple's Terminal.app:
cargo run -p mneme-audio --bin mneme-record -- --out ~/mneme-recording
#    or, from any terminal, through the bundle (files go under $TMPDIR/mneme-record/):
crates/mneme-audio/scripts/record.sh

# 3. Allow the Microphone and System Audio Recording prompts. If a prompt was
#    open for a while, press Ctrl-C and start again for a clean recording.
# 4. Talk, let the other side talk, talk over each other once. Press Ctrl-C.
# 5. Read the summary, then listen:
afplay <dir>/mic.wav       # manual check 1: only you
afplay <dir>/system.wav    # manual check 1: only the other side
# 6. Manual check 2: open both files as two tracks in an audio editor and play
#    them together. The conversation should sound as it did.
```

Expected summary: both sources "captured until the recording stopped", "Source errors: none", little or no padded silence, a peak level for both files, exit status 0. Please record what the first real run prints, including the two permission lines at the top and how long each source took to start.

If it goes wrong, the README's "If something goes wrong" section covers a missing permission, a suspected wrong access check (`MNEME_SKIP_TAP_ACCESS_CHECK=1`), `tccutil reset`, and AirPods.

### Deviations and choices not spelled out in the issue

- **`Recorder` takes a list of sources, not exactly two.** Two is the only case used; a list made the code no longer and the tests simpler.
- **Sources do not share one `FrameSender`.** See "How frames travel" above.
- **`Recorder::start` cannot fail and does not wait.** A source's start error is in `status()`, not in a return value, because one failed source must not fail the recording.
- **`--fake` was added.** It is the only way the binary's main path, its signal handling and the bundle wrapper could be run at all under this issue's limits, and it lets the user check the tool before a call. It needs `--seconds` (at most 600).
- **A silent `system.wav` does not change the exit status.** It cannot be told from "nothing was playing", so the summary says so in a note instead of calling it a failure or a success.
- **A fake source that plays to its end reads as `Ended`**, the same as a live source that died. A live source never ends by itself in good health, so `is_failure()` treats `Ended` as a failure. Consumers testing with fakes should compare phases, not call `is_failure()`.
- **Exit status 1 when a source did not run to the end**, although both files are written and valid. For a tool whose purpose is to prove capture works, a half-empty recording should not pass as success in a script.
- **The modules `recorder` and `recording` are public.** `rustdoc` refuses links from public items to private module docs, and the design is explained there.
- **`live-system-test.sh` now also puts `NSMicrophoneUsageDescription` in its bundle** (the shared helper writes both keys). Otherwise unchanged.

### Follow-ups

- `issues/README.md` still lists 004 as `in progress`; it is outside this issue's paths.
- **`Cargo.lock`**: `signal-hook` 0.4.6 and `signal-hook-registry` 1.4.8 were added by this issue. The lockfile at `HEAD` already contains them (it was committed with another lane's work while this one was in progress), so the lockfile and `crates/mneme-audio/Cargo.toml` agree once this issue is committed.
- **Before anything else builds on audio: run the manual checks.** Three issues of capture code now rest on zero real recordings.
- **009 (live session):** use `Recorder`, do not start the sources yourself.
  - `let (recorder, frames) = Recorder::start(vec![mic, system]);` with the microphone first. `frames` is a `std::sync::mpsc::Receiver<AudioFrame>`: read it on a blocking thread, not on the async runtime.
  - Flush a speaker's chunker when its source ends: poll `recorder.status()` (cheap) and act when a phase becomes `Ended` or `FailedToStart`. A source reads as `Ended` only after its last frame is in the stream, so drain the receiver first, then flush.
  - At stop: `let report = recorder.stop();` then drain `frames.iter()` (it ends), then flush both chunkers. `report.errors()` and `report.sources` are the per-source failures for the session summary; `report.duration` is the meeting duration. `stop()` does not wait for a source still stuck in `start`; it reports it as `Abandoned`.
  - Elapsed time for the status: `recorder.elapsed()`.
  - `MNEME_KEEP_AUDIO`: `RecordingWriter::create(folder)`, `write(&frame)` for every frame, `flush()` now and then, `finish(report.duration)` at stop. It pads and aligns the two files as the CLI does.
  - With fakes, a source that runs out reads as `Ended`; see the deviation above.
  - The frame channels are unbounded. If the consumer stalls, memory grows by about 64 kB per second of audio per source.
- **021 (Tauri commands):** `Recorder::start` returns at once and never blocks, so it can be called from a command handler; `stop()` waits for running sources to close their devices and belongs off the UI thread. For the recording bar, `status()` gives each source's phase: `Starting` for more than a second or two most likely means a permission prompt is open. `PermissionDenied` as the first error of a `FailedToStart` source is the signal to send the user to System Settings. An `Abandoned` source may still be blocked inside `start` on a detached thread when a new recording begins; starting a second tap while the first prompt is unanswered is untested.
- **024 (permissions flow):** a recording started while a prompt is open has silence at the front of that channel until the prompt is answered. If that matters, request both permissions before starting the recorder.
- Not built: a way to tell the recorder to give up on a source that has been `Starting` for too long; a level meter or progress line while recording; a check for a tap that delivers only zeros while the recording is running (today it is reported at the end).
