# 022: Menu bar icon

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 021 · Owns: `src-tauri/`

## Goal

Recording state visible from anywhere on the Mac, with start and stop one click away, because the Mneme window is usually behind the call.

## Scope

1. A menu bar (tray) icon present while the app runs. Two states, idle and recording, as template images so they adapt to light and dark menu bars. Placeholder artwork is fine; make the two states clearly different in shape, not only in color.
2. While recording, the elapsed time shows next to the icon and updates each second.
3. **Menu**: `Start Recording` or `Stop Recording` (whichever applies; Start is disabled with a hint when no model is installed), `Open Mneme`, `Quit Mneme`.
4. Start from the menu bar records without opening or focusing the window. Open Mneme shows and focuses the main window, on the live meeting if one is running.
5. Menu bar actions and in-window actions use the same start/stop functions from issue 021, so state never disagrees.
6. Closing the main window keeps the app running in the menu bar. If closing while recording, recording continues.
7. A start error from the menu bar (no model, mic denied) opens the window showing the error; it never fails silently.

## Acceptance criteria

- [x] Tests for the pure parts: menu model for each state (idle, recording, no model), title text formatting for the timer.
- [x] The app builds; `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] Icon changes on start and stop from the window, the menu, and after a crash recovery launch.
- [ ] Timer ticks in the menu bar; readable in light and dark menu bars.
- [ ] Start from the menu bar with the window closed records, and Open Mneme shows the live meeting.

## Result

**Built** (all in `src-tauri/`):

- `src/tray.rs`: the menu bar icon. Pure part: `Phase` (idle / recording / stopping, from the `RecordingState` the window gets), `menu_model` (the three menu lines for a phase and "is a model installed"), `title_text` (elapsed time in the window's `formatElapsed` format, only while recording), `icon_kind`, `toggle_action`, `menu_click_action`. Real part: `setup` builds the tray icon with Tauri's own `tray-icon` feature (no plugin), `recording_state_changed` / `refresh` redraw it, `toggle_recording` and the menu call `AppCore::start_meeting` / `stop_meeting` on the blocking pool, `show_main_window` shows, unminimizes and focuses the window.
- `src/host.rs`: every recording state sent to the window is also handed to the tray (same call, same state, about once a second while recording); a download that ends refreshes "is a model installed".
- `src/lib.rs`: tray set up after the app state; closing the main window hides it (`prevent_close` + `hide`); `ExitRequested` with no code is prevented; `core.shutdown()` now runs only on `RunEvent::Exit` (Quit Mneme calls `app.exit(0)`, Cmd+Q also ends there); a Dock icon click (`RunEvent::Reopen`) shows the window again.
- `icons/tray-idle.png` (five waveform bars) and `icons/tray-recording.png` (disc in a ring): 36x36 black-plus-alpha template images, drawn by `icons/make_tray_icons.py` (standard library only), embedded at compile time with `tauri::include_image!`.
- `Cargo.toml`: `tauri` feature `tray-icon`. No new crate beyond what that feature brings (`tray-icon`, `muda`, already in `Cargo.lock`); `tauri.conf.json` and the capability file are unchanged.

Behaviour decisions:

- Menu: `Start Recording` / `Start Recording (no model installed)` (disabled) / `Stop Recording` / `Stopping…` (disabled), a separator, `Open Mneme`, `Quit Mneme`.
- A start or stop error from the menu bar shows and focuses the window, then emits `recording-error` with `{ action: "start" | "stop", message }`.
- The tray never waits for the main thread from a core callback (it queues the redraw), because at quit the main thread waits for the recording to stop.
- A menu click does what its line said or nothing: if the meeting ended by itself while the menu was open, Stop does not start a new one.
- While a start or stop asked for from the menu bar is under way, further toggles are dropped.

**Commands and outcomes**

- `cargo test -p mneme`: 32 passed (12 new in `tray::tests`; the config tests in `lib.rs` are unchanged and pass).
- `cargo clippy -p mneme --all-targets -- -D warnings`: clean. `cargo fmt -p mneme` applied.
- `pnpm tauri build --debug`: built `target/debug/bundle/macos/Mneme.app`.
- The app was not launched. Nothing in this issue was seen on screen.

**Manual checks still open** (everything visual; none done)

- The three under "Manual checks" above.
- The icon appears at launch, both shapes are readable at menu bar size, and they tint correctly in light and dark menu bars.
- Start disabled with the hint when no model is installed; it becomes enabled after a download finishes, and disabled again after the last model is deleted (this relies on the pointer entering the icon before the menu opens).
- Closing the window (red button, Cmd+W) hides it and a recording continues; Open Mneme and the Dock icon bring it back.
- Quit Mneme and Cmd+Q while recording leave a complete meeting; Quit with the window hidden exits.
- A start error from the menu bar (microphone denied) opens the window.

**Known gaps**

- The window has no listener for `recording-error` yet (`src/` is outside this issue): on a menu bar start error the window comes to the front but shows no message until the UI subscribes to that event. The message is also written to stderr.
- Open Mneme shows the window on whatever screen it was on; going to the live meeting is left to the UI, which already follows `recording-state`.
- While a start asked for in the window is still loading the model, the core reports idle, so the menu still offers Start; choosing it then is refused with "A meeting is already being recorded." and opens the window.
- A model deleted in Settings is noticed by the tray only on the next pointer-enter or click on the icon, or the next state change. If the menu is stale, Start fails with the no-model error and opens the window.
- The real tray code (`setup`, `draw`, `toggle`, window hide and exit handling) has no automated test; only the pure functions do.
- If the tray icon cannot be created, the app logs it and runs without one; with the window then closed (hidden), only the Dock icon brings it back.

