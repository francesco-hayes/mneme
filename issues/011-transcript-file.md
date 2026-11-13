# 011: Transcript append, parse, sort

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 010 · Owns: `crates/mneme-store/`

## Goal

Write the transcript one line at a time so a crash loses seconds, and read it back reliably.

## Scope

1. **Line format**: `[hh:mm:ss] Me: text` / `[hh:mm:ss] Them: text`, using `mneme-core`'s timestamp functions. Newlines and runs of whitespace inside text become single spaces; empty text is not written.
2. **`TranscriptWriter`** for a meeting: `append(segment)` writes one line and flushes to disk before returning. Safe to call from the pipeline's worker thread.
3. **`finalize()`**: sorts the file by timestamp, stable for equal timestamps, through the safe write helper. Malformed lines are kept at the end rather than dropped.
4. **Parser**: returns lines in file order as `(timestamp, speaker, text)`; skips malformed lines and reports how many were skipped. A cheap line count for the UI.
5. A half-written last line (crash mid-append) is handled: the parser skips it; the next append starts on a new line.

## Acceptance criteria

- [x] Tests: format and parse round-trip, including text containing brackets and colons; out-of-order appends are sorted by `finalize`; equal timestamps keep arrival order; multi-line text is flattened; a truncated last line is skipped and the next append is well-formed; line count matches.
- [x] Test: lines appended before a simulated crash (writer dropped without finalize) are all on disk.
- [x] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Implemented 2026-11-12. All work is in `crates/mneme-store/`: the new module `src/transcript.rs`, the new test file `tests/transcript.rs`, and small internal changes to `src/store.rs` and `src/lib.rs` (below). No dependency was added and nothing outside the crate was edited except this file.

### What was built

Public API (`mneme_store::…`):

```rust
// Writer: opened by meeting id, owns no borrow of the store, Send + 'static.
Store::transcript_writer(&self, id: &MeetingId) -> Result<TranscriptWriter>
TranscriptWriter::append(&mut self, segment: &mneme_core::Segment) -> Result<bool>  // false: empty text, nothing written
TranscriptWriter::finalize(&mut self) -> Result<TranscriptStats>                     // sort; writer stays usable
TranscriptWriter::meeting_id(&self) -> &MeetingId
TranscriptWriter::path(&self) -> &Path                                               // where the file was last seen

// The same sort without a writer (recovery).
Store::finalize_transcript(&self, id: &MeetingId) -> Result<TranscriptStats>
struct TranscriptStats { lines: usize, malformed: usize, last_timestamp_ms: Option<u64> }

// Parser. Never fails on content.
Store::transcript(&self, id: &MeetingId) -> Result<Transcript>
Transcript::read(path: &Path) -> Result<Transcript>      // a missing file is an empty transcript
Transcript::parse(bytes: &[u8]) -> Transcript            // no I/O, no error
Transcript::last_timestamp_ms(&self) -> Option<u64>      // the latest timestamp of any line
struct Transcript { lines: Vec<TranscriptLine>, skipped: usize }      // lines in file order
struct TranscriptLine { timestamp_ms: u64, speaker: mneme_core::Speaker, text: String }
TranscriptLine::timestamp(&self) -> String               // "hh:mm:ss"

// Line count: equals Transcript::lines.len(), without building the lines.
Store::transcript_line_count(&self, id: &MeetingId) -> Result<usize>
count_transcript_lines(path: &Path) -> Result<usize>

// The line format on its own.
format_transcript_line(segment: &Segment) -> Option<String>   // no newline; None when the text is empty
```

Crate-internal, for 013: `transcript::finalize_file(path) -> Result<TranscriptStats>`, the sort for a caller that already holds the store lock.

Rules the code follows:

