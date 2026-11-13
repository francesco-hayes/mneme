# mneme-audio

Microphone and system audio capture for Mneme, as 16 kHz mono frames on one shared clock. The API is documented in the source (`cargo doc -p mneme-audio --no-deps --open`). This file covers the command-line recorder, `mneme-record`.

> **Status: no real audio has been recorded with any of this yet.** The recorder, its file alignment, Ctrl-C handling and the bundle wrapper are tested with synthetic sources. The microphone and the system audio tap behind it have never been run against hardware (see issues 002 and 003). Every step below that opens a real device is untested and is marked so. The first real run is the test.

## What `mneme-record` does

```
mneme-record --out <dir> [--seconds <n>] [--fake]
```

It records the default microphone to `<dir>/mic.wav` and everything the Mac is playing to `<dir>/system.wav`, until Ctrl-C or for `n` seconds. Both files are 16 kHz mono 16-bit PCM.

The two files always have the same length and line up in time. Sample `n` of either file is the same moment of the recording. Where a channel has no audio, its file has silence of the same length instead:

| What happened | In that channel's file |
| --- | --- |
| The source came up late (for example a permission prompt was open) | silence at the front |
| The device changed or stalled mid-recording | silence in the middle |
| The source died, or never started | silence to the end |

A source that fails does not stop the other one. The summary at the end says, per file, how much was captured and how much is padding, what became of each source, and every error either source reported. The exit status is 0 only if both sources ran until the recording was stopped (1: a source failed or the files could not be written; 2: wrong usage).

Ctrl-C (or SIGTERM) stops the sources and finishes both files. A second Ctrl-C quits at once, without finishing them: the files are then still playable, because their headers are brought up to date every second, but they can be up to a second short and need not be equal in length.

`mneme-record --help` prints the full usage.

## Check the tool without touching any device

`--fake` records two synthetic tones through the same recording loop: 440 Hz as the microphone, and 880 Hz as system audio that starts half a second late. It opens no device and causes no permission prompt. These two commands have been run and work:

```sh
# From the repository root.
cargo run -p mneme-audio --bin mneme-record -- --out /tmp/mneme-fake --fake --seconds 3
crates/mneme-audio/scripts/record.sh --fake --seconds 3
```

Expect two files of exactly 3.00 s, with `system.wav` reporting 0.50 s of padded silence.

## Permissions

A real recording needs two macOS permissions:

| Permission | Needed for | Without it |
| --- | --- | --- |
| Microphone | `mic.wav` | If it was denied before, the microphone fails to start with a permission error. |
| System Audio Recording | `system.wav` | macOS reports no error and delivers silence. The recorder checks for this when it starts system audio and fails that source with a permission error, but that check has never been run. |

macOS grants both per app, not per program. A command-line program has no identity of its own, so which app is asked depends on how `mneme-record` is started. There are two ways.

## Way 1: from Apple's Terminal (untested)

The permissions are then Terminal's. Use Apple's Terminal.app, not a terminal inside an editor or another tool: the request would be attributed to that app, and some terminals never show the system audio prompt.

```sh
# From the repository root. Stop with Ctrl-C.
cargo run -p mneme-audio --bin mneme-record -- --out ~/mneme-recording
```

What should happen, none of it observed yet:

1. macOS asks whether Terminal may use the microphone. Allow it.
2. macOS asks whether Terminal may record system audio. Allow it. Starting system audio may wait while this prompt is open; the microphone records meanwhile, and `system.wav` gets silence for that stretch.
3. Lines such as `[  0.21 s] microphone: capturing` appear as each source comes up.

If no system audio prompt appears and the summary shows a permission error or a silent `system.wav`, use way 2.

## Way 2: through the bundle wrapper (untested with real devices)

`scripts/record.sh` builds `mneme-record`, wraps it in a small app bundle called "Mneme record" that carries the two usage descriptions macOS requires, signs it ad hoc, and starts it the way Finder would. The permissions then belong to "Mneme record", whatever terminal you use.

```sh
# From the repository root. Stop with Ctrl-C, or give --seconds.
crates/mneme-audio/scripts/record.sh
crates/mneme-audio/scripts/record.sh --seconds 60
crates/mneme-audio/scripts/record.sh --out /tmp/my-call
```

- The arguments are `mneme-record`'s. Without `--out`, the files go to a new folder under `$TMPDIR/mneme-record/`, and the summary prints the paths. The bundle has no access to Documents, Desktop or Downloads unless macOS is told to allow it, so prefer a folder outside those.
- Ctrl-C in the terminal is passed on to the recorder, which finishes both files.
- The recorder's output is shown as it comes and is also kept in `$TMPDIR/mneme-record/output.log`.

