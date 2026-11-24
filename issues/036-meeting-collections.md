# 036: Folders of meetings

Status: done · PRD: [04](../prds/04-storage.md), [05](../prds/05-ui.md) · Lane: several (store, app, ui) · Depends on: 035 · Owns: collections in `crates/mneme-store/` (scan, create in, move, create/rename collection), the collection commands in `src-tauri/`, the contract additions in `src/lib/api/*` and the PRD 05 block, folders on Home

## Goal

Meetings that belong together (a client, a project, a hiring loop) live in one folder, so they are easy to find. A meeting can be started in a folder or moved into one later, as in Granola.

## Words

The app says **folder**. The code says **collection**, because "folder" already means a meeting's own folder on disk everywhere in the store (`MeetingSummary::folder`, `folderPath`). A collection is a folder of meeting folders.

## The rule on disk

A collection is a real folder in the meetings root, one level deep:

```
~/Documents/Mneme/
  2026-11-24 0930 Standup/          unfiled meeting
  Acme/                             a collection
    2026-11-20 1400 Kickoff/
    2026-11-24 1430 Pricing call/
  Hiring/                           an empty collection
```

So Finder, Obsidian and backups see the same grouping the app does, and nothing besides the folders holds it. A collection made or renamed in Finder shows up on the next list.

How a scan reads the root's direct subfolders:

| Entry | Is |
| --- | --- |
| Folder with a `notes.md` | a meeting, unfiled |
| Hidden folder (`.obsidian`) | skipped, as before |
| Folder with no `notes.md` whose name starts `YYYY-MM-DD HHMM ` | a meeting folder a crash left without its `notes.md`: a `NoNotesFile` warning, as before |
| Any other folder | a collection; its direct subfolders are read as meetings |

Inside a collection, a folder with a `notes.md` is a meeting; a folder without one is a `NoNotesFile` warning (no nesting), and files and hidden entries are skipped without a warning (a collection may hold an agenda PDF).

Collection names follow the folder name rules for titles (`sanitize_title`: no separators, no leading dot, capped length). A name that would read as a meeting folder (starts with `YYYY-MM-DD HHMM `) or is empty is refused, as is one that is taken.

Moving a meeting renames its folder into the other parent, keeping the folder name. It is refused while the meeting records (files in it are being appended to) and when the target already has a folder with that name. Renaming a collection is refused while a meeting in it records. Deleting a collection moves it, with its meetings, to the Trash, refused while a meeting in it records.

A meeting's identity stays the `id` in its frontmatter, so a move breaks nothing that refers to it.

## Contract change

In `src/lib/api/types.ts` and the PRD 05 block:

- `MeetingSummary.collection: string | null`: the collection the meeting is in, null when unfiled.
- `startMeeting(collection?: string | null)`: the new meeting is created in that collection. The menu bar and the shortcut start unfiled meetings.
- `listCollections(): Promise<string[]>`: every collection, sorted by name (case-insensitive), empty ones included.
- `createCollection(name): Promise<string>`: returns the name as stored.
- `renameCollection(name, newName): Promise<string>`.
- `deleteCollection(name): Promise<void>`: to the Trash with its meetings.
- `moveMeeting(id, collection: string | null): Promise<MeetingSummary>`: null moves it out to the root.

## UI

- Home has a folder bar above the list: All, Unfiled when there are folders, each folder, and New folder…. The choice is remembered while the app runs.
- In a folder, New meeting starts the meeting there, and the folder has its own ⋯ menu: Rename…, Delete….
- Each row's ⋯ menu gets Move to…, a dialog listing No folder and every folder; disabled while the meeting records. In All, a row in a folder shows the folder name in its detail line.

## Acceptance criteria

- [x] Store tests: a collection's meetings are listed, found by id and recovered; an empty collection is listed; a date-named folder without `notes.md` stays a warning; create in a collection; move in, between and out; move refused while recording and onto a taken name; create and rename collection refuse empty, date-like and taken names; rename refused while a meeting in it records; meetings keep their ids across all of it.
- [x] App: `start_meeting` takes a collection, `move_meeting`, the collection commands, `MeetingSummary.collection` in the DTO; delete refuses a collection holding the live meeting.
- [x] Mock implements the same rules; Home tests cover the folder bar, start in a folder, Move to…, rename and delete.
- [x] `cargo test -p mneme-store -p mneme`, clippy, fmt, `pnpm test`, `pnpm lint` and `pnpm build` pass.

