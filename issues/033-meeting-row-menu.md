# 033: A ⋯ menu on each meeting, with View and Delete

Status: done · PRD: [05](../prds/05-ui.md), [06](../prds/06-integration-and-finish.md) · Lane: several (ui, app) · Depends on: 031 · Owns: `src/components/MenuButton.*`, `src/components/ConfirmDialog.*`, the meeting rows in `src/screens/HomeScreen.*`, `deleteMeeting` in `src/lib/api/*` and the PRD 05 contract block, `delete_meeting` and `Host::trash` in `src-tauri/`

## Goal

Each meeting on Home ends in a ⋯ button. It opens a menu with View, which opens the meeting, and Delete…, which asks in a dialog and then moves the meeting to the Trash.

## Contract change

In `src/lib/api/types.ts` and the PRD 05 block, which stay identical:

- `deleteMeeting(id: string): Promise<void>`: moves the meeting's folder, with its notes and transcript, to the Trash, where Finder's Put Back restores it. Refused with a sentence while that meeting is being recorded or stopped.

The Trash was chosen over a permanent delete: the folders are the user's only copy, and a mistake can be undone in Finder.

## Acceptance criteria

- [x] Every row has a ⋯ button (`Options for <title>`) that opens a menu with View and Delete…. Arrow keys, Home and End move through it, Escape and Tab close it, so does a click elsewhere; the list's own arrow keys do not fire while it is open.
- [x] View opens the meeting, as a click on the row does.
- [x] Delete… opens a modal dialog, "Move “<title>” to the Trash?", with Cancel (focused) and Move to Trash. Tab stays inside it, Escape and Cancel close it and put focus back on the ⋯ button, and the window behind is `inert` while it is open.
- [x] Move to Trash calls `deleteMeeting`, closes the dialog and reads the list again. A failure is shown in the dialog, which stays open so it can be tried again.
- [x] Delete… is disabled, with a tooltip, on a meeting being recorded; the backend refuses it too.
- [x] Backend: `AppCore::delete_meeting` checks the live meeting under the recording lock, finds the folder by id, and calls `Host::trash`. `TauriHost` uses `NSFileManager.trashItemAtURL` (objc2-foundation, already in the tree).
- [x] `cargo test -p mneme`, clippy, fmt, `pnpm test`, `pnpm lint` and `pnpm build` pass.

## Manual checks

- [ ] In the real app, delete a finished meeting: it is gone from Home and its folder is in the Trash, and Put Back restores it (it shows again on the next list).
- [ ] During a recording, the live meeting's Delete… is disabled.

## Result

- Rust: new test `a_meeting_is_deleted_to_the_trash_but_not_while_it_records`. The test host's `trash` removes the folder.
- UI: new `MenuButton`, `ConfirmDialog` and `MoreIcon` in `src/components`. `ConfirmDialog` is a portal with its own focus handling rather than `<dialog>`, because jsdom has no `showModal`. Home rows are a `MeetingRow` with the menu and the dialog. Nine new Home tests, one new mock test; the Tauri command table test counts `delete_meeting`.
- Mock: `deleteMeeting` drops the meeting, and refuses the live one.
- Seen in the browser against the mock: menu, dialog (Cancel focused, window inert), delete. Not seen in the real app.

## Known gaps

- After a delete, focus is left on the page rather than moved to the next row.
- A notes save still in flight for a meeting that was just deleted fails with "not found". Home is only reachable after leaving the meeting, so this needs a very fast delete.
- Built in the same checkout as another session's microphone picker, which also numbered its work 032; the shared files (`app.rs`, `tests.rs`, `types.ts`, `mock.ts`, `index.test.ts`, the PRD block) hold both changes.
