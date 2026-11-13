# Known gaps

Noted during the first build session (2026-11-12) to deal with later. Each issue's own "Known gaps" or "Not verified" section has the detail; this is the short list. Manual test steps are in [release.md](release.md).

## Do first: things nobody has seen work

1. **The app has never been launched.** Open it and check: styles load under the content security policy, the editor (in a shadow root) is styled and typing, selection and copy/paste work, the top bar sits right under the title bar, the accent color follows the system, `<progress>` bars and radio buttons look right, Shift-Command-T toggles the transcript.
2. **No real call has been recorded.** Three issues of capture code (002, 003, 004), the live session (009) and the app's start/stop (021) rest on one 5-second tone capture. With headphones, on a call: run `mneme-record` first (steps in `crates/mneme-audio/README.md`), then a meeting in the app.
3. **The microphone path has never opened a real microphone.**
4. **System audio beyond the tone test:** capture through silence, excluding Mneme's own sound, following an output device change, anything longer than 5 seconds.
5. **Forced quit mid-meeting, then relaunch.** Recovery is tested with a killed child process only.

## Open decisions for Francesco

| Decision | Options | Where |
| --- | --- | --- |
| Speech dropped when two hours of audio are queued | Keep the ceiling (protects memory) or remove it (matches "nothing is dropped") | 009 |
| HTTP client `ureq` 3.5.2 was published the day it was adopted | Pin the previous release until it has aged, or keep | 007, 025 |
| Reading the system-audio permission | Keep "unknown" (public API only) or enable the private-API feature (rules out the App Store) | 003, 024 |
| Spellcheck and autocorrect in the editor | Off today (CodeMirror's default); a one-line change | 017 |
| Speaker bleed without headphones | Recommend headphones (today) or add echo cancellation | scope doc |
| Which models to list | All ten today; five are multilingual and v1 is English only | 007, 019 |
| First-run recommended model | `small` is a placeholder until the benchmark is done | 008, 020 |
| A meetings folder synced from another Mac | Recovery assumes one running copy per folder | 013 |
| Hiding a transcript's last line when its final newline is missing | Follows the crash-detection rule; one-place change to relax | 011 |

## Unfinished work

- **008 benchmark:** the `mneme-bench` tool is built and its tests pass, but no model has been benchmarked and there is no table. The agent was stopped before reporting, so its behavior is unreviewed. Models on disk: tiny.en, base, small, large-v3-turbo.
- **022 menu bar icon, 023 global shortcut, 024 permissions flow:** see `issues/README.md` for where each stood when the session ended.
- **Entitlements** are not set in the app config; `scripts/release.sh` refuses to run until they are.
- **docs/release.md** gathered its manual checks before 009 and 020–024 were finished; re-gather them.

## Weak spots by area

### Audio
- Chunker detects loudness, not speech: typing, coughs and music become chunks; quiet speech in a noisy room is missed. Defaults are untuned on real meetings.
- A silent `system.wav` with no errors cannot be told from "nothing was playing" or "permission lost".
- A permission prompt left unanswered for about a minute may break Core Audio in that process until restart (reported by Chromium's source, not reproduced).
- Long recordings: clock drift between device and system clocks is unmeasured.

### Transcription
- whisper-rs is slow-moving: one maintainer, last release 2026-03, ten whisper.cpp versions behind. It is pinned and isolated in one file.
- A `WhisperEngine` alive at process exit aborts the process; the app drops it on quit, but any new code path that keeps one alive will crash on exit.
- First load of a model takes about 7 s longer while Metal compiles shaders. The model is loaded per meeting.
- A transcription call that never returns hangs Stop. Failed chunks are not retried.
- No disk-space check before a model download.

### Storage
- Empty folders left by a crash during create are reported on every list, never deleted.
- Timings were measured on a loaded machine and warm cache only.
- Not opened in Obsidian; validity rests on two YAML parsers in tests.

### UI
- Every Delete in Settings is locked during any recording, because the app does not tell the UI which model is in use.
- A download row disappears if the window reloads mid-download, until the next progress event.
- "Mic only" (system audio denied) is not shown in the UI; it goes to the log.
- Quitting after the first-run download but before Finish skips the permissions step next time.
- Focus is not moved when the first-run step changes; no screen reader was used anywhere.
- Shortcut keys show US-layout labels.
- A failed background refresh on Home replaces the list with an error and Retry.
- JavaScript bundle is about 590 kB (190 kB gzipped), mostly CodeMirror.

### App
- Quit can block the main thread for up to about 10 s while a recording stops.
- The command layer (`commands.rs`, `host.rs`) has no tests of its own; the core logic under it does.
- `startedAt` carries the local offset, not `Z` as the mock does.

### Release
- Neither signing script has run for real. The config overlays were checked against the schema, never through a build.
- The local-only audit works on crate names; it cannot see a dependency using the standard library's networking directly.
- Release output is a zip, not a DMG.

## Process notes

- An agent left 36 busy-loop processes running for half an hour; timings reported by issues 011 and 012 were taken under that load.
- A root `.gitignore` rule (`models/`) briefly hid a source folder from git; it is now anchored to `/crates/*/models/`.