## Manual checks

- [x] In the real app: make a folder, start a meeting in it, stop; in Finder the meeting folder is inside the folder.
- [x] Move a finished meeting in and out of a folder; it keeps its notes and transcript, and Show in Finder finds it.
- [x] Make a folder in Finder: it shows on Home after the window gets focus. (Seen when Home was shown again; the focus refresh itself was not triggered.)

## Not in this issue

- Nested folders. The scan reads one level; nesting can come later without changing what is on disk.
- Drag and drop onto a folder.
- Choosing a folder from the menu bar or the shortcut.
- Search or Q&A scoped to a folder (later, with the index).

## Result

Built 2026-11-24 on branch `feature/036-meeting-folders`. Not committed.

### What was built

- **Store** (`crates/mneme-store/`): the scan in `list.rs` reads collections one level down (`Found`, `scan_collection`); `MeetingList::collections`. New `collection.rs`: `list_collections`, `create_collection`, `rename_collection`, `collection_folder`, `collection_is_recording`, `move_meeting`. `Store::create_meeting_in(new, collection)`; `create_meeting` calls it with `None`. `MeetingSummary::collection(root)`. `naming::looks_like_meeting_folder`. New errors `Recording`, `CollectionNotFound`, `CollectionRecording`, `InvalidCollectionName`. A collection name from the window must be exactly one existing, non-hidden folder without `notes.md` in the root, so it can never reach outside it.
- **App** (`src-tauri/`): `AppCore::start_meeting_in`, `move_meeting`, `list_collections`, `create_collection`, `rename_collection`, `delete_collection` (to the Trash). Move, rename and delete check the live meeting under the recording lock. `dto::MeetingSummary.collection`. Five new commands; `start_meeting` takes an optional `collection`. The menu bar and the shortcut still call `start_meeting()` and start unfiled.
- **UI**: contract additions in `types.ts` and the PRD 05 block, `tauri.ts`, the mock (plus a `?mock=folders` scenario). Home: `FolderBar`, the folder ⋯ menu (Rename…, Delete…), Move to… in the row menu, the folder name in All's detail line, an empty state per folder. Dialogs in `src/screens/home/folders.tsx`; the remembered view in `folderView.ts`. `ConfirmDialog` takes `children` (fields) and `confirmDisabled`, and focuses the first field when it has one.

### Behavior change

A root folder with no `notes.md` used to be a `NoNotesFile` warning. Now it is a collection, unless its name starts `YYYY-MM-DD HHMM ` (a crash leftover, still a warning). The listing test's `Attachments/` case was updated to match.

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` | All pass, 14 new in `tests/collections.rs` plus a naming unit test. |
| `cargo test -p mneme` | 70 pass, 1 new (`meetings_start_in_and_move_between_collections`). |
| `cargo test -p mneme-pipeline` | All pass. |
| `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` | Clean. |
| `pnpm test` | 425 pass (new: 4 mock, 7 Home folder tests, command table). |
| `pnpm lint`, `pnpm build` | Clean. |

Seen in the browser preview against the mock (`?mock=folders`): the folder bar, a folder view with its ⋯ menu, and the Move to… dialog. No console errors.

### Real app (2026-11-24)

Debug bundle (`pnpm tauri build --debug`), with `meetingsRoot` pointed at a throwaway `~/Documents/MnemeTest036` and restored afterwards; the user's own meetings were not touched.

- On an empty root, New folder… made `Acme/`; New meeting in Acme recorded 15 s with `small.en` and the folder was `Acme/2026-11-24 1454 Meeting/`, `status: complete`.
- Move to… No folder moved it to the root and back into Acme; the 2-line transcript went with it, All showed "· Acme", and Show in Finder opened `Acme/` with the meeting selected.
- A folder made outside the app (`mkdir "Hiring loop"`) was in the folder bar when Home was shown again.
- Found and fixed during this check: with no meetings and no folders the folder bar was hidden, so a new user could not make a folder before a first meeting. The bar now always shows, a folder view with nothing in it gets its own empty state, and a Home test covers it.

### Not verified

- Delete… on a folder in the real app (`NSFileManager` Trash with meetings inside): only the test host's `trash` ran.
- Rename… on a folder in the real app.
- The refresh on window focus after a Finder change (seen on re-showing Home instead).

### Follow-ups

- Choosing a folder from the menu bar or the shortcut (both start unfiled).
- A meeting view does not show or change its folder; only Home does.
