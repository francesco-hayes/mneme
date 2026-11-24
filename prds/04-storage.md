# PRD 04: Storage

Covers build step 4, the "Storing meetings" section, the gap "Incremental saves", and settings persistence for MVP feature 5. Issues: 010–014, 026, 036.

## Problem

Meetings must be plain files the user owns, readable in any editor, and safe against a crash mid-meeting. The folders are the source of truth; nothing else may hold data that cannot be rebuilt from them.

## Goals

- One folder per meeting holding `notes.md`, `transcript.md` and `notes-timeline.jsonl`.
- A forced quit mid-meeting loses seconds, not the meeting.
- The Home list comes from scanning folders and reading frontmatter.
- App settings persist across restarts.

## Non-goals

- SQLite, full-text search, vectors. Later, as a rebuildable index.
- A `transcript.jsonl` sidecar with word timings.
- Stored audio.

## Requirements

### Layout

```
~/Documents/Mneme/
  2026-10-27 0930 Standup/
    notes.md               frontmatter + my notes
    transcript.md          [00:01:23] Them: ...
    notes-timeline.jsonl   {"t":"00:04:12","text":"- ask Ana about the API"}
  2026-10-27 1430 Meeting/  default title kept
  Acme/                     a collection (issue 036)
    2026-10-28 1000 Kickoff/
```

### Collections (issue 036)

- A collection is a folder of meeting folders directly in the root; the app calls it a folder. One level only.
- A root subfolder with a `notes.md` is a meeting. A hidden one is skipped. One with no `notes.md` whose name starts `YYYY-MM-DD HHMM ` is a meeting folder a crash left behind (a warning). Any other is a collection, and its direct subfolders are read as meetings.
- Collection names follow the title rules for folder names; empty, date-like and taken names are refused.
- Moving a meeting renames its folder into another parent, keeping its name. Refused while it records or onto a taken name. Renaming or deleting (to the Trash) a collection is refused while a meeting in it records.

### Folder names

- `YYYY-MM-DD HHMM Title`, from the local start time.
- A meeting that keeps its default title uses `Meeting` as the title part. The title shown in the app (for example "Meeting, Oct 27 2:30 PM") lives in frontmatter.
- Characters that are unsafe in file names are removed from the title part; an empty result falls back to `Meeting`.
- The folder is renamed when the title changes, but never while recording. A rename requested during recording is applied at stop.
- If the target name already exists, the rename fails with a clear error and nothing changes.
- A new meeting whose folder name is taken (a second meeting started in the same minute) gets a numeric suffix: `2026-10-27 1430 Meeting 2`. The lowest free number is used, up to 999; creating a meeting does not fail on a name clash short of that bound. The suffix is part of the folder name only, not the title.

### Frontmatter (`notes.md`)

| Key | Value |
| --- | --- |
| `id` | Stable unique id. Never changes; the folder name is not the id. |
| `title` | Display title. |
| `title_edited` | `false` until the user changes the title. Decides whether the folder uses `Meeting`. |
| `date` | Start time with UTC offset. |
| `duration` | Seconds; empty while recording. |
| `status` | `recording`, `complete` or `interrupted`. |
| `model`, `language` | What transcribed this meeting. |

Unknown keys are kept on rewrite, so hand edits and future versions survive.

### Transcript

- Strict lines: `[hh:mm:ss] Me: text` or `[hh:mm:ss] Them: text`. One line per segment; newlines inside text become spaces.
- Each line is appended and flushed as it arrives.
- At stop the file is sorted by timestamp (stable, so equal timestamps keep arrival order) through a safe write.
- A parser returns lines in order and skips malformed ones without failing.

### Notes

- Saves go through a temp file and rename in the same folder, so a crash never leaves a half-written `notes.md`.
- Saving the body never loses frontmatter, and updating frontmatter never touches the body.

### Note timeline

- While recording, each notes save appends one JSON line per changed or added line: `{"t":"hh:mm:ss","text":"..."}`, with meeting time on the transcript's clock.
- Append-only. It is a record of what was typed when, not a pointer into `notes.md`.
- The text is the trimmed line. A line that grows across autosaves is recorded once per saved state (`- ask`, `- ask Ana`, …), so entries can repeat a prefix; the earliest entry for a text is when it was first typed. Entries carry no link to the text they replaced (decided 2026-11-12).
- Nothing reads it in v1.

### Listing and recovery

- List meetings by scanning the root and its collections, newest first, returning id, title, date, duration and status. Folders without a readable `notes.md` are skipped. Fine for hundreds of meetings.
- Look up a meeting by id.
- On launch, any meeting still marked `recording` is set to `interrupted`, its transcript is sorted, and its duration is set from the last transcript timestamp.

### Settings

`settings.json` in the app data folder, saved through a safe write: default model id, language (always `en` in v1; nothing in the UI changes it), global shortcut (none by default), meetings root. Missing or corrupt file falls back to defaults without crashing.

## Done when

- Unit tests cover every rule above using temporary folders, including a simulated crash (process stops between appends) and recovery.
- Files written by Mneme open cleanly in a plain editor and in Obsidian.
- Manual: forced quit mid-meeting, relaunch, the meeting is listed as interrupted with its transcript up to the last seconds.
