# 012: Notes safe writes and note timeline

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 026 · Owns: `crates/mneme-store/`

## Goal

Save notes without ever leaving a half-written file, and record when each note line was typed.

## Scope

1. **Read notes**: return the body of `notes.md` without frontmatter.
2. **Save notes**: replace the body, keep frontmatter byte-for-byte, through the safe write helper.
3. **Note timeline**: `save_notes_at(body, meeting_time_ms)` used while recording. Compare the new body with the previously saved body line by line and append one JSON line to `notes-timeline.jsonl` for each line that is new or changed: `{"t":"hh:mm:ss","text":"<line>"}`.
   - Blank lines and lines that only moved are not recorded.
   - Deleted lines are not recorded.
   - Append-only and flushed on each save.
   - The first save diffs against the body on disk, so restarting the app mid-meeting does not re-log every line.
4. Saves when not recording do not touch the timeline.

## Acceptance criteria

- [x] Tests: body save keeps frontmatter exactly, including unknown keys; a body containing `---` survives; typing three lines over three saves yields three entries with the right times; editing one line yields one entry with the new text; reordering lines yields none; saving the same body twice yields none; saves while not recording yield none.
- [x] Every timeline line is valid JSON on its own line; text with quotes and unicode round-trips.
- [x] Test: a crash between temp write and rename leaves the old `notes.md` intact.
- [x] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Implemented 2026-11-12. All work is in `crates/mneme-store/`: new modules `src/notes.rs` and `src/append.rs`, new test file `tests/notes.rs`, small internal changes to `src/store.rs`, `src/transcript.rs` and `src/lib.rs`, and two dependencies in the crate's `Cargo.toml` (`serde` and `serde_json`, both inherited from the workspace). Nothing outside the crate was edited except this file; cargo added the two dependency names to this crate's entry in `Cargo.lock`.

### What was built

Public API (`mneme_store::…`):

```rust
// Read: the body of notes.md, everything after the frontmatter, byte for byte.
Store::notes(&self, id: &MeetingId) -> Result<String>

// Save without touching the timeline (meeting not recording).
Store::save_notes(&self, id: &MeetingId, body: &str) -> Result<()>

// Save while recording: also appends timeline entries for new and changed lines.
Store::save_notes_at(&self, id: &MeetingId, body: &str, meeting_time_ms: u64) -> Result<SavedNotes>

struct SavedNotes {
    timeline_entries: usize,          // entries this save appended
    timeline_error: Option<Error>,    // notes saved, timeline could not be appended to
}
```

All three take the store lock for the whole call and find the folder by id, so a save cannot undo a rename or a finish (the carry-over from 010), and a save after the folder moved lands in the new folder.

Crate-internal changes:

- **`append.rs`** (the carry-over from 011): `open_for_append`, `is_file_at` and `is_linked` moved here from `transcript.rs` unchanged, and the "start on a fresh line after a cut-off one, then one write" step became `write_lines(file, lines)`, which the transcript writer and the timeline both call. The transcript writes the same bytes in the same single write as before; its 53 tests are untouched and pass.
- **`store.rs`**: `load`, `OpenNotes` and its fields are now `pub(crate)`. `OpenNotes::stage()` is new (temp file written, not yet renamed); `OpenNotes::save()` is `stage` then `commit`, which is what `safe_write` did.
- The store keeps nothing in memory between saves. "The previously saved body" is the body on disk, read under the lock as part of the save.

### How the body round-trips

The body is the `NotesFile::body` field from 010: everything after the line that closes the frontmatter. A save sets that field and writes the file back; the frontmatter is the text that was read.

- A new meeting's body is the empty string. After `save_notes(id, "hello")` the file is the frontmatter followed directly by `hello`: no blank line is put between them and no newline is added at the end.
- Nothing is trimmed or normalized. A body with `\r\n` line endings, with mixed endings, without a final newline, with leading blank lines, with trailing spaces, with a byte order mark or a NUL in it is stored with exactly those bytes and read back with exactly those bytes (16 bodies in `the_body_is_stored_exactly_as_given`, compared as bytes on disk and through `notes()`).
- A file with CRLF frontmatter keeps it; the body keeps whatever endings it was given, so the two can differ.
- Saving the body that was just read writes nothing at all: the file is not replaced, even with the same bytes (checked by inode). Tested on five file shapes, including a blank line after the frontmatter (that line is part of the body and comes back from `notes()`), CRLF throughout, a byte order mark, spaces after the closing `---`, and a file that ends at the closing `---` with no newline.
- That last shape is the one place a byte is added: the body needs a line of its own, so the first non-empty save puts a newline after the `---`. The newline belongs to the delimiter line, not the body; the body still reads back as given.
- The body must be UTF-8, since it is a `&str`.