What has been run: the wrapper with `--help`, with `--fake --seconds N`, and Ctrl-C during a `--fake` recording. What has not: any run that opens the microphone or the tap, so whether macOS shows the prompts for "Mneme record" and grants it access is unverified.

Rebuilding changes the bundle's ad hoc signature, so macOS may ask again after a rebuild.

## Testing during a call

Untested; this is the manual check of issue 004.

1. Put on headphones. With speakers, the other side of the call is picked up by the microphone and ends up in both files.
2. Join a call (Zoom, Meet, FaceTime) with someone who will talk, or play a video as a stand-in for the other side.
3. Start the recorder, either way above. Answer the permission prompts. If a prompt was open for a while, start again so the recording is clean from the beginning.
4. Talk for a bit, let the other side talk, talk over each other once. Then press Ctrl-C.
5. Read the summary. Expected: both sources "captured until the recording stopped", no source errors, little or no padded silence, and a peak level for both files.
6. Listen to each file:
   ```sh
   afplay <dir>/mic.wav       # only you
   afplay <dir>/system.wav    # only the other side
   ```
   Both should be clean and at normal speed.
7. Check that they line up: open both files as two tracks in an audio editor (Audacity, for example) and play them together. The conversation should sound as it did, with no echo or offset between the tracks.

Worth trying afterwards: switch the output device mid-recording (plug in headphones, connect AirPods). Neither file should stop; `system.wav` gets a short gap of silence at the switch, which the summary reports as a gap.

## Reading the summary

```
system.wav: 61.20 s captured, 2.30 s silence padded, peak -14.2 dBFS
  audio from 2.30 s to 63.50 s
  system audio: captured until the recording stopped
```

- **captured / silence padded**: how much of the file came from the source and how much is silence written where the source delivered nothing.
- **peak**: the loudest captured sample. "all of it is digital silence" means the source delivered only zeros.
- **ENDED BY ITSELF**, **FAILED TO START**, **NEVER STARTED**: the source did not run to the end. Its errors follow. The other file is still complete.
- **error** lines under a source that kept running are things it recovered from, such as re-opening after a device change.

A silent `system.wav` with no errors is not a success, and the summary says so in a note. It means one of two things that cannot be told apart from inside the program: nothing was playing, or macOS is withholding system audio (the permission is missing or was revoked mid-recording). Play something and record again to find out.

## If something goes wrong

- **Permission error for system audio, or `system.wav` is silent while audio was playing.** System Settings > Privacy & Security > Screen & System Audio Recording > System Audio Recording Only: enable Terminal (way 1) or "Mneme record" (way 2), then run again.
- **The permission is shown as allowed and system audio still fails with a permission error.** The access check itself may be wrong; it has never been run. Start with `MNEME_SKIP_TAP_ACCESS_CHECK=1` set (both ways pass it on). If `system.wav` then has audio, the check is at fault: please report it. If it is silent, the permission really is missing.
- **Permission error for the microphone.** System Settings > Privacy & Security > Microphone: enable Terminal or "Mneme record".
- **To be asked again from scratch (way 2):** `tccutil reset Microphone app.mneme.record` and `tccutil reset AudioCapture app.mneme.record`.
- **"NEVER STARTED" for a source.** Its start had not returned when the recording stopped, most likely because a permission prompt was still open. Answer the prompt and record again.
- **AirPods and an empty `mic.wav`.** One unverified report says AirPods only deliver microphone audio if the microphone is opened before the system audio tap. The recorder starts the microphone first for that reason. If the microphone is still silent with AirPods, note it in issue 004.

## Using the recorder from code

`Recorder` is the reusable part: it starts any set of `AudioSource`s on one clock, each on its own thread, and gives back one stream of frames plus the state and errors of each source. `RecordingWriter` writes that stream as the two aligned WAV files. `record_to_wav` is the two together, and is what `mneme-record` calls.

```rust
use mneme_audio::{AudioSource, MicSource, Recorder, SystemAudioSource};

// The microphone first: sources are started in this order.
let sources: Vec<Box<dyn AudioSource>> =
    vec![Box::new(MicSource::new()), Box::new(SystemAudioSource::new())];
let (recorder, frames) = Recorder::start(sources); // returns at once

for frame in frames.iter().take(1_000) {
    // frame.speaker, frame.start_ms, frame.samples
}
for source in recorder.status() {
    // source.phase, source.errors
}

let report = recorder.stop();   // stops both; the stream then ends
for frame in frames.iter() {
    // what was captured up to the stop
}
```
