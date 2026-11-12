# 022: Menu bar icon

Status: todo · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 021 · Owns: `src-tauri/`

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

- [ ] Tests for the pure parts: menu model for each state (idle, recording, no model), title text formatting for the timer.
- [ ] The app builds; `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] Icon changes on start and stop from the window, the menu, and after a crash recovery launch.
- [ ] Timer ticks in the menu bar; readable in light and dark menu bars.
- [ ] Start from the menu bar with the window closed records, and Open Mneme shows the live meeting.

## Result

_Filled in by the implementer._
