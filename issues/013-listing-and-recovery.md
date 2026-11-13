# 013: Meeting list and crash recovery

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 012 · Owns: `crates/mneme-store/`

## Goal

List meetings from the folders alone, find one by id, and repair meetings a crash left open.

## Scope

1. **List**: scan the root's direct subfolders, read each `notes.md` frontmatter, return summaries (id, title, date, duration, status, folder path) newest first by `date`. Skip folders with no `notes.md` or unreadable frontmatter, and report them as warnings rather than failing the list.
2. **Find by id**: resolve an id to its folder. Works after the folder was renamed outside Mneme. A simple scan is fine; an in-memory cache refreshed on list is optional.
3. **Load meeting**: summary plus notes body and transcript line count.
4. **Recovery**: for every meeting with `status: recording` (none should exist at launch): finalize the transcript (sort), set `duration` from the last transcript timestamp (zero if empty), set `status: interrupted`, apply any pending folder rename. Returns what it repaired.
5. Edits made outside Mneme (title changed in frontmatter, notes edited in Obsidian) show up on the next list or load; nothing is cached on disk.

## Acceptance criteria

- [x] Tests: ordering by date not by folder name; skip and warn on junk folders and files in the root; find by id after an external folder rename; duplicate ids are reported, not silently merged.
- [x] Recovery tests: a meeting left as `recording` with out-of-order transcript lines and a truncated last line becomes `interrupted`, sorted, with the right duration; running recovery twice changes nothing the second time.
- [x] Listing 500 generated meetings completes fast enough for a launch (state the measured time in the Result).
- [x] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Implemented 2026-11-12. All work is in `crates/mneme-store/`: new modules `src/list.rs` and `src/recovery.rs`, new test files `tests/listing.rs` and `tests/recovery.rs`, a new example `examples/list_bench.rs`, and small changes to `src/store.rs`, `src/meeting.rs`, `src/error.rs`, `src/safe_write.rs` and `src/lib.rs`. No dependency was added and nothing outside the crate was edited except this file. The earlier test files were not touched.

### What was built

Public API (`mneme_store::…`):

```rust
// List: one scan of the root. Err only when the root itself cannot be read.
Store::list_meetings(&self) -> Result<MeetingList>
struct MeetingList { meetings: Vec<MeetingSummary>, warnings: Vec<ListWarning> }
struct ListWarning { path: PathBuf, reason: SkipReason }      // Display: "<path>: <reason>", one line
#[non_exhaustive] enum SkipReason {
    NotAFolder,                                  // a file or a link in the root
    NoNotesFile,                                 // a folder with no notes.md
    Unreadable(Error),                           // notes.md cannot be read, or broken frontmatter
    DuplicateId { id: MeetingId, listed: PathBuf },   // another folder has this id; `listed` is the one shown
}

// Find by id: unchanged signature, new behavior for duplicates.
Store::meeting(&self, id: &MeetingId) -> Result<MeetingSummary>     // .folder is the folder
Error::DuplicateId { id: MeetingId, folders: Vec<PathBuf> }         // new variant; folders sorted

// Load meeting: summary, notes body and transcript line count from one lookup.
Store::load_meeting(&self, id: &MeetingId) -> Result<Meeting>
struct Meeting { summary: MeetingSummary, notes: String, transcript_line_count: usize }

// Recovery. Err only when the root itself cannot be read.
Store::recover(&self) -> Result<RecoveryReport>
struct RecoveryReport {
    recovered: Vec<RecoveredMeeting>,     // were `recording`, now `interrupted`; folder name order
    failed: Vec<RecoveryFailure>,         // still `recording`; the next call tries again
    in_progress: Vec<MeetingSummary>,     // `recording` and created by this Store: not touched
    warnings: Vec<ListWarning>,           // the same warnings a list gives
}
RecoveryReport::nothing_to_do(&self) -> bool          // nothing recovered and nothing failed
struct RecoveredMeeting {
    meeting: MeetingSummary,              // as it now is on disk
    transcript: TranscriptStats,          // what the sort left; duration = last_timestamp_ms / 1000
    rename_error: Option<Error>,          // closed, but the waiting folder rename could not be done
    removed_temp_files: Vec<PathBuf>,
}
struct RecoveryFailure { meeting: MeetingSummary, error: Error }
```