### Timeline rules

For a save with a meeting time, on a meeting whose frontmatter says `recording`:

1. Both bodies (on disk, and the new one) are split into lines with `str::lines` (so on `\n`; a lone `\r` does not end a line). Each line is reduced to its text without surrounding whitespace. Lines with no text left are dropped.
2. The two bodies are compared as bags of texts, ignoring position. Each text of the new body is paired with one unused copy of the same text in the old body. The texts of the new body left without a partner are the result, in the order they appear in the new body. When a text is in the new body more often than in the old, the first copies are the paired ones.
3. One entry per result text is formatted by `serde_json` from a struct (`{"t":"hh:mm:ss","text":"…"}`, `t` from `mneme_core::format_timestamp`), all entries of the save in one write, then `sync_data` (`F_FULLFSYNC` on macOS, as for the transcript).

What that gives:

| Case | Recorded |
| --- | --- |
| New line | Yes |
| Edited line | Yes, with the new text |
| Moved line, any reordering | No |
| Deleted line | No |
| Blank or whitespace-only line | No |
| Unchanged line, same body saved again | No |
| Indentation, trailing spaces or line ending changed | No |
| Same text on two lines, third copy added | One entry |
| Three identical lines pasted | Three entries |
| Line edited back to an earlier text (A, B, A) | A, B, then A again |
| Paste of thousands of lines | One entry each, one write, one sync; linear time |

The recorded text is the trimmed text, the same thing that is compared. So an indented `    - item` is recorded as `- item`.

Known imperfections, all consequences of comparing two snapshots line by line:

- **Whitespace inside a line, capitals and markers are text.** `- a b` to `- a  b`, `api` to `API`, and ticking `- [ ]` to `- [x]` each record the line again.
- **Splitting or joining lines** records the pieces (or the joined line) as new, although no character was typed.
- **Edited back** records the old text a second time: the store compares with the body on disk only and does not know what was there two saves ago. Read the earliest entry for a text as the time it was first typed.
- **A copy deleted in one place and typed in another within one save** looks like a move and is not recorded. An edit that turns a line into the text of a line deleted in the same save is likewise not recorded.
- **Marker-only lines.** Only blank lines are exempt, as the issue says. So an empty bullet (`-`, left by pressing Enter in a list and pausing), `#` or `---` is recorded as a line. See follow-ups.
- **All entries of one save share one time**, the one the caller passed, cut to whole seconds. A line's time is when the save saw it, not when its first key was pressed.
- **Lines that did not come through `save_notes_at`** have no entry: lines saved with the plain `save_notes` during a recording, and lines written into `notes.md` by another editor. Later saves find them on disk and treat them as unchanged.
- **Line separator characters inside a line** (U+2028, U+2029, U+0085) are written as they are, which is valid JSON. A reader must split the file on `\n` only.
- **No cap on length.** A line of any length is recorded whole.

### Growing lines

Each save records each line it finds changed, so a line that is still being typed is recorded once per save that catches it: `- ask`, `- ask Ana`, `- ask Ana about the API` (test `a_line_that_grows_between_saves_is_recorded_at_each_save`). This is the issue's rule, implemented as written.

Why this is a reasonable record, and not only the literal one:

- The first of those entries is the closest thing there is to the moment the line was started, which is the moment the enhancer wants to match against the transcript. Recording a line only once it has stopped changing would keep the end time and lose the start.
- It needs no memory in the store and no step at stop, so nothing is lost in a crash and the file stays append-only.
- Size is small. Worst case measured here: 100 lines of 67 characters saved every 5 characters (1,200 saves) gave a 6.4 KB `notes.md` and an 80 KB timeline (1,299 entries). The editor's autosave in `src/lib/autosave.ts` as it is today saves after an 800 ms pause, or every 5 s while typing never pauses, so the real number of states per line is a few, not a dozen. The cost grows with the square of a line's length, so the heavy case is prose typed as one long paragraph line: a 600-character paragraph saved every 5 characters would add about 40 KB (120 entries averaging 300 characters; worked out, not run).

