# Known gaps

Noted during the first build session (2026-11-12) to deal with later. Each issue's own "Known gaps" or "Not verified" section has the detail; this is the short list. Manual test steps are in [release.md](release.md).

## First real use (reported by Francesco, 2026-11-18)

Francesco launched the app, ran a meeting, and reported that it worked and felt smooth. So the app opens, first run completes, a recording starts and stops, and a meeting is saved. He chose to move forward with the two checks below still open.

What that report does not tell us, because the details were not recorded: whether the meeting was a real call with another person's audio, whether both Me and Them lines appeared and were labeled correctly, whether it was launched with `pnpm tauri dev` or the built app, and which of the visual points (editor styling, transcript pane, menu bar icon, shortcut) were looked at.

## Still open, to check when convenient

1. **Forced quit mid-meeting, then relaunch.** The meeting should come back marked as interrupted with its transcript. Recovery is tested with a killed child process only. Not yet checked (noted 2026-11-18).
2. **The error message for a failed start from the menu bar or the shortcut.** Fixed in issue 027 (2026-11-18): the window now shows "Could not start recording: …" on any screen. Not yet seen in the real app. To check: turn off Mneme's microphone access in System Settings, choose Start Recording from the menu bar icon, and the window should open with the reason. Turn access back on afterwards.
3. **A real call with headphones,** if the first meeting was not one: only Me in the mic lines, only the other side in the Them lines.
4. **System audio edge cases:** capture through silence, excluding Mneme's own sound, following an output device change mid-meeting, a long meeting.
5. **The visual checklist,** for anything not already looked at: the editor (in a shadow root) styled and typing, selection and copy/paste, the top bar under the title bar, the accent color, `<progress>` bars and radio buttons, Shift-Command-T toggling the transcript, the menu bar icon and timer.

## Open decisions for Francesco

| Decision                                                          | Options                                                                                      | Where     |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | --------- |
| Speech dropped when two hours of audio are queued                 | Keep the ceiling (protects memory) or remove it (matches "nothing is dropped")               | 009       |
| HTTP client `ureq` 3.5.2 was published the day it was adopted     | Pin the previous release until it has aged, or keep                                          | 007, 025  |
| Reading the system-audio permission                               | Keep "unknown" (public API only) or enable the private-API feature (rules out the App Store) | 003, 024  |
| Speaker bleed without headphones                                  | Recommend headphones (today) or add echo cancellation                                        | scope doc |
| First-run recommended model                                       | `small.en` stays while the benchmark is on hold; revisit if it feels slow or inaccurate      | 008, 020  |
| A meetings folder synced from another Mac                         | Recovery assumes one running copy per folder                                                 | 013       |
| Hiding a transcript's last line when its final newline is missing | Follows the crash-detection rule; one-place change to relax                                  | 011       |

## Fixed on 2026-11-24 (issue 028, not yet seen in the real app)

- Spellcheck is a setting (Settings, Notes, Check spelling), off by default. Autocorrect stays off.
- A meeting that records the microphone only says so above the notes.
- Open Mneme in the menu bar goes to the live meeting.
- Only the model in use is locked from Delete during a recording.
- The menu reads "Starting…", disabled, while a meeting loads its model.

## Unfinished work

- **008 benchmark (on hold by choice, 2026-11-24):** Francesco decided benchmarking is not needed for now. The `mneme-bench` tool is built and its tests pass, but no model has been benchmarked and there is no table. The agent was stopped before reporting, so its behavior is unreviewed. Models in `crates/mneme-transcribe/models/`: tiny.en, base, small, large-v3-turbo. Base and Small are now retired multilingual models, so resuming would first need base.en and small.en downloaded.
- **Entitlements** are set (audio input only) but have never been embedded by a real signature; the debug build is ad-hoc signed.

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

- A download row disappears if the window reloads mid-download, until the next progress event.
- The mic-only warning (issue 028) shows on the live meeting only, not on Home or Settings during a meeting.
- Quitting after the first-run download but before Finish skips the permissions step next time.
- Focus is not moved when the first-run step changes; no screen reader was used anywhere.
- Shortcut keys show US-layout labels.
- A failed background refresh on Home replaces the list with an error and Retry.
- JavaScript bundle is about 590 kB (190 kB gzipped), mostly CodeMirror.

### App

- Quit can block the main thread for up to about 10 s while a recording stops.
- The command layer (`commands.rs`, `host.rs`) has no tests of its own; the core logic under it does.
- `startedAt` carries the local offset, not `Z` as the mock does.

### Menu bar, shortcut, permissions (022–024)

- None of the three has been seen or triggered: no tray icon on screen, no key press, no permission prompt.
- Clicking the Dock icon shows the window but does not go to the live meeting; the menu's Open Mneme does (028).
- While a start from the menu bar or shortcut loads its model, the window's New meeting is still enabled and is refused if pressed.
- A model deleted in Settings is noticed by the menu only on the next hover or click.
- macOS may not refuse a shortcut that is already in use, so a reserved combination may show no error. A saved shortcut that fails to register at launch is only logged.
- System audio reads "undetermined" until a recording settles it. A prompt refused at first run stays "undetermined" until a meeting is refused; if macOS does not block the start while its prompt is open, the first meeting may read "denied" early.
- The two System Settings links (`Privacy_Microphone`, `Privacy_AudioCapture`) were never opened.

### Release

- Neither signing script has run for real. The config overlays were checked against the schema, never through a build.
- The local-only audit works on crate names; it cannot see a dependency using the standard library's networking directly.
- Release output is a zip, not a DMG.

## Process notes

- An agent left 36 busy-loop processes running for half an hour; timings reported by issues 011 and 012 were taken under that load.
- A root `.gitignore` rule (`models/`) briefly hid a source folder from git; it is now anchored to `/crates/*/models/`.
