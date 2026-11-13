# Mneme: codebase walkthrough

Written 2026-11-12, at the end of the first build session. It explains how the code is laid out, how a meeting flows through it, and why each decision was made and by whom. Status of each issue is in [issues/README.md](../issues/README.md); what is weak or untested is in [known-gaps.md](known-gaps.md).

## What exists

A macOS desktop app (Tauri 2: Rust backend, React frontend) that records the microphone and system audio as two channels, transcribes both with a local Whisper model, and stores each meeting as a folder of Markdown files. The backend crates, the UI and the wiring between them are all built and tested. The app has been compiled but never launched, and nothing has run on a real call.

## Layout

```
mneme/
  mneme-scope.md        the scope doc and both decision logs (source of truth)
  prds/                 one PRD per build area
  issues/               one file per issue: scope, criteria, result, known gaps
  docs/                 this file, known-gaps.md, release.md
  scripts/              dev signing, release signing, local-only audit
  crates/
    mneme-core/         shared types
    mneme-audio/        mic + system audio capture, recorder, mneme-record
    mneme-transcribe/   Whisper engine, chunker, model catalog + downloader, mneme-bench
    mneme-store/        meeting folders, transcript, notes, timeline, list, recovery, settings
    mneme-pipeline/     the live session that joins the three above
  src-tauri/            the app: commands, events, app state
  src/                  the React UI
```

Each crate builds and tests alone (`cargo test -p <crate>`). The frontend runs alone against a mock (`pnpm dev`).

## How a meeting flows through the code

1. **New meeting** in the UI calls `api.startMeeting()` (`src/lib/api/tauri.ts`), which invokes the `start_meeting` command in `src-tauri`.
2. The app core (`AppCore` in `src-tauri/src/`) checks that a default model is installed, creates the meeting folder through `mneme_store::Store::create_meeting`, loads the model into a `WhisperEngine`, builds a `MicSource` and a `SystemAudioSource`, and starts a `mneme_pipeline::Session`.
3. Inside the session, `mneme_audio::Recorder` starts both sources on one clock. Each source converts whatever the device delivers to 16 kHz mono and stamps frames with meeting time.
4. One `Chunker` per speaker cuts the frames at pauses and drops silence. Chunks go to a queue.
5. One worker thread takes chunks oldest first, runs them through the engine, and appends each line to `transcript.md` through `TranscriptWriter`, which syncs every line to disk.
6. Meanwhile the editor autosaves notes through `save_notes`. While that meeting is recording, the store also appends changed lines with the meeting time to `notes-timeline.jsonl`.
7. **Stop** stops the sources, flushes the chunkers, drains the queue, sorts the transcript, and `finish_meeting` writes the duration and status and applies any pending folder rename.
8. On the next launch, `Store::recover()` repairs any meeting a crash left marked as recording.

## The crates, one at a time

### mneme-core
Four things every crate agrees on: `SAMPLE_RATE` (16,000), `Speaker` (`Me`, `Them`), `AudioFrame`, `Segment`, and the `hh:mm:ss` timestamp functions. Nothing else lives here, on purpose.

### mneme-audio
- `AudioSource` is the trait both sources implement. Code that consumes audio never knows which API is behind a source.
- `Framer` is the meeting clock: the wall clock places a stream's first sample once, then time advances by sample count, so timestamps do not jitter.
- `MicSource` uses cpal. `SystemAudioSource` uses a Core Audio process tap inside a private aggregate device, excluding Mneme's own process. Both re-open when the default device changes.
- `Recorder` starts sources on separate threads against one start instant and reports each source's phase and errors.
- `mneme-record` writes `mic.wav` and `system.wav`, padded with silence so they line up. It is a development tool.

### mneme-transcribe
- `Transcriber` is the trait; `WhisperEngine` implements it with whisper-rs and Metal; `FakeTranscriber` is for tests.
- `Chunker` detects speech by loudness against an adaptive noise floor, cuts after 700 ms of pause, and force-cuts at 28 s.
- `catalog()` lists ten Whisper models with sizes and SHA-256 values read from Hugging Face at a pinned revision. `ModelStore` downloads to a `.part` file, verifies, then renames.
- This crate holds the only code in the app allowed to use the network.