The cost is to the reader: the timeline holds drafts, and nothing in an entry says which earlier entry it replaced. A reader can treat an entry as a draft when a slightly later entry starts with its text, but that is a guess, and it fails when a line was fixed in the middle.

**Where a better design conflicts with the issue.** Two changes would help the v2 reader, and both go against the issue or the PRD, so neither was made (see follow-ups):

1. Recording a line only once it has stopped changing. It conflicts with "typing three lines over three saves yields three entries" (the third would still be waiting), needs state in the store and a flush at stop, and loses the last line in a crash.
2. Saying in the entry what the line replaced, for example a third key holding the previous text when exactly one line changed. It keeps every test in this issue true but changes the format the PRD gives. It is the only one of the two that cannot be done later for meetings already recorded.

### Recording or not: who decides

The caller says "recording, at time T" by passing the time (`save_notes_at`); the store reads no clock. The status in frontmatter has the last word: if a time is passed for a meeting that is `complete` or `interrupted`, the notes are saved, the time is ignored and the timeline is not touched (`saves_while_not_recording_give_no_entry`, `an_interrupted_meeting_gets_no_entries`). It is not an error, because a save that was on its way when the meeting was stopped must still save the notes.

### A crash between the notes and the timeline

Order within one save, all under the store lock: (1) write the new `notes.md` to its temp file and sync it, (2) append the timeline entries and sync them, (3) rename the temp file over `notes.md`.

