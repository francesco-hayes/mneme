# 010: Meeting folders, frontmatter, naming

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 001 · Owns: `crates/mneme-store/`

## Goal

Create a meeting as a folder with a `notes.md` whose frontmatter holds its identity, and rename it safely.

## Scope

1. **`Store`** opened on a root folder (created if missing).
2. **Create meeting**: given a start time, makes `YYYY-MM-DD HHMM Meeting/` (local time) with `notes.md` (frontmatter, empty body), an empty `transcript.md` and an empty `notes-timeline.jsonl`. Frontmatter keys as in PRD 04: `id`, `title` (default display title like `Meeting, Oct 27 2:30 PM`), `title_edited: false`, `date` with UTC offset, `duration` empty, `status: recording`, `model`, `language`.
3. **Frontmatter read/write**: parse and rewrite without touching the body; keep unknown keys and their order where practical; tolerate a file with no frontmatter or broken frontmatter by returning an error for that meeting only.
4. **Safe write helper** used by everything in the crate: write to a temp file in the same folder, sync, rename over the target.
5. **Folder name rules**: sanitize the title part (remove path separators, control characters, leading dots, trailing spaces and dots; collapse whitespace; cap the length); empty result becomes `Meeting`.
6. **Rename**: sets `title` and `title_edited: true`, then renames the folder to match. Refused while `status` is `recording`: the title is saved to frontmatter and the folder rename is applied by a `finish` call at stop. If the target folder exists, return an error and change nothing.
7. **Finish**: sets `duration` and `status: complete`, applies any pending folder rename.

## Acceptance criteria

- [x] Tests in temp folders: create; frontmatter round-trip with unknown keys and a body containing `---` lines; sanitizing nasty titles; rename after completion; rename during recording is deferred and applied at finish; rename onto an existing folder fails cleanly; the id never changes across renames.
- [x] Created files open as plain Markdown with valid YAML frontmatter.
- [x] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Implemented 2026-11-12. All work is in `crates/mneme-store/`; nothing outside it was edited except this file (cargo updated `Cargo.lock` for the new dependencies).

### What was built

Modules: `store` (the `Store`), `meeting` (types and frontmatter keys), `frontmatter` (reader/writer for `notes.md`), `naming` (folder names), `time` (`StartTime`), `safe_write`, `error`. Issues 011–014 add their own modules beside these.

Public API (`mneme_store::…`):

```rust
// Store: one per root, shared by reference; Send + Sync. An internal lock
// serializes every read-change-write of a notes.md and every folder rename.
Store::open(root: impl Into<PathBuf>) -> Result<Store>          // creates the root if missing
Store::root(&self) -> &Path
Store::create_meeting(&self, new: NewMeeting) -> Result<MeetingSummary>
Store::meeting(&self, id: &MeetingId) -> Result<MeetingSummary>  // scans the root for the id
Store::rename_meeting(&self, id: &MeetingId, title: &str) -> Result<MeetingSummary>
Store::finish_meeting(&self, id: &MeetingId, duration_secs: u64) -> Result<FinishedMeeting>

struct NewMeeting { started_at: StartTime, model: Option<String>, language: Option<String> }
struct FinishedMeeting { meeting: MeetingSummary, rename_error: Option<Error> }
struct MeetingSummary {
    id: MeetingId, title: String, title_edited: bool, started_at: StartTime,
    duration_secs: Option<u64>, status: MeetingStatus,
    model: Option<String>, language: Option<String>, folder: PathBuf,
}
MeetingSummary::{notes_path, transcript_path, timeline_path}(&self) -> PathBuf
MeetingSummary::expected_folder_name(&self) -> String

struct MeetingId          // MeetingId::new(impl Into<String>), as_str(), Display, From<String>, From<&str>
enum MeetingStatus { Recording, Complete, Interrupted }   // as_str(), parse(&str) -> Option, Display
struct StartTime          // now_local(), parse_rfc3339(&str), FromStr, to_rfc3339(), Display,
                          // unix_seconds(), utc_offset_seconds(); Ord and Eq compare the instant

safe_write(path: &Path, contents: &[u8]) -> io::Result<()>
is_temp_file_name(name: &str) -> bool
sanitize_title(title: &str) -> String
const NOTES_FILE, TRANSCRIPT_FILE, TIMELINE_FILE, DEFAULT_FOLDER_TITLE

enum Error { Io { path, source }, Frontmatter { path, source: FrontmatterError },
             MeetingNotFound { id }, FolderExists { path }, EmptyTitle, NotRecording { id, status } }
enum FrontmatterError { Missing, Unterminated, InvalidLine { line }, DuplicateKey { key },
                        MissingKey { key }, InvalidValue { key, reason } }
```