- **Line format.** `[hh:mm:ss] Me: text` or `[hh:mm:ss] Them: text`, from `mneme_core::format_timestamp(segment.start_ms)` and `Speaker`'s `Display`. In the text, every run of whitespace or control characters (line breaks of every kind, tabs, no-break spaces, NUL) becomes one space and the ends are trimmed. A segment with no text left is not written.
- **Parsing a line.** The timestamp ends at the first `] `, the speaker at the first `: ` after it, so brackets and colons in the text are plain text. Timestamps go through `mneme_core::parse_timestamp`, speakers through `Speaker::from_str`; both are exact. A line with no text after the speaker is malformed. A trailing `\r` and a byte order mark at the start of the file are tolerated. The file is read as bytes, so invalid UTF-8 is just a malformed line.
- **Complete lines.** A line counts once it ends in a newline. Whatever follows the last newline is a line cut off mid-write: the parser skips it and counts it in `skipped`, the line count leaves it out, and the next append writes a newline first so it starts on a fresh line. Blank lines are ignored and not counted as skipped.
- **Sort.** Well-formed lines first, by parsed timestamp (so `100:00:00` comes after `99:00:00`), stable for equal timestamps. Timestamps on disk are whole seconds, so two segments in the same second keep arrival order whatever their milliseconds were. Malformed lines follow in their original order, byte for byte. Blank lines are dropped. The result is written through `PendingWrite::stage` + `commit` (the safe write helper). A file already in order is not rewritten, so finalizing twice changes nothing.

Internal changes to 010's code, with no change to its public API: `Store`'s lock is now an `Arc<Mutex<()>>` so writers can share it, `Store::lock` and `Store::find_folder` are `pub(crate)`, and the folder lookup is also a free function `store::find_folder(root, id)`.

### Durability choice

`append` does one `write` of the whole line to a file opened in append mode, then `File::sync_data`, and only then returns. On macOS `sync_data` is `fcntl(F_FULLFSYNC)` (checked in the std source: Apple targets use it for both `sync_all` and `sync_data`, with no fallback), so the line is on the disk itself, not in a cache.

Why the strong form: the audio is dropped as soon as a chunk is transcribed, so a line that is lost cannot be made again. The cost was measured on this machine (internal APFS SSD, debug build, 2,000 lines of about 100 bytes): 4.2 ms per append on average, 5.1 ms at the 95th percentile, 8.5 ms at worst, 8.5 s in total for what stands in for an hour-long meeting. A standalone loop doing the write alone, with no sync, took 4 µs per line. A few milliseconds per line is nothing next to the seconds Whisper spends on each chunk.

Crash consequence: every line whose `append` returned survives an app crash, a forced quit, a kernel panic and a power cut. Only the line being written at that moment can be lost or cut short. Nothing is buffered in the writer, so dropping it without `finalize` loses nothing; the file is merely unsorted. With write-only (no sync) the first two cases would be equally safe, and the last two could lose the lines the OS had not flushed yet.

The sync runs after the store lock is released, so a notes save or a rename never waits behind it.

### Folder rename

The writer is opened by meeting id and holds an open file handle plus the path it last saw. An open file stays the same file when its folder is renamed, so appends keep landing in the right file. Under the store lock, before each append and each sort, the writer checks that its path still names the file it has open (device and inode). If not, it finds the folder again by id and reopens the file there. So:

- finalize then finish, finish then finalize, and appends after finish all work (three tests, with a title typed during recording so that finish moves the folder). Nothing is ever created under the old folder name.
- A folder renamed by hand mid-recording is followed the same way.
- A transcript replaced by an editor's save (new file renamed over the old one, old one possibly kept as a backup) is followed: later appends go to the new file. A deleted transcript is created again.
- If the folder moved and the meeting cannot be found by id (broken `notes.md`), appends still go to the open file; `finalize` returns `MeetingNotFound` because it has no path to sort.
- If the whole folder is gone, `append` and `finalize` return `MeetingNotFound`.

