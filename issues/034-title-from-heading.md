# 034: The meeting title follows the first heading in the notes

Status: done · PRD: [04](../prds/04-storage.md), [05](../prds/05-ui.md) · Lane: several (store, app, ui) · Depends on: 033 · Owns: `heading_title` and the title rule in `crates/mneme-store/src/notes.rs`, `AppCore::save_notes` and the `save_notes` command, `saveNotes` in `src/lib/api/*` and the PRD 05 contract block, `onTitleSaved` in `src/screens/meeting/notesAutosave.ts`, `TitleField`

## Goal

Typing `# Standup` at the top of the notes names the meeting "Standup", without going to the title field.

## The rule

On every notes save, the store takes the title from the notes' first level-one `# ` heading while the title still follows the notes:

- the title was never edited (it is still the date-time default), or
- the title is the same as the first heading of the body on disk before this save.

So editing the heading renames the meeting again. A title typed in the title field that differs from the heading stops it for good. Notes without a heading leave the title as it is. A title taken this way is saved as edited (`title_edited: true`), so the folder is renamed to match, exactly as a typed title is: at once after the meeting, at Stop while it records. A rename that fails (the folder name is taken) does not fail the save; the title is kept and the error is logged.

No new frontmatter key: "follows" is worked out from `title_edited`, the title and the old body.

Which line counts (`heading_title`): CommonMark ATX level-one headings, up to three spaces of indent, a space or tab after `#`, an optional closing run of `#`, lines in ``` or ~~~ fences skipped, empty headings skipped. Setext headings are not read.

## Contract change

In `src/lib/api/types.ts` and the PRD 05 block:

- `saveNotes(id, notes): Promise<MeetingSummary>` (was `Promise<void>`): the meeting after the save, so the window can show a title the save gave it.

## Acceptance criteria

- [x] An untitled meeting takes its first `# ` heading as its title on the next save; the folder follows after Stop, or at once for a finished meeting.
- [x] Editing that heading renames it again; a title typed in the field is never replaced by the heading.
- [x] The meeting view's title field shows the new title after the save, unless the field is being edited or a rename from it is still on its way.
- [x] A taken folder name leaves the notes and the title saved.
- [x] `cargo test -p mneme-store -p mneme`, clippy, fmt, `pnpm test`, `pnpm lint` and `pnpm build` pass.

## Manual checks

- [ ] In the real app, type `# Something` at the top of a new meeting's notes: the title changes after a short pause, and after Stop the folder in Finder is `YYYY-MM-DD HHMM Something`.
- [ ] Type a title in the field, then change the heading: the typed title stays.

## Result

- Store: `heading_title` (public), the rule in `stage_save`, `SavedNotes` gains `meeting`, `title_changed` and `rename_error`; `Store::save_notes` now returns `SavedNotes`. `apply_pending_rename` is `pub(crate)`. Seven new tests in `tests/notes.rs`. Two older tests used `# Agenda` / `# Standup` bodies to check that a save leaves the frontmatter or folder alone; they now use `##`, and the listing test that saves `# Agenda` asserts the new title and uses the renamed folder.
- App: `save_notes` returns `dto::MeetingSummary` and logs a failed folder rename. The finder test asserts the title from `# Agenda`.
- UI: `notesAutosave` publishes each saved title (`onTitleSaved`); `TitleField` takes it when it is not being edited. The mock has the same rule (`headingTitle`), with its own tests. Three new meeting-view tests. The `deferred()` save in `MeetingScreen.test.tsx` now resolves with the meeting.
- Not tried in the real app by Claude. The running `tauri dev` had picked up the change when this was written.

## Known gaps

- The title changes after the save (a short pause in typing), not on every key.
- Once a title is typed by hand there is no way back to following the heading, short of typing the heading's text as the title.
- Existing meetings whose notes already start with a heading take it as their title the next time their notes are saved, not before.
- While a meeting records, the title changes at once but the folder only at Stop, as with a typed title.