Crate-internal pieces the next store issues build on: `frontmatter::NotesFile` (`parse`, `render`, `get`, `set`, `set_string`, public-in-crate `body` field), `safe_write::PendingWrite` (`stage` then `commit`; stopping between the two is the "crash between temp write and rename" case for 012), `Store::lock`, `store::load` / `OpenNotes::save`, and `Store::close(open, duration_secs, status)`, which recovery in 013 can call with `Interrupted`.

A new meeting's `notes.md`, byte for byte (asserted by `create_writes_this_exact_notes_file`):

```
---
id: be5b4a63-9ed1-4efe-b438-373d7c79b1aa
title: Meeting, Oct 27 2:30 PM
title_edited: false
date: "2026-10-27T14:30:00+01:00"
duration:
status: recording
model: large-v3-turbo
language: auto
---
```

Behavior worth knowing:

- **Time.** The crate reads the clock and time zone in one place, `StartTime::now_local()`. `create_meeting` takes the start time as a value, so the clock and zone are injected by the caller. The folder name and default title come from the wall clock inside that value; later renames use the offset saved in `date`, not the machine's zone.
- **Ids** are random version 4 UUIDs, written once at creation. No later call writes the `id` key.
- **Rename.** The display title is tidied to one line and kept as typed in frontmatter; the folder gets the sanitized form. While recording, only frontmatter changes and `finish_meeting` moves the folder. The target name is checked before anything is written, during recording too, so a clash is reported when the title is typed. If the folder rename itself fails after the frontmatter was saved, the old frontmatter is written back.
- **Finish** saves `duration` and `status: complete` first. If the waiting folder rename then fails, the meeting is still complete and the failure is in `FinishedMeeting::rename_error`. Calling finish on a meeting that is not recording returns `NotRecording` and leaves the saved duration alone.
- **Hand-renamed folders.** Only an edited title moves a folder. If the title was never edited and the folder has another name, someone renamed it by hand and it is left alone.
- **Safe write** names its temp file `.<name>.<pid>-<n>.tmp` in the same folder, syncs the file (F_FULLFSYNC on macOS), renames, then syncs the folder. A crash can leave a hidden temp file behind; `is_temp_file_name` recognizes it.

### YAML library or own parser

A small purpose-built parser, on purpose. A YAML library reads the block into a value tree and writes a new block, which loses comments, key order or styling, and reformats lists that a person or Obsidian wrote; `serde_yaml` is also unmaintained. Mneme reads and writes eight one-line scalars, so `frontmatter.rs` works on lines: each top-level `key:` line plus the lines under it is one entry kept as raw text, setting a key rewrites that one line, and everything else is written back byte for byte. A file parsed and rendered without a change is identical, including CRLF line endings and a byte order mark. Only the first closing `---` ends the block, so `---` lines in the body are plain text.