- **Crash between 2 and 3:** the old `notes.md`, a hidden temp file, and a timeline that already has the entries. **The timeline wins**: nothing typed is missing its time. If the lines are saved again they are compared with the old notes and recorded a second time with the later time. So the timeline can repeat a text and never lacks one. The other order (notes first) could leave lines in the notes with no entry for good, and the time a line was typed cannot be worked out afterwards; a repeated entry costs nothing, because the earliest one is the one to read. Test: `a_crash_between_the_timeline_and_the_notes_repeats_entries_and_drops_none`.
- **Crash during 2:** the timeline can end in a cut-off entry. The next append starts on a fresh line, so the cut-off entry stays behind as one line that is not valid JSON and every other line is whole (`entries_after_one_cut_off_by_a_crash_start_on_a_fresh_line`). A reader must skip lines that do not parse.
- **Step 1 fails** (full disk, folder not writable): the save returns an error and has logged nothing, so the retry does not double the entries (`a_save_that_cannot_write_its_temp_file_logs_nothing`). That is why the temp file is written before the timeline.
- **Step 2 fails:** the notes are saved anyway and the failure is returned in `SavedNotes::timeline_error` instead of as an error. The notes are the user's words and are never held back for the timeline. Those entries are lost (`a_timeline_that_cannot_be_written_never_blocks_the_notes`).
- **Step 3 fails:** the save returns an error with the old notes in place; the entries are already in the timeline and a retry repeats them.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0. Other agents were running CPU stress tests on the machine throughout (load average between 95 and 240).

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` (before any change) | 163 passed: 85 unit, 46 in `tests/meetings.rs`, 32 in `tests/transcript.rs`. |
| `cargo test -p mneme-store` | 212 passed, 0 failed, 0 ignored: 101 unit (3 new in `append`, 13 new in `notes`), 46 in `tests/meetings.rs`, 33 in `tests/notes.rs` (new), 32 in `tests/transcript.rs`. Run 7 times at the final code, same result. The 163 earlier tests are unchanged and pass. |
| `TZ=<zone> cargo test -p mneme-store` for UTC, Asia/Kolkata, America/Los_Angeles, Pacific/Kiritimati | 212 passed each time. |
| `cargo clippy -p mneme-store --all-targets -- -D warnings` | Exit 0, no warnings. |
| `cargo fmt -p mneme-store -- --check` | Exit 0. |
| `cargo doc -p mneme-store --no-deps` | No warnings. |

Acceptance criteria (integration tests in `tests/notes.rs` unless a module is named):

1. **Tests.**
   - Frontmatter kept exactly, unknown keys included: `saving_the_body_keeps_the_frontmatter_byte_for_byte` (a hand-edited block with a comment, a list, a flow list, a block scalar containing `---`, odd spacing and a trailing comment; plain save, save with a time, and save of an empty body), `frontmatter_with_windows_line_endings_or_a_byte_order_mark_is_kept`.
   - Body containing `---`: `a_body_with_delimiter_lines_survives` (also through a rename and a finish, and for a body that is only `---` or looks like frontmatter of its own).
   - Three lines over three saves: `three_lines_typed_over_three_saves_give_three_entries_with_their_times` (the file compared byte for byte; times 12 s, 4 min 12.999 s and 1 h 2 min 5 s).
   - One edited line: `editing_one_line_gives_one_entry_with_the_new_text`.
   - Reordering: `reordering_lines_gives_no_entry`. Same body twice: `saving_the_same_body_twice_gives_no_entry_the_second_time`.
   - Not recording: `saves_while_not_recording_give_no_entry` (plain save during the recording, both kinds of save after finish), `an_interrupted_meeting_gets_no_entries`.
   - First save after a restart: `the_first_save_after_a_restart_logs_only_what_changed` (a second `Store` on the same root).
2. **JSON.** `every_timeline_line_is_json_and_awkward_text_round_trips` and unit `notes::tests::awkward_text_is_escaped_onto_one_line_and_reads_back`: quotes, backslashes, a tab, control characters, NUL, several scripts, emoji, a line that is itself a JSON object, U+2028. The `timeline` helper used by every timeline test fails unless each line parses as an object with exactly `t` and `text`. Checked once with a second parser as well: a throwaway test (deleted) wrote a timeline of nine awkward lines, and Python's `json` module parsed every line and got the note lines back exactly; `jq` read the same file.
3. **Crash between temp write and rename.** Unit `notes::tests::a_crash_before_the_rename_leaves_the_old_notes_intact`: the save is stopped after the temp file is written and before the rename, with no destructor run; `notes.md` is byte for byte the old file, the meeting still loads, and the next save goes through.
4. **Test and clippy commands**: as in the table.

Other tests: body stored exactly, no-op save, file ending at the delimiter, frontmatter updates leaving the body alone (`updating_frontmatter_never_touches_the_body`), unknown id, a save following a renamed folder, copies counted, edited back, trimmed text, append-only (each save's file starts with the previous one), notes edited in another editor, deleted timeline recreated, 5,000-line paste, saves racing renames and racing the stop from two threads, notes and transcript interleaved.

**The tests fail when the behavior is broken.** Each of these 32 changes was made in turn by a script, the whole suite run, and the change undone. 31 made at least one test fail: body trimmed; CRLF normalized; final newline added; blank line put before the body; frontmatter rebuilt from the parsed values; diff by position; diff by set without counting copies; diff without trimming; blank lines recorded; diff against an empty body; deleted lines recorded; status ignored; plain save logging; `notes.md` renamed before the timeline is appended; `notes.md` written in place; timeline appended before the temp file is written; timeline failure failing the save; timeline failure swallowed; entry count not reported; no store lock around a save (run 5 times, caught 5 times, by both two-thread tests); JSON written by hand; time as bare seconds; time rounded up; keys in the other order; entries without newlines between them; timeline truncated on each save; no fresh line after a cut-off entry; a spare newline before every append; unchanged file rewritten anyway; read returning the whole file; read trimming the body. The one change no test catches is removing the `sync_data` call on the timeline, as with the transcript in 011. One thing to know about the method: the script first restored each file with its old modification time, so after the last change cargo kept the broken build and one later run of the suite failed two tests against sources that were already correct. The files were touched, the suite rebuilt, and every run counted in the table above is from after that.

Measured with a throwaway test (deleted), debug build, under the load noted above. Two runs; the figures are from the slower one:

| What | Time |
| --- | --- |
| Save with a time that adds one entry | 14 ms on average, 25 ms worst of 1,200 |
| Plain save | 10 ms |
| Save of an unchanged body | 0.35 ms |
| Paste of 20,000 lines (969 KB) in one save | 85 ms |
| Save that adds one entry, with 501 meetings in the root | 63 ms |
| `notes()` with 501 meetings in the root | 57 ms |

The last two rows are the id lookup from 010, which reads every `notes.md` under the root on each call; see follow-ups.

### Not verified

- **That the timeline sync does what it says.** No test can tell a synced entry from one still in the OS cache. The "timeline first" guarantee across a power cut rests on the `sync_data` call being there.
- **A real crash.** The crash tests stop the save between its steps inside the test process. No process was killed mid-save and no machine was crashed. (011's SIGKILL test covers the shared append code for the transcript.)
- **The UI's real autosave against this code.** The numbers on growing lines for the 800 ms and 5 s timers are reasoning from `src/lib/autosave.ts`, not a measurement; only the every-5-characters worst case was run.
- **Timings on a quiet machine.** All were taken under heavy load from other work and in a debug build.
- **A volume that rejects `F_FULLFSYNC`.** There each save that logs would report `timeline_error`; the notes save itself already fails on such a volume (010).
- **The `#[cfg(not(unix))]` branches**, here and in the moved helpers. Never compiled.

