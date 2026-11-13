# 023: Global shortcut

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 022 · Owns: `src-tauri/`

## Goal

One system-wide shortcut that toggles recording, off until the user sets it.

## Scope

1. No shortcut is registered by default. On launch, register the one saved in settings, if any.
2. `updateSettings` with a new `globalShortcut` unregisters the old one and registers the new one at once. Setting it to none unregisters.
3. If registration fails (invalid or taken by the system), return a readable error and keep the previous shortcut registered and saved.
4. Pressing the shortcut toggles: starts a meeting when idle (same path as the menu bar: no window is forced open), stops when recording. Ignored while stopping. Key repeat does not toggle twice.
5. The shortcut string format accepted by the backend matches what the Settings recorder (issue 019) produces. If they differ, convert in `src-tauri` and note it under Follow-ups.
6. A start error from the shortcut opens the window showing the error, as in issue 022.

## Notes from earlier issues

- From 019: the Settings recorder saves `[Control+][Alt+][Shift+][Command+]<KeyboardEvent.code>`, modifiers always in that order, for example `Shift+Command+KeyR`, `Control+Alt+Space`, `Alt+F5`; null means none. It was written to match the parser in the Tauri global-shortcut plugin (global-hotkey `parse_hotkey`), read but never run: confirm it parses. `updateSettings({ globalShortcut })` must reject with a readable sentence and keep the old shortcut on failure.

## Acceptance criteria

- [x] Tests for the pure parts: parsing and validating shortcut strings from the UI format; toggle decision for idle, recording and stopping; failed registration leaves the old setting.
- [x] The app builds; `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] Set a shortcut in Settings; from another app it starts and stops recording.
- [ ] Clear it; the combination no longer does anything.
- [ ] Pick a combination macOS reserves; the error shows and the old one still works.

## Result

Built in `src-tauri/` (2026-11-12). No shortcut was registered on this machine and the app was not launched: everything that needs a key press is under "Manual checks still open".

**What was built**

- Dependency: `tauri-plugin-global-shortcut = "2.4.1"` (official plugin, brings `global-hotkey` 0.8.0; no network use). Configured from Rust only: nothing added to `tauri.conf.json` or to `capabilities/default.json`, so the window cannot call the plugin's commands. The config tests in `src/lib.rs` are untouched and pass.
- `src/shortcut.rs` (new).
  - Pure: `parse` (the UI string, through the plugin's own parser, plus "at least one modifier"), `wanted` (patch value to keep / clear / set, trimmed like the settings store), `press` (only the first press toggles; repeat and release do not), `register_saved` (launch) and `change` (register new, then unregister old, then save; a failed registration stops there; a failed save or a failed unregister puts the old one back). The last two work through a small `Registrar` trait so they are tested with a fake.
  - Real: `setup` adds the plugin with a handler that calls `tray::toggle_recording` on the first press, then registers the saved shortcut; any failure is logged (`mneme: global shortcut: ...`) and the app carries on. `update_settings` wraps `AppCore::update_settings`.
- `src/commands.rs`: `update_settings` goes through `shortcut::update_settings`. `src/lib.rs`: `shortcut::setup` in the setup hook after the tray. `src/tray.rs`: `#[allow(dead_code)]` removed from `toggle_recording`.
- Format (scope 5): the Settings format parses as is with the plugin's parser (`Shortcut::from_str`, which is global-hotkey `parse_hotkey`). No conversion. Tested for `Shift+Command+KeyR`, `Control+Alt+Space`, `Alt+F5`, all four modifiers with `Digit1`, `Backquote`, `ArrowUp`, `NumpadAdd`.
- Toggle decision for idle / recording / stopping (scope 4) is `tray::toggle_action` from issue 022, already tested there (`tray.rs` tests); the shortcut uses that same path, so a start error opens the window as in 022 (scope 6).
- Errors the window gets: invalid string: `"<text>" is not a shortcut Mneme can use. Hold Control, Alt, Shift or Command and press one key.`; refused by the system: `That shortcut could not be registered. macOS or another app may already be using it. Try a different combination.`

**Commands and outcomes**

- `cargo test -p mneme`: 46 passed, 0 failed (14 new in `shortcut::tests`).
- `cargo clippy -p mneme --all-targets -- -D warnings`: clean. `cargo fmt -p mneme -- --check`: clean.
- `pnpm tauri build --debug`: built `target/debug/bundle/macos/Mneme.app`. Not launched.

**Manual checks still open**

- All three under "Manual checks" above (set and use from another app; clear; reserved combination).
- Launch with a shortcut already saved: it works without opening Settings.
- Hold the shortcut down: recording toggles once, not repeatedly.
- Press it during "stopping": nothing happens. Press it with no model installed: the window opens with the error.

**Known gaps**

- Nothing in the real path (`setup`, `update_settings`, the handler, the plugin itself) has run; only the pure functions and the fake registrar are tested. That the plugin can register from the setup hook rests on its docs and on reading tauri-runtime-wry (a main-thread task runs inline when called on the main thread).
- "Taken by the system" may not produce an error on macOS: the plugin uses Carbon `RegisterEventHotKey`, which, as far as I know, accepts many combinations macOS or other apps already use (the system then simply wins, or both fire). The third manual check may therefore show no error. The error path fires for whatever the system does refuse, and for a combination this app already holds.
- The repeat guard relies on a release event after each press. It is reset on every shortcut change; if macOS ever dropped a release otherwise, the next press would be ignored once.
- The backend requires at least one modifier and otherwise accepts anything the plugin parses (including spellings the recorder never produces, such as `Cmd+R`); the string is saved as given. If the recorder allows a bare key (for example `F5` alone), the backend refuses it with the sentence above.
- If the saved shortcut cannot be registered at launch it is only logged; Settings still shows it. Picking it again in Settings retries.

## Follow-ups

- None needed for the format: no conversion in `src-tauri`.