### mneme-store
- Everything is plain files. `Store` creates, finds, renames and finishes meeting folders.
- `frontmatter.rs` is a small line-preserving parser: changing one key rewrites one line and leaves every other byte alone.
- `safe_write` writes a temp file, syncs, and renames, so a crash never leaves a half-written file.
- `TranscriptWriter` appends and syncs each line, and follows the folder if it is renamed.
- `list_meetings` scans folders and returns warnings for anything it skips. `recover` marks crashed meetings as interrupted.
- `SettingsStore` keeps `settings.json`.

### mneme-pipeline
`Session` is the live meeting: a capture thread owns the recorder and both chunkers, a transcription thread owns the engine reference and the transcript sink. It uses plain threads, no async runtime.

### src-tauri
Commands and events for every method in the API contract, one `AppCore` holding the stores and the current session, recovery at launch, a single-instance lock, and a clean stop on quit.

### src (frontend)
- `src/lib/api/`: the contract (`types.ts`), a mock, and the Tauri implementation. The UI imports one `api` object and never calls the backend any other way.
- `src/state/`: navigation (a hand-rolled route stack), recording state, and the model library (downloads and the default-model rule).
- `src/screens/`: First run, Home, Meeting (live and after), Settings.
- `src/editor/`: CodeMirror 6 mounted in a shadow root.

## Decisions and why

### Made by Francesco before the build (from the scope doc)

| Decision | Why |
| --- | --- |
| Everything stays on the machine | Privacy; no accounts or servers. |
| Tauri (Rust + React), macOS first | Small binary, native audio access, direct Whisper bindings; system audio is per-OS. |
| Whisper models only, downloaded on demand | One engine keeps v1 simple; a small installer. |
| Live transcription, hidden during the meeting | Transcript ready at stop; attention stays on the notes. |
| Me vs. them from separate channels | A speaker split with no diarization model. |
| No stored audio | The notes are the product; nothing sensitive lingers. |
| Meetings as Markdown folders; SQLite later as a rebuildable index | Plain data the user owns; no schema or export step. |
| Folder names `YYYY-MM-DD HHMM Title`; stable id in frontmatter | Folders sort by time; a rename never breaks identity. |
| Note timing saved from v1 | The time a line was typed cannot be recovered later. |
| CodeMirror 6, styled raw Markdown | The file on disk matches the editor. |
| The UI decisions in the design log | Notes first; quiet during meetings; recording state always visible. |

### Made by Francesco during the build

| Decision | Why |
| --- | --- |
| Git repo with one commit per issue | Every piece of work is separately reviewable and revertable. |
| Update Rust to current stable and install cmake | The installed Rust (1.70) could not build Tauri 2; whisper.cpp needs cmake. |
| Core Audio process taps for system audio; minimum macOS 14.4 | Audio-only permission with no recurring re-prompt, unlike ScreenCaptureKit. |
| A second meeting in the same minute gets a numbered folder | A recording must never fail to start. The scope doc had assumed two meetings could not share a minute. |
| Note timeline entries stay `{"t","text"}` with no link to replaced text | Keeps the format as designed; drafts of a growing line are matched by prefix later. |
| v1 is English only; no language setting | The app was only ever meant for English. The risk-table row that suggested a language picker was not MVP. |
| Benchmark is low priority; download base, small and large-v3-turbo for it | Enough to cover the realistic first-run choices. |
| Start 021 before the last UI screen; run 025 alongside 022–024 | Shortens the serial tail of the plan. |
| Lean agent briefs; note gaps instead of chasing them; hard stops | The first briefs asked for too much proof and cost too much time and tokens. |

### Made by Claude (the orchestrator) to structure the build

