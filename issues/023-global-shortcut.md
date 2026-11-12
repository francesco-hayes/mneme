# 023: Global shortcut

Status: todo · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 022 · Owns: `src-tauri/`

## Goal

One system-wide shortcut that toggles recording, off until the user sets it.

## Scope

1. No shortcut is registered by default. On launch, register the one saved in settings, if any.
2. `updateSettings` with a new `globalShortcut` unregisters the old one and registers the new one at once. Setting it to none unregisters.
3. If registration fails (invalid or taken by the system), return a readable error and keep the previous shortcut registered and saved.
4. Pressing the shortcut toggles: starts a meeting when idle (same path as the menu bar: no window is forced open), stops when recording. Ignored while stopping. Key repeat does not toggle twice.
5. The shortcut string format accepted by the backend matches what the Settings recorder (issue 019) produces. If they differ, convert in `src-tauri` and note it under Follow-ups.
6. A start error from the shortcut opens the window showing the error, as in issue 022.

## Acceptance criteria

- [ ] Tests for the pure parts: parsing and validating shortcut strings from the UI format; toggle decision for idle, recording and stopping; failed registration leaves the old setting.
- [ ] The app builds; `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] Set a shortcut in Settings; from another app it starts and stops recording.
- [ ] Clear it; the combination no longer does anything.
- [ ] Pick a combination macOS reserves; the error shows and the old one still works.

## Result

_Filled in by the implementer._