Limits of that choice: the keys Mneme reads must be one-line scalars (plain, single-quoted or double-quoted). A block scalar, list or map under one of those keys is an error for that meeting. Unknown keys can be any YAML. What the crate writes is checked against real YAML parsers (below). Strings are written plain only when they start with a letter and cannot be mistaken for another type; otherwise they are double-quoted (`language: "no"`, `title: "Q4: planning"`). `date` is always quoted, because unquoted it is a timestamp to YAML 1.1 tools, which may write it back in UTC and lose the offset the folder name depends on.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` | 90 passed, 0 failed, 0 ignored (55 unit, 35 in `tests/meetings.rs`). Run 4 times, same result. |
| `TZ=<zone> cargo test -p mneme-store` for UTC, Asia/Kolkata, Asia/Kathmandu, America/Los_Angeles, Pacific/Kiritimati, Australia/Lord_Howe | 90 passed each time. |
| `cargo clippy -p mneme-store --all-targets -- -D warnings` | Exit 0, no clippy or rustc warnings. Cargo prints one manifest warning, see below. |
| `cargo fmt -p mneme-store -- --check` | Exit 0. |
| `cargo doc -p mneme-store --no-deps` | No rustdoc warnings. |

Acceptance criteria:

1. Tests in temp folders, all passing. Create: `create_makes_the_folder_and_its_three_files`, `create_writes_this_exact_notes_file`, `create_returns_what_a_later_read_returns`. Round-trip with unknown keys and `---` in the body: `rewrites_keep_unknown_keys_their_order_and_the_body`, `parse_then_render_is_byte_identical`, `a_file_with_windows_line_endings_keeps_them`. Nasty titles: `nasty_titles_are_kept_in_frontmatter_and_made_safe_in_the_folder_name` plus the `naming` unit tests. Rename after completion: `rename_after_completion_updates_frontmatter_and_moves_the_folder`. Deferred rename: `rename_during_recording_is_saved_but_the_folder_waits_for_finish`. Existing target: `rename_onto_an_existing_folder_fails_and_changes_nothing` (compares every file under the root before and after). Stable id: `rename_can_be_repeated_and_the_id_never_changes`. Broken frontmatter affecting one meeting only: `broken_frontmatter_is_an_error_for_that_meeting_only`.
2. Valid YAML, checked with two independent parsers. In the test suite, `saphyr` 0.3.4 (YAML 1.2, dev-dependency) parses the frontmatter of created and renamed meetings and of 40 awkward strings, and gets the exact strings and types back. By hand, a throwaway program outside the repo created eight meetings through the public API (one with `StartTime::now_local()`, titles including `no`, `2026-10-27` and `Q4: planning #2 "draft"`); the system Ruby (Psych 3.1.0, libyaml 0.2.1, YAML 1.1) parsed every `notes.md` with `id`, `title`, `date`, `status` as strings, `title_edited` as a boolean and `duration` as nil or an integer. `file` reports the one with an emoji title as UTF-8 text. The throwaway program and its output were deleted.
3. Test and clippy commands as in the table.

### Not verified

- Opening the files in Obsidian. Not run here; validity rests on the two YAML parsers above.
- Durability across a real power cut or forced quit. The helper follows the temp, sync, rename order and a test stops between the last two steps, but no machine was crashed.
- The leftover-frontmatter restore when a folder rename fails is tested through a read-only root; other failure causes were not simulated.
- Behavior on a case-sensitive volume. The case-only rename test passed on this machine's default volume; it is written to pass on both kinds.

### Deviations from the issue and PRD

- **Sanitizing replaces rather than deletes.** Path separators and the other reserved characters (`/ \ : * ? " < > |`) become a word break instead of vanishing, so `Q3/Q4 review` gives `Q3 Q4 review`, not `Q3Q4 review`. The characters are still gone from the name. The title part is capped at 80 characters and 200 bytes. Text-direction override characters are removed as well.
- **Rename during recording can be refused.** The issue says the title is saved and the folder renamed at finish. That is what happens, except that a title whose folder name is already taken is refused straight away with `FolderExists`, per the PRD rule that such a rename "fails with a clear error and nothing changes".
- **Empty title.** Not specified. `rename_meeting` returns `EmptyTitle` and changes nothing.
- **`finish_meeting` returns `FinishedMeeting`**, not a bare summary, so a blocked folder rename cannot make a stop look like it failed.
- **`date` is quoted** in frontmatter (reason above). It is still "start time with UTC offset".
- **Lookup by id** (`Store::meeting`) is here already, because rename and finish take an id. It is a plain scan; 013 adds listing, duplicate-id reporting and warnings.

### Follow-ups

- **Two meetings in one minute.** Start, stop and start again within the same minute asks for the same folder name, and `create_meeting` returns `FolderExists`. The decisions log assumes this cannot happen. Needs a product decision (a suffix, or a wait) before 021 wires up start; until then 021 should show the error.
- **Manifest warning.** Cargo prints `unused dependency mneme-core` for this crate. The scaffold declared the dependency before anything used it; it is not a clippy failure. Issue 011 uses `mneme-core` (segments, timestamps), which removes it.
- **`issues/README.md`** still lists 010 as `in progress`; the table is outside this lane.
- **For 012:** take the store lock for every notes save (`Store::lock`, then `load`, change `doc.body`, `OpenNotes::save`), otherwise a save can race a rename and lose one of the two changes.
- **For 013:** `Store::find_folder` picks the first folder by name when an id is duplicated; replace with reporting. Leftover `.notes.md.*.tmp` files can be cleaned during recovery with `is_temp_file_name`.
- **For 021:** `MeetingSummary` maps to the frontend type as `id.as_str()`, `title`, `started_at.to_rfc3339()` for `startedAt`, `duration_secs`, `status.as_str()`, and `folder` for `folderPath`. The types do not derive `serde`; add that here if 021 prefers it to mapping by hand.