| Decision | Why |
| --- | --- |
| One crate per area in a Cargo workspace | Each area builds and tests without the UI, and parallel work never touches the same files. |
| One typed API module with a mock | The UI could be built and tested before the backend existed. |
| Sources, transcriber and transcript sink are traits with fakes | The live session is tested end to end with no microphone, model or permissions. |
| Meetings in `~/Documents/Mneme`; models and settings in the app data folder; bundle id `app.mneme.desktop` | Defaults to unblock the build; the scope doc did not say. Easy to change. |
| Headphones recommended, no echo cancellation | The scope doc left speaker bleed open; this is the cheap default. Still open. |
| Agents may not open the mic, create taps or launch the app unattended | A permission prompt would appear on screen under the wrong app, or with nobody there. |

### Made by the implementing agents and accepted

| Decision | Why | Issue |
| --- | --- | --- |
| whisper-rs pinned to 0.16.0, used in one file | It is slow-moving (eight months since a release) but builds and runs with Metal; pinning and isolating it makes it replaceable. | 005 |
| `objc2-core-audio` called directly for taps | It exposes every symbol needed and was already in the tree through cpal; cpal's own loopback cannot exclude Mneme's process. | 003 |
| Tap rebuilt when the output device changes | Simplest correct behavior; costs a short gap at the switch. | 003 |
| Private macOS permission API behind an off-by-default feature | macOS has no public way to read the system-audio permission; the private route would rule out the App Store. | 003 |
| Energy-based speech detection, no model | Dependency-light; a model-based detector is the next step if real recordings need it. | 006 |
| `ureq` as the HTTP client; HTTPS checked on every redirect hop | Blocking, small, supports cancel; it is the only network-capable crate. | 007 |
| Model URLs pinned to one repository commit | A checksum can never go stale because upstream replaced a file. | 007 |
| Hand-written frontmatter parser | A YAML library rewrites the whole block and loses comments and hand edits. | 010 |
| Every transcript line is synced to disk | Audio is discarded after transcription, so a lost line cannot be regenerated. About 4 ms per line. | 011 |
| Duplicate meeting ids are refused, not guessed | Two folders cannot be one meeting; the error names both. | 013 |
| In-memory id hints, re-checked before use | A notes save no longer scans every folder; nothing is cached on disk. | 013 |
| Hand-rolled route stack instead of a router | Four screens, no URL bar, no deep links. | 015 |
| CodeMirror in a shadow root | The app's style policy blocks injected styles; in a shadow root CodeMirror uses a constructed stylesheet, so the policy did not have to be loosened. | 017 |
| Transcript rendered plainly, no windowing | 2,000 lines scroll under 10 ms a frame; the CSS feature that would help needs a newer Safari than the minimum. | 018 |
| The UI applies the default-model rule | The backend contract leaves the default alone on install; one place decides for Settings and First run. | 019 |
| Plain threads in the live session, no async runtime | Easier to test deterministically; the transcriber blocks for seconds anyway. | 009 |
| A two-hour ceiling on queued audio, with drops reported | Protects memory if transcription stalls. This departs from "nothing is dropped" and is worth revisiting. | 009 |

## What is proven and what is not

| Area | Proven by | Not proven |
| --- | --- | --- |
| System audio capture | A real tap captured a 1 kHz tone at exactly 1000.0 Hz for 5 s. | Silence, device changes, excluding Mneme's own sound, anything longer. |
| Microphone capture | Unit tests on synthetic input. | Any real microphone. |
| Whisper engine | The tiny English model transcribed the test clip correctly on the GPU. | Larger models, real meeting audio, accuracy. |
| Model download | One real download through the downloader, checksum matched. | The other nine files. |
| Storage | 283 tests, including a killed child process and recovery. | A real forced quit; a power cut. |
| Live session | Fake sources, plus one end-to-end run with the real model and store. | Real audio sources. |
| UI | 360 tests and browser checks in Chromium against the mock. | The real app window (WebKit), the real backend. |
| The app | Compiles and bundles; command logic tested with fakes. | It has never been launched. |

## Where to start reading

- To understand the product: `mneme-scope.md`, then `prds/README.md`.
- To follow the code: `src/lib/api/types.ts` (the contract), then `src-tauri/src/` (who implements it), then `crates/mneme-pipeline/src/session.rs` (the live meeting).
- To see what a piece of work decided and verified: the Result section of its issue file.