`append` and `finalize` take the store lock, so a sort never runs between another writer's check and its write, and a folder rename never lands between the sort's read and its rename.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` | 142 passed, 0 failed, 0 ignored: 76 unit (21 new in `transcript`), 35 in `tests/meetings.rs` (unchanged), 31 in `tests/transcript.rs`. Run 6 times, same result. The 90 tests from 010 all pass. |
| `cargo clippy -p mneme-store --all-targets -- -D warnings` | Exit 0, no warnings. The `unused dependency mneme-core` manifest warning from 010 is gone. |
| `cargo fmt -p mneme-store -- --check` | Exit 0. |
| `cargo doc -p mneme-store --no-deps` | No warnings. |

Acceptance criteria:

1. Tests, all passing:
   - Format and parse round-trip with brackets and colons: `appended_segments_read_back_as_the_same_values` (through the writer and the store), unit `a_formatted_line_parses_back_to_the_same_values`, `a_segment_is_formatted_as_one_strict_line`, `lines_that_are_not_strict_are_rejected`.
   - Out-of-order appends sorted: `finalize_sorts_lines_that_arrived_out_of_order` (exact file contents, including hours past 99).
   - Equal timestamps keep arrival order: `lines_with_the_same_timestamp_keep_the_order_they_arrived_in`.
   - Multi-line text flattened: `text_with_line_breaks_is_written_as_one_line`, unit `whitespace_of_every_kind_becomes_single_spaces`; empty text: `a_segment_with_no_text_is_not_written`.
   - Truncated last line skipped, next append well-formed: `a_cut_off_last_line_is_skipped_and_the_next_append_starts_a_new_line`, `a_line_cut_off_inside_its_timestamp_stays_out_of_the_transcript`, unit `a_last_line_without_its_newline_is_skipped_even_if_it_looks_whole` (a two-line file cut at every byte offset).
   - Line count matches: `the_line_count_matches_the_parsed_lines`.
   - Malformed lines kept at the end: `finalize_keeps_malformed_lines_at_the_end`, unit `sort_never_loses_or_changes_a_line`. Parser never fails: `reading_never_fails_on_a_file_that_is_not_a_transcript`, unit `parse_never_fails_on_junk`.
2. Crash:
   - `lines_appended_before_a_crash_are_all_on_disk`: 25 appends, then the writer is thrown away with `mem::forget` (no destructor) and again with a plain drop; the file holds exactly the appended lines.
   - `lines_survive_a_killed_process`: the test binary starts itself as a child process that appends 30 lines and waits; the parent kills it with SIGKILL and reads 30 whole lines.
   - Crash during the sort: unit `a_crash_during_the_sort_leaves_the_unsorted_file_whole` stops between writing the temp file and the rename; the unsorted file is intact and the next sort completes.
3. Test and clippy commands as in the table.

Other tests: rename orders (above), `a_worker_thread_appends_while_the_meeting_is_renamed_and_read` (writer on its own thread while another thread renames, reads and finishes the meeting), `two_writers_on_one_transcript_never_tear_or_lose_a_line`, `finalize_twice_changes_nothing_the_second_time`, `a_transcript_can_be_finalized_without_a_writer`, `an_unknown_meeting_is_not_found`.

**The tests fail when the behavior is broken.** Each of these changes was made to `transcript.rs` in turn, the suite run, and the change undone; every one made at least one test fail: no sort; ties reversed; sort by text instead of time; no flattening; empty text written; no newline before an append after a cut line; writer never re-finds the file; writer checks only that its file still exists (not that the path names it); no fallback to the open file; parser accepts an unterminated last line; malformed lines dropped by the sort; line count as a count of newlines; byte order mark ignored; sort written in place instead of temp file and rename; sorted file rewritten anyway; speaker label parsed loosely; empty text accepted by the parser; append without the store lock (caught by the two-writers test in 4 runs of 4); a writer that buffers lines in memory (both crash tests fail). One change no test can catch: removing the `sync_data` call. See below.

Measured with a throwaway test (deleted afterwards), debug build, 2,000-line transcript of 198 KB: line count 5 ms, full parse 5 ms, first sort 15 ms, second sort (no rewrite) 4 ms.

### Not verified

- **That the sync does what it says.** No test can tell a synced line from one still in the OS cache; the suite passes with the `sync_data` call removed. The claim about kernel panics and power cuts rests on the call being there and on the std source, not on a crashed machine.
- A real forced quit of the app. The SIGKILL test is the closest stand-in; the manual check is in 021.
- A volume that rejects `F_FULLFSYNC` (some network shares). There `append` would write the line and then return an error on every call. 010's `safe_write` has the same limit, so such a root already fails at `create_meeting`.
- The `#[cfg(not(unix))]` branches. They compile only on other platforms and were never built.
- Timings are from this one machine.