**What is in the root, and what the list does with it**

| Entry | Listed | Warning |
| --- | --- | --- |
| Folder whose `notes.md` has readable frontmatter | yes | none |
| Folder with no `notes.md` (including the empty folder a crash during create leaves) | no | `NoNotesFile` |
| Folder whose `notes.md` cannot be read, is not UTF-8, is a directory, or has broken frontmatter | no | `Unreadable(error)`; the error names the file and, for frontmatter, the key |
| A folder whose id another folder has too | once | `DuplicateId` for every folder but the listed one |
| A file or a link in the root | no | `NotAFolder` |
| A hidden file or link (`.DS_Store`) | no | none |
| A hidden folder with no `notes.md` (`.obsidian`, `.trash`) | no | none |

Warnings are sorted by path. A link to a folder is not followed: it could point outside the root or at a folder that is already listed. A hidden folder that does hold a `notes.md` is a meeting like any other, because identity is the id and not the name.

**Order.** Newest first by the instant in `date` (`StartTime`'s ordering, which compares the instant and ignores the offset), so a meeting recorded at 09:00 in Seattle sorts after one recorded at 14:30 in Oslo on the same date. Not by folder name and not by the text of `date`. **Tie-break:** meetings with exactly the same instant are ordered by folder name, last name first, so of two meetings started in the same second `… Meeting 2` comes before `… Meeting`. The result does not depend on the order the file system returns entries in.

**Duplicate ids.** One meeting cannot be in two folders, and the store does not guess which one is meant:
- Every call that takes the id (`meeting`, `load_meeting`, `notes`, `save_notes`, `save_notes_at`, `rename_meeting`, `finish_meeting`, `transcript`, `transcript_line_count`, `finalize_transcript`, `transcript_writer`) returns `Error::DuplicateId` naming every folder, and writes nothing.
- The list shows the id once, from the first folder by name whose frontmatter is readable, with a `DuplicateId` warning for each other folder. It is shown rather than hidden so the user can see the meeting and get the error that names both folders when they open it.
- Removing one folder, or changing the id in one, makes the id work again without a restart.
- Recovery works on folders, not ids, so two `recording` folders with one id are each repaired; the duplicate is in the report's warnings.

**Edits made outside.** Nothing about a meeting is written anywhere but in its folder, and no result is reused: every list, load and lookup reads the files as they are. A title changed in frontmatter, notes edited in another editor, transcript lines added by hand, a folder renamed, added or deleted in Finder all show on the next call (`edits_made_outside_show_up_on_the_next_list_and_load`).

**Listing takes the store lock** for the scan (about 12 ms for 500 meetings), so it never sees a folder between its creation and its `notes.md`, or under a name a rename is just taking away.

### The id cache: built, as hints

Built. `Store` keeps, in memory only, the folder each id was in at the last scan (`list::FolderHints`). It is written by every scan (`list_meetings`, `recover`, and any lookup that has to scan), by `create_meeting` for the new id, and by the store's own folder renames.

How it stays correct:
- **A hint is never trusted.** Before it is used, the `notes.md` in the hinted folder is read and must still carry the id. If the folder was renamed, deleted, replaced by another meeting's folder, or its id was edited, the check fails and the root is scanned as before. Cost of the check: one small file read.
- **It is not a second source of truth.** It is never written to disk, a new `Store` starts without it, every scan throws it away and rebuilds it, and it only ever holds what a scan would find.
- **It never hides a duplicate that a scan knew of.** An id gets a hint only if the last scan found it in exactly one folder, or if this store created the meeting.

The one thing a hint delays: a folder **copied** while the app runs gives an id two folders, and a lookup that hits a valid hint does not notice. Until the next scan, calls keep going to the folder the store was already using and never switch to the copy. The next `list_meetings` reports the duplicate, and from then on the id is refused. Test: `a_copy_made_while_the_app_runs_never_takes_over_and_is_caught_by_the_next_list`. I considered closing that gap with the root folder's modification time and decided against it: it is only reliable on file systems with fine timestamps, and making it reliable needs the system clock, which this crate does not read.

Effect, measured (release build, 500 meetings, see below): a lookup by id went from 12 ms to 0.04 ms, and a notes save from 21 ms to 8 ms, which is the cost of the sync alone. With 2,000 meetings the save stays at 8 ms instead of 58 ms.

`TranscriptWriter` re-finds a moved folder through the free function `store::find_folder(root, id)`, which has no store and so no hints: always a scan, with the same duplicate reporting. That path only runs after a folder was moved.

### Recovery

For every folder whose `notes.md` says `recording`, in folder name order, under the store lock:

1. Sort `transcript.md` with the crate-internal `transcript::finalize_file` (not `Store::finalize_transcript`, which would deadlock). Every line is kept; a cut-off last line becomes a transcript line if it still reads as one, otherwise it goes after the others.
2. Set `duration` to `last_timestamp_ms.unwrap_or(0) / 1000` from what the sort returned, and `status` to `interrupted`, through `Store::close(open, secs, MeetingStatus::Interrupted)`: one atomic replacement of `notes.md`.
3. `close` then applies a folder rename that a title typed before the crash was waiting for, by `pending_folder_name()`. Default-titled folders, numbered or not, stay where they are. A rename that cannot be done is reported in `RecoveredMeeting::rename_error`; the meeting is still recovered.
4. Delete the temp files that interrupted saves of `notes.md` and `transcript.md` left in that folder, and list them in `removed_temp_files`.

The notes body, the timeline and every other frontmatter key are left byte for byte.

**Failures.** The sort comes before the status change on purpose: if the status changed first, nothing would ever come back to sort the file. A meeting that fails in step 1 or 2 is still `recording` with all its data, is reported in `failed` with the error, and is tried again by the next call. The others are repaired regardless. A crash in the middle of recovery leaves the same two possibilities.

**Idempotent.** A second call finds no `recording` meeting and writes nothing at all (checked by content and by inode).

**Never against a live recording.** The rule for the caller, stated in the docs of `Store::recover`: call it once at launch, before the first list and before any meeting is started. On top of that the store makes the mistake harmless: it remembers the ids of the meetings it created, and recovery never treats one of those as a leftover. They are returned in `in_progress` and not touched, not even their temp files. So with one `Store` per root, which is how the store is meant to be used, calling `recover` at any time, from any thread, cannot close a recording (`a_meeting_this_store_is_recording_is_never_treated_as_a_leftover`, `recovery_called_during_recordings_on_other_threads_never_closes_one`). What the store cannot see is a recording made through a second `Store` value on the same root, or by another running copy of the app; see follow-ups.

**Leftover temp files: removed, narrowly.** Only in the folders of meetings being recovered, only plain files, and only names of the exact form the safe write helper produces for those two files (`.notes.md.<pid>-<n>.tmp`, `.transcript.md.<pid>-<n>.tmp`; new crate-internal `safe_write::is_temp_file_of`). Such a file is a save that never took effect and may be cut short, and nothing reads it. A hidden `.tmp` file with any other name is not ours to delete. Temp files in meetings that are not being recovered are left; they are hidden and harmless.

**The empty folder from a crash during create: not cleaned up.** A crash between creating the folder and writing `notes.md` leaves a folder with at most two empty files. Listing skips it with a `NoNotesFile` warning every time, recovery reports the same warning, and nothing in this crate deletes it. Reason: the store cannot prove such a folder is its own. A folder that a sync service is still filling looks the same for a moment, and deleting it locally could delete it everywhere. The cost of leaving it is a warning in the log and a number on the next folder created in that minute. The user can delete it.

### Listing time

`cargo run -p mneme-store --release --example list_bench` generates 500 finished meetings in a temp folder (each a `notes.md` in the store's own shape with 30 lines of notes, and a 200-line transcript) and times each call 20 or 200 times. Apple Silicon Mac (Mac14,9, 12 cores), macOS 27.0.1, internal SSD. Load averages at the time, from `uptime`: 3.97, 6.74, 24.62 (other agents were building on the same machine; an earlier run at 1-minute load 9.6 gave medians within 2 ms of these).

| Call, 500 meetings | min | median | max |
| --- | --- | --- | --- |
| `list_meetings`, new store | 10.2 ms | 12.2 ms | 13.3 ms |
| `recover`, nothing to do | 10.0 ms | 10.8 ms | 14.1 ms |
| `recover` then `list_meetings`, new store (a launch) | 20.6 ms | 23.2 ms | 25.8 ms |
| `meeting(id)`, new store (scan) | 10.4 ms | 12.2 ms | 12.7 ms |
| `meeting(id)`, same store (hint) | 0.03 ms | 0.04 ms | 0.07 ms |
| `load_meeting(id)`, same store | 0.06 ms | 0.07 ms | 0.13 ms |
| `save_notes`, same store (hint) | 7.9 ms | 8.0 ms | 12.6 ms |
| `save_notes`, new store (scan) | 19.0 ms | 21.0 ms | 27.0 ms |

With 2,000 meetings: list 51 ms median, launch (recover then list) 101 ms, lookup with a hint 0.04 ms, save with a hint 8 ms. The scan is linear in the number of meetings, about 25 µs each. In the debug-build test the list of 500 took 23 ms.

A launch with 500 meetings spends about 23 ms in the store (recovery plus the first list). That is fast enough. The files had just been written, so these are warm-cache figures; see "Not verified".

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` (new code in place, before any new test) | 212 passed: the earlier suite, unchanged. |
| `cargo test -p mneme-store` | 256 passed, 0 failed, 0 ignored: 102 unit (1 new), 24 in `tests/listing.rs` (new), 19 in `tests/recovery.rs` (new), 46 in `tests/meetings.rs`, 33 in `tests/notes.rs`, 32 in `tests/transcript.rs`. Run 5 times in a row at the final code, same result. The 212 earlier tests are unchanged and pass. |
| `TZ=<zone> cargo test -p mneme-store` for UTC, Asia/Kolkata, America/Los_Angeles, Pacific/Kiritimati | 256 passed each time. |
| `cargo clippy -p mneme-store --all-targets -- -D warnings` | Exit 0, no warnings. |
| `cargo fmt -p mneme-store -- --check` | Exit 0. |
| `cargo doc -p mneme-store --no-deps` | No warnings. |
| `cargo run -p mneme-store --release --example list_bench` (and with `2000`) | Table above. |

Acceptance criteria:

1. **List and find tests** (`tests/listing.rs`).
   - Ordering by date, not folder name: `meetings_are_ordered_by_date_not_by_folder_name` (folders renamed so that names sort against the dates, then a date edited by hand), `a_meeting_recorded_in_another_time_zone_is_placed_by_when_it_happened` (three offsets and a date-line case, where folder name, wall clock and instant give three different orders), `five_hundred_meetings_are_listed_in_order`.
   - Same-minute pair and the tie-break: `two_meetings_in_one_minute_are_both_listed_by_their_seconds` (also with the later meeting in the plain folder and the earlier one in `… Meeting 2`), `meetings_with_the_same_date_are_ordered_by_folder_name_last_first`.
   - Junk skipped with warnings: `junk_in_the_root_is_skipped_with_a_warning_for_each_entry` (two plain files, an empty folder, a folder without `notes.md`, binary `notes.md`, `notes.md` that is a directory, a link to a real meeting, five kinds of broken frontmatter, `.DS_Store`, `.obsidian`; each warning's path, reason and text; nothing on disk changed), `a_hidden_folder_that_holds_a_meeting_is_still_a_meeting`, `folders_without_an_id_are_not_duplicates_of_each_other`, `a_root_that_cannot_be_read_is_the_one_thing_that_fails_the_list`.
   - Find by id after an external rename: `a_meeting_is_found_by_id_after_its_folder_is_renamed_outside` (by a store that knew the old name; then renamed again with another meeting's folder moved onto the old name; then deleted), `a_meeting_that_appears_from_outside_is_found_without_a_list`, `an_id_changed_in_the_file_is_what_the_meeting_is_found_by`, `a_folder_renamed_by_the_store_is_found_by_the_same_store_and_by_others`.
   - Duplicate ids reported, not merged: `duplicate_ids_are_reported_not_merged` (ten by-id calls refused, nothing written, list and warning text, works again once a copy is removed), `a_copy_made_while_the_app_runs_never_takes_over_and_is_caught_by_the_next_list`, `a_duplicate_with_broken_frontmatter_still_makes_the_id_ambiguous`.
   - Also: `loading_a_meeting_gives_its_summary_notes_and_line_count`, `loading_fails_the_way_reading_a_meeting_fails`, `edits_made_outside_show_up_on_the_next_list_and_load`, `the_list_holds_what_reading_each_meeting_returns`, `a_meeting_the_store_knows_is_found_without_reading_the_whole_root` (with the root made unlistable, calls by id still work for folders the store learned from a list, from a lookup and from creating them), `listing_while_other_threads_rename_and_create_never_sees_half_a_change`.
2. **Recovery tests** (`tests/recovery.rs`).
   - Out-of-order lines and a truncated last line: `a_meeting_left_recording_becomes_interrupted_with_a_sorted_transcript_and_its_duration` (exact transcript, exact `notes.md` with only `duration` and `status` changed, a hand-added key, the body and the timeline untouched; duration 310 from the sort's `last_timestamp_ms`), `a_cut_off_last_line_that_still_reads_as_a_line_counts_toward_the_duration` (duration taken from what the sort returns: 555), `the_duration_is_the_latest_timestamp_whatever_order_the_lines_arrived_in`, `a_meeting_with_no_transcript_lines_gets_duration_zero` (empty, junk only, and missing transcript).
   - Twice changes nothing: `recovering_twice_changes_nothing_the_second_time` (same store and a fresh one; contents and inodes equal), `a_launch_with_nothing_left_open_has_nothing_to_do`.
   - A real kill: `a_killed_process_leaves_a_meeting_that_the_next_launch_recovers` starts the test binary as a child that creates a meeting, types a title, appends five segments and saves notes; the parent kills it with SIGKILL, opens the root, recovers and lists: interrupted, duration 310, transcript sorted, notes and timeline there, folder renamed to the title.
   - Also: only `recording` meetings touched, live meetings never touched (one-thread and four-thread tests), pending rename applied and numbered default folder left alone, blocked rename reported, temp files removed narrowly, unreadable folders reported and left, duplicate folders each repaired, one failing meeting not stopping the others (and repaired on a later launch), a read-only folder failing at the sort and at the status change without losing a byte, a crash between the sort and the status change.
3. **Listing 500 meetings**: 12 ms median in a release build; table above.
4. **Test and clippy commands**: as in the table.

**The tests fail when the behavior is broken.** A script made 57 changes to the source one at a time, ran the unit tests and the two new test files, and put the source back (rewriting the file, so it got a fresh modification time and cargo rebuilt). 55 made at least one test fail: ordering by folder name, oldest first, by the text of `date`, tie-break reversed, no tie-break, no sort; each kind of warning dropped; hidden entries warned about; links followed; warnings unsorted; a missing `notes.md` reported as unreadable; duplicates all listed, not warned about, resolved to the first by name; the listed duplicate chosen without regard to readability; ids of broken folders not indexed; empty ids treated as ids; a hint trusted without reading the folder; hints never used; hints kept for a duplicated id or given to one by a rename; hints not refreshed by a list, by a lookup or by create; load returning the whole file, counting raw lines, failing on a missing transcript; no sort in recovery; duration in milliseconds, from the line count, one second for an empty transcript; closed as `complete`; meetings recovered whatever their status; created meetings recovered or not remembered; a failed sort ignored; the first failure stopping recovery; failures not reported; temp files not removed, removed too broadly, reported but not deleted; a blocked rename not reported; recovery order reversed; duplicates and scan warnings missing from the report; `nothing_to_do` ignoring failures; list without the store lock and recover without the store lock (each run 4 times, caught 4 times).

The two that no test catches, and why:
- Rounding the duration up instead of down. Timestamps in a transcript are whole seconds, so the two give the same number for every possible file.
- Not moving the hint when the store renames a folder. The stale hint fails its check and the next lookup scans, so the result is the same and only one lookup is slower.

### Not verified

- **A cold first scan.** Every timing is with the files in the operating system's cache. The first list after a reboot reads 500 small files from disk and will be slower; clearing the cache needs administrator rights, so it was not measured.
- **A quiet machine.** Other agents were building during the measurements (load figures above).
- **A real forced quit of the app.** The SIGKILL test is the closest stand-in; the manual check is in 021.
- **Durability across a power cut.** As in 010 to 012, this rests on the sync calls being there.
- **Two processes on one root.** Not tested; recovery in a second copy of the app would close the first copy's recording. See follow-ups.
- **A folder renamed only in capitals on a disk that ignores case.** A hint keeps the old spelling of the path, which still opens the folder, so `MeetingSummary::folder` can show the old capitals until the next scan. Nothing is read from or written to the wrong place. Not tested.
- **The folder-during-sync reasoning** for not deleting empty folders is a judgment about sync services, not something that was reproduced.
- **The `#[cfg(not(unix))]` branches**, as before. The permission tests return early when run as root.

### Deviations from the issue and PRD

- **The summary has more than the issue lists.** `list_meetings` returns the existing `MeetingSummary` (also `title_edited`, `model`, `language`), not a new narrower type.
- **Hidden entries give no warning.** The issue says to report skipped folders. `.DS_Store` and `.obsidian` would then be reported on every list on most Macs, so hidden files, and hidden folders without a `notes.md`, are passed over in silence.
- **A duplicated id is an error for calls by id**, where 010 picked the first folder by name. `Error` is `#[non_exhaustive]`, so the new variant is not a breaking change for callers that match on it.
- **Recovery also deletes temp files** of the meetings it recovers, which the issue does not ask for (the 010 and 012 follow-ups suggested it).
- **A meeting that cannot be repaired stays `recording`** rather than being marked `interrupted` with an unsorted transcript, so that a later launch can finish the job.
- **`Store` keeps two things in memory** (folder hints, and the ids it created), where 010 said it keeps nothing. Neither is meeting data and neither is written to disk.
- **The two `store::tests::closing_an_interrupted_meeting_…` unit tests** from 026 were left in place; `tests/recovery.rs` covers the same ground through `recover`.

### Follow-ups

- **For 021 (launch).** Open one `Store` for the root and keep it in app state for the life of the process; call `store.recover()` once before serving the first list. Log `report.recovered`, `report.failed` and `report.warnings` (each warning prints as one line). A meeting in `failed` is still `recording` on disk although nothing is recording it: show it as interrupted in the UI, or at least do not show it as live. If the meetings root changes in settings, open a new `Store` and run `recover()` on it.
- **For 021 (one copy of the app).** Recovery assumes one running copy per root. A second copy started during a recording would mark that recording `interrupted` and sort its transcript under the first copy. Use Tauri's single-instance plugin or an equivalent lock. The same goes for a root synced between two Macs: a meeting being recorded on one looks like a crashed meeting to the other at launch. That needs a product decision if synced roots are to be supported.
- **For 021 (commands).** `listMeetings` is `store.list_meetings()?.meetings` mapped as in 010's follow-up; log the warnings, the contract has no place for them. `getMeeting` is `store.load_meeting(&id)`: `notes`, `transcript_line_count`, and `summary.folder` for `folderPath`; it replaces three separate calls. `Error::DuplicateId` should reach the user as its message, which names both folders ("meeting id … is used by 2 folders: "A", "A copy""); the fix is theirs to make. A stop that fails with `DuplicateId` (the live folder was copied mid-meeting and a list ran) leaves the meeting `recording`; the next launch repairs it.
- **For 021 (performance).** With the hints, `save_notes` no longer scans, so autosave cost does not grow with the number of meetings. `list_meetings` holds the store lock for the scan (12 ms per 500 meetings), which delays a transcript append or a notes save by at most that.
- **For 014.** Nothing in this issue touches settings. `safe_write` is unchanged. If the meetings root in settings is ever changed while the app runs, that is a new `Store` (see above), not a method on the old one. `is_temp_file_of(name, "settings.json")` is there (crate-internal) if 014 wants to clean up its own temp file.
- **Empty folders left by a crash during create** are reported on every list and never deleted. If that turns out to be noise, the place to clean them is recovery, with a rule tight enough to be sure the folder is Mneme's (default folder name, nothing inside but empty `transcript.md` and `notes-timeline.jsonl`), and a decision about synced roots first.
- **Detecting a copied folder sooner.** A duplicate made while the app runs is noticed at the next list, not at the next save. Comparing the root folder's modification time before trusting a hint would notice it at once on APFS; it was left out because it is unreliable on file systems with coarse timestamps unless the clock is consulted.
- **PRD 04, Listing and recovery.** Could say that duplicate ids are an error, that hidden entries are ignored, and that a meeting whose repair fails stays `recording` until a later launch. The PRD is outside this lane.
- **`issues/README.md`** lists 013 as `in progress`; the table is outside this lane.