### Deviations from the issue and PRD

- **`save_notes_at` returns `SavedNotes`**, not `()`, so a timeline failure can be reported without making the notes save look failed, and so callers and tests can see how many entries were written.
- **Timeline before notes.** The issue does not give an order; the reasons are above.
- **Synced, not just flushed**, as 011 chose for the transcript.
- **The recorded text is trimmed.** The issue says `"text":"<line>"`. Indentation and trailing whitespace are not recorded, because a line is identified by its trimmed text and indenting it must not count as typing.
- **Meeting ids are taken as an argument**, which the issue's `save_notes_at(body, meeting_time_ms)` leaves out.
- **No reader.** The PRD says nothing reads the timeline in v1; the tests parse it themselves.

### Follow-ups

- **Decision wanted before v1 ships: drafts in the timeline.** As it stands the timeline holds every state of a growing line and nothing links a state to the one before. If the v2 enhancer should not have to guess, the cheap fix is an optional third key with the text the line replaced, written when exactly one line changed in a save. That needs a PRD change to the entry format, and it cannot be added afterwards for meetings already recorded, which is the same reason the timeline exists in v1 at all. The alternative, recording a line only when it stops changing, conflicts with this issue's tests and is worse in a crash. Doing nothing is also workable: the v2 reader collapses drafts by prefix.
- **Decision wanted: marker-only lines.** An empty bullet left by Enter-then-pause is recorded as `-`. Treating a line with no letter or digit as blank would remove those entries; a note that is only `??` or an emoji would then get no entry. It is a one-line change in `notes::texts`.
- **For 013 (load meeting).** Call the crate-internal `store::load(&folder)` once and take `open.summary` and `open.doc.body` from it. Do not call `Store::notes` while holding the store lock: the lock is not re-entrant. Recovery has nothing to do for the timeline; a cut-off last entry is left as it is. A crash mid-save can leave a hidden `.notes.md.<pid>-<n>.tmp` in the meeting folder; `is_temp_file_name` recognizes it for cleanup.
- **For 013 (find by id).** Every notes save and read looks the folder up by scanning the root: about 50 ms per call with 500 meetings on this (loaded) machine, on top of a 10 to 14 ms save (measured: 63 ms against 14 ms). The optional id cache in 013's scope would remove that from every autosave.
- **For 021 (save notes).** If the session is recording this meeting, call `save_notes_at(&id, &notes, elapsed_ms)` with the session's clock, the same one segment `start_ms` values are on; otherwise `save_notes`. There is no need to check the status first: a time passed for a meeting that is no longer recording is ignored. Treat `Err` as "not saved" (the UI retries). Treat `SavedNotes::timeline_error` as something to log, not as a failed save. Entries are only written while the status is `recording`, so the editor's before-stop flush has to reach the store before `finish_meeting` for its last lines to be timed; a stop from the menu bar or the shortcut that skips the flush leaves those lines untimed (this is the open point from 015 in 021's notes). The acceptance test "notes saved during recording produce timeline entries" can assert on `SavedNotes::timeline_entries`.
- **For v2 (reading the timeline).** Split on `\n` only. Skip lines that are not valid JSON. The same text can appear more than once; the earliest entry is when it was first typed. Texts are trimmed, so look a note line up by its trimmed text. Not every note line has an entry (see imperfections).
- **PRD 04, Note timeline.** Could say that the text is the trimmed line and that an entry can repeat. The PRD is outside this lane.
- **`issues/README.md`** lists 012 as `in progress`; the table is outside this lane.

### Decisions after the hand-back (2026-11-12)

- **Drafts:** Francesco chose to leave the entry format as `{"t","text"}`. No replaced-text key is added; v2 matches drafts of a growing line by prefix.
- **Marker-only lines:** left as implemented. An empty bullet saved on its own is recorded as `-`; filtering it would also drop notes that are only `??` or an emoji.