### Deviations from the issue and PRD

- **Synced, not just flushed, per line.** Stronger than the PRD's "appended and flushed". Reasons and cost above.
- **A cut-off last line can become a real line.** While it has no newline it is skipped, as the issue says. Once the next append or the sort gives it its newline, it is a line like any other: if what is left still reads as `[hh:mm:ss] Me: text` it is kept in the transcript with its shortened text, otherwise it is malformed and goes to the end. Nothing in the format can tell a cut line from a short one, and the words in it are real.
- **Blank lines** are not counted as skipped and are removed by the sort.
- **Control characters** are flattened along with whitespace; a strict one-line format should not carry them.
- **`append` takes `&Segment` and returns `Result<bool>`**, so the caller can count lines actually written.
- **Reading by path and by id.** A missing `transcript.md` reads as empty rather than as an error; the sort leaves it missing.
- **Extra API** beyond the scope list: `Store::finalize_transcript` (013 needs a sort without a writer), `TranscriptStats`, `format_transcript_line`.
- A helper, `crash_child_appends_and_waits_to_be_killed`, shows up in the test list. It is the child half of the kill test and does nothing when run on its own.

### Follow-ups

- **For 009.** Get the writer from `Store::transcript_writer(&id)` and move it to the worker thread. The sink adapter is two calls: `append(&segment)` and `finalize()`. `append` returns `Ok(false)` for an empty segment; count lines from `Ok(true)`, or take `TranscriptStats::lines` from `finalize`. Do not retry an `append` that returned an error: the line may already be in the file. `finalize` and `Store::finish_meeting` can run in either order, and a segment that arrives after `finalize` can still be appended (call `finalize` again afterwards). With `MNEME_KEEP_AUDIO=1`, take the folder from `store.meeting(&id)` at the time of writing, not from `writer.path()`, which can be out of date after a rename.
- **For 013.** Recovery holds the store lock, so it must call the crate-internal `transcript::finalize_file(&summary.transcript_path())`, not `Store::finalize_transcript`: the lock is not re-entrant and that call would deadlock. `TranscriptStats::last_timestamp_ms` is the last line's start time in milliseconds (`None` for an empty transcript); duration in seconds is `last_timestamp_ms.unwrap_or(0) / 1000`. A cut-off last line that still reads as a line counts toward it after the sort (see deviations), so derive the expected duration in the recovery test from what `finalize` returns. Running it twice rewrites nothing. For "load meeting", `count_transcript_lines(&summary.transcript_path())` needs no second folder scan.
- **For 021.** `getTranscript` maps each `TranscriptLine` to `{ t: line.timestamp(), speaker: line.speaker.as_str(), text: line.text }`; `transcriptLineCount` is `Store::transcript_line_count`. `Transcript::skipped` is there if the UI ever wants to say that lines were left out. The types do not derive `serde`, as in 010.
- **For 012.** The note timeline is the same kind of file (append one line, sync). `open_for_append`, `ends_with_newline` and the follow-the-file check in `transcript.rs` are private but written to be shared; move them to a common module rather than copying.
- **Last line without a newline in a hand-edited file.** If someone edits a finished transcript and their editor removes the final newline, the last line is skipped by the parser and the count until something sorts the file again. That is the rule the issue asks for. If it turns out to matter, the change is one place in `Transcript::parse` and `count_lines`: accept the unterminated tail when it parses.
- **`issues/README.md`** still lists 011 as `in progress`; the table is outside this lane.
