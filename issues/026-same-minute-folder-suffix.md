# 026: Folder name suffix for same-minute meetings

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 011 · Owns: `crates/mneme-store/`

## Goal

Starting a second meeting in the same minute as another must never fail. Decided 2026-11-12: the new folder gets a numeric suffix.

## Background

Issue 010 found that start, stop, start again within one minute asks for the same folder name (`2026-10-27 1430 Meeting`), and `create_meeting` returns `FolderExists`. The scope doc had assumed this could not happen.

## Scope

1. **Create**: when the folder name a new meeting wants is taken, use the same name with ` 2` appended, then ` 3`, and so on: `2026-10-27 1430 Meeting 2`. The first meeting keeps the plain name. Creation and the check are atomic with respect to other creates in the same `Store`, and a folder that appears from outside between check and create is handled by trying the next number, not by failing.
2. The suffix belongs to the folder name only. The display title in frontmatter is unchanged (both meetings show "Meeting, Oct 27 2:30 PM").
3. **Later behavior stays consistent** with a suffixed folder:
   - A suffixed, never-retitled folder is not mistaken for a hand-renamed folder, and is not "corrected" back to the plain name by rename, finish or recovery.
   - When the title is edited, the folder moves to the new title's name with no suffix if that name is free.
   - A rename by the user onto a name that is taken still fails with `FolderExists`, as issue 010 built it. The suffix is for automatic names at creation, not for titles the user chose.
4. Decide and document where the suffix sits relative to the length cap from issue 010, so a long title plus a suffix still fits.

## Acceptance criteria

- [x] Tests: two creates in the same minute give `… Meeting` and `… Meeting 2`; a third gives `… Meeting 3`; ids differ; both list as separate meetings with the same display title.
- [x] Test: a gap is reused or skipped by a stated rule (for example, with `Meeting` and `Meeting 3` present, say which name the next one gets) and the rule is in the Result.
- [x] Tests: finish, rename and (if present by then) recovery leave a suffixed default-titled folder where it is; editing the title moves it to the unsuffixed new name; user rename onto a taken name still fails cleanly.
- [x] Test: concurrent creates from several threads in the same minute all succeed with distinct folders.
- [x] Existing tests still pass; `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Implemented 2026-11-12. All work is in `crates/mneme-store/`; nothing outside it was edited except this file. No dependency was added.

### What changed

- **`Store::create_meeting`** (`src/store.rs`) no longer fails on a taken folder name. It calls a new private function, `create_free_folder(root, name)`, which tries `name`, then `name 2`, `name 3` and so on, and returns the first folder it managed to create. The signature and the returned `MeetingSummary` are unchanged; `summary.folder` is the folder that was actually created.
- **No check, so no race.** A name is claimed by `fs::create_dir`, which the file system refuses when anything (folder, file, link) already has the name. A refusal moves on to the next number. Creates on one `Store` still run under the store lock, one at a time. A folder that appears from outside between two attempts costs one more attempt.
- **`MeetingSummary::pending_folder_name(&self) -> Option<String>`** (`src/meeting.rs`, new, public). The one function that answers "is this folder name acceptable for this meeting as it is". `None` means leave the folder alone; `Some(name)` means it should move to `name`. Rename, finish and `Store::close` (which recovery will use) all decide through it: the store's private `rename_target` is now a one-line wrapper around it.
- **`naming.rs`**: `MAX_FOLDER_NUMBER` (999) and `numbered_folder_name(name, n)`, both crate-internal.
- Doc comments on `create_meeting`, `expected_folder_name`, `MeetingSummary::title_edited`, `Error::FolderExists` and the crate layout example now describe numbered folders.

The number is in the folder name only. Nothing writes it to frontmatter, and nothing ever reads it back out of a folder name: there is no parser for it anywhere in the crate. That is what keeps the later behavior consistent:

- Title never edited: `pending_folder_name` is always `None`. The plain `… Meeting`, a numbered `… Meeting 2` and a folder renamed by hand are all left where they are, by rename, by finish and by `close`. They are not told apart, because none of them is ever moved. A numbered folder is not "corrected" even when the plain name has become free.
- Title edited: the folder must have exactly `expected_folder_name()`, which never carries a number. A numbered default folder whose meeting gets a title moves to `… Title`, at once if the meeting is complete, at finish if it is recording.
- A title whose folder name is taken still returns `FolderExists` from `rename_meeting`, during recording too, and changes nothing. Titles the user chose never get a number.

`expected_folder_name()` still returns the plain name (`… Meeting`) for a numbered, default-titled meeting. So comparing it with the real folder name does not tell you whether a rename is waiting. Use `pending_folder_name()` for that.

### Rules

- **Gap rule: lowest free name.** The plain name counts as number 1. With `… Meeting` and `… Meeting 3` present, the next meeting gets `… Meeting 2`; the one after that gets `… Meeting 4`. If the plain name is free (the first meeting was deleted or given a title) the next meeting gets the plain name even though `… Meeting 2` exists. Consequence: within one minute, folder numbers are not a reliable creation order after a deletion or a retitle. The `date` in frontmatter has seconds and is the order to trust.
- **Bound: 999.** After `… Meeting 999` is refused, `create_meeting` returns `Error::FolderExists` with the path of the plain name and creates nothing. A full failed search is 999 refused `mkdir` calls: measured at 1.6 to 3.2 ms on this machine (five runs, debug build). A create that lands on number 999 took 18 ms, against 11 ms for a create on a free plain name.
- **Length cap: the number is outside it.** The cap from 010 (80 characters, 200 bytes) applies to the title part alone. The number is added after the capped title part, is never cut, and the title part is never shortened to make room. The cap leaves that room: 16 bytes of date prefix, 200 of title and 4 for ` 999` come to 220 of the 255 bytes a file name holds. Today only `Meeting` ever gets a number, because a new meeting always has the default title; the rule and its tests cover a long title in case creation with a title is added later.
- **Titles ending in a number.** `Sprint 2` gives `… Sprint 2` and is only ever a title: saving it again changes nothing, retitling to `Sprint` moves the folder to `… Sprint`. A meeting the user really titled `Meeting 2` has `title_edited: true` and the folder `… Meeting 2`; new meetings in that minute go around it (`… Meeting`, then `… Meeting 3`), and unlike a numbered default folder it follows its title when retitled.
- **What takes a name.** Anything at that path: a folder, a plain file, a dangling link. On a disk that ignores case, a name in other capitals too. None of them is touched.

Two consequences of the rules, both intended by the scope but worth knowing:

- In a same-minute pair, the second meeting cannot be titled `Meeting`, or a title that sanitizes to nothing (`???` falls back to `Meeting`), while the first still has `… Meeting`. `rename_meeting` returns `FolderExists`.
- A meeting cannot be given a title that spells out another meeting's numbered folder (`Meeting 2` while `… Meeting 2` exists). Same error.

### Changes to existing tests

Two tests in `tests/meetings.rs` asserted the old behavior, not one. Both were replaced:

- `create_in_a_minute_that_already_has_a_folder_fails_and_touches_nothing` became `creates_in_the_same_minute_get_numbered_folders`. It keeps the old test's check that the first meeting's folder is byte for byte untouched.
- `two_creates_in_the_same_minute_from_two_threads_give_one_meeting` asserted that exactly one of two threads succeeds and the other gets `FolderExists`. It became `concurrent_creates_in_the_same_minute_all_succeed_in_folders_of_their_own`.

The other 140 tests from 010 and 011 are unchanged and pass.

### Verification

Run on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-store` (before any change) | 142 passed: 76 unit, 35 in `tests/meetings.rs`, 31 in `tests/transcript.rs`. |
| `cargo test -p mneme-store` | 163 passed, 0 failed, 0 ignored: 85 unit (9 new), 46 in `tests/meetings.rs` (33 unchanged, 2 replaced, 11 added), 32 in `tests/transcript.rs` (1 new). Run 7 times at the final code, same result each time. |
| `TZ=<zone> cargo test -p mneme-store` for UTC, Asia/Kolkata, America/Los_Angeles, Pacific/Kiritimati | 163 passed each time. |
| `cargo clippy -p mneme-store --all-targets -- -D warnings` | Exit 0, no warnings. |
| `cargo fmt -p mneme-store -- --check` | Exit 0. |
| `cargo doc -p mneme-store --no-deps` | No warnings. |

Acceptance criteria, with the tests that cover them (integration tests in `tests/meetings.rs` unless a module is named):

1. **Same minute, numbered folders, separate meetings.** `creates_in_the_same_minute_get_numbered_folders`: three creates with different seconds in one minute give `… Meeting`, `… Meeting 2`, `… Meeting 3`, each with its three files, the first folder untouched. `same_minute_meetings_are_separate_meetings_with_the_same_title`: three different ids; each `notes.md` is byte for byte the standard new file with only the id differing, so the title is `Meeting, Oct 27 2:30 PM` for all three; each id resolves to its own folder. "Both list as separate meetings": there is no list call yet (013). The test does what a list will do, reading the frontmatter of every folder under the root with a YAML parser, and gets three entries with three ids and one title.
2. **Gap rule.** `a_new_meeting_takes_the_lowest_free_number` (gap filled, then highest plus one, then the plain name as a gap), `names_taken_by_things_that_are_not_meetings_are_skipped_and_left_alone`, unit `store::tests::a_free_folder_is_the_plain_name_then_the_lowest_free_number`.
3. **Later behavior.**
   - Finish: `finish_leaves_a_numbered_folder_where_it_is`, including a finish after the plain name became free.
   - Recovery is not built yet. The call it will make, `Store::close` with `MeetingStatus::Interrupted`, is covered by two unit tests: `store::tests::closing_an_interrupted_meeting_leaves_a_numbered_folder_where_it_is` and `store::tests::closing_an_interrupted_meeting_applies_a_title_typed_before_the_crash`.
   - Title edit: `editing_the_title_moves_a_numbered_folder_to_the_name_without_a_number` (after completion, and during recording then finish).
   - Taken name: `a_title_whose_folder_name_is_taken_is_still_refused_not_numbered` (while recording and after; every file under the root compared before and after), plus 010's `rename_onto_an_existing_folder_fails_and_changes_nothing`, unchanged.
   - Numeric titles: `a_title_that_ends_in_a_number_is_kept_whole`, `a_meeting_titled_like_a_numbered_default_is_treated_as_titled`, unit `meeting::tests::a_title_that_ends_in_a_number_is_a_title_not_a_numbered_default` and `a_folder_only_has_a_pending_name_when_an_edited_title_asks_for_another`.
4. **Concurrent creates.** `concurrent_creates_in_the_same_minute_all_succeed_in_folders_of_their_own`: 8 threads, 3 creates each, one store; 24 meetings, 24 ids, folders exactly `… Meeting` to `… Meeting 24`, each holding the id of the meeting returned for it. `concurrent_creates_through_separate_stores_on_one_root_never_share_a_folder`: 6 stores (so 6 separate locks) on one root, 4 creates each, same checks.
5. **Existing tests, test and clippy commands**: as in the table.

Other tests: bound (`create_gives_up_cleanly_when_every_number_is_taken`, with 999 real folders: error names the plain name, nothing created, another minute unaffected, a freed 999 is found), length cap (`naming::tests::the_longest_title_with_the_highest_number_is_still_a_valid_file_name` creates the longest possible names on disk; `store::tests::a_long_title_part_keeps_its_length_and_gets_the_number_after_it`), other errors are not retried (`store::tests::a_failure_other_than_a_taken_name_is_reported_at_once`), capitals (`a_name_taken_in_other_capitals_counts_as_taken_where_the_disk_says_so`), transcript writers of a same-minute pair stay apart and follow a retitled numbered folder (`tests/transcript.rs`: `same_minute_meetings_keep_their_transcripts_apart_and_a_writer_follows_its_own`).

**The tests fail when the behavior is broken.** Each of these changes was made in turn, the suite run, and the change undone. All but the last made at least one test fail: create fails on a taken name (the old behavior, 17 tests fail); gaps skipped instead of filled; numbering off by one (`Meeting`, `Meeting 3`); a default-titled folder "corrected" to the plain name; an edited title keeping a numbered folder; a trailing number in a title stripped as if it were a suffix; a rename onto a taken name getting a number instead of `FolderExists`; check first and create second (the separate-stores test failed in 4 runs of 4; with the lock removed as well, the one-store test failed too, 4 of 4); no bound; bound of 99; bound of 998; the number written into the title; the error at the bound naming the last name tried; the title part shortened to make room for the number; an existing empty folder reused instead of skipped.

The one change no test catches: taking the store lock out of `create_meeting` while keeping the atomic `create_dir` (4 runs, nothing failed). See below.

### Not verified

- **What the store lock adds to create.** Two creates cannot collide with or without it. The lock is there so that no rename on the same store runs while a new folder is still empty: a folder rename replaces an empty folder that has its target name (checked on this machine), after which create's cleanup would delete the meeting that was moved there. The window is between a rename's last check and its `rename` call, microseconds wide, and no test hits it. The lock is the same one 010 put there.
- **A real list and a real recovery.** Neither exists yet. Listing is stood in for by reading every folder's frontmatter; recovery by calling `Store::close` with `Interrupted`.
- **Two processes.** Separate `Store` values in one process stand in for them. No second process was started.
- **A folder appearing from outside in the exact gap between two attempts.** There is no check to interleave with, so this is the same code path as a name that was taken all along, which is tested. The interleaving itself was not forced.
- **A case-sensitive volume.** The temp folder on this machine ignores case, so the capitals test took that branch only. The `#[cfg(not(unix))]` line in one test was never compiled.
- Timings are from this one machine.

### Deviations from the issue and PRD

- **Create can still fail on names, at the bound.** PRD 04 says creating a meeting never fails on a name clash. It fails when the plain name and all 998 numbered ones are taken. The scope asked for a bounded search; this is where the bound shows.
- **A numbered folder is not distinguished from a hand-renamed one.** The scope says it must not be "mistaken" for one. Here both are left alone by the same rule, so there is nothing to mistake and nothing to parse.

### Follow-ups

- **For 013 (recovery).** Call `Store::close(open, duration_secs, MeetingStatus::Interrupted)` as 010 planned; it applies a waiting rename through `MeetingSummary::pending_folder_name` and so leaves numbered and hand-renamed folders alone. If recovery or listing ever needs to ask directly whether a folder is fine as it is, the call is `summary.pending_folder_name().is_none()`. Do not compare `expected_folder_name()` with the folder's name: for a numbered default folder they differ and nothing is wrong. The two `store::tests::closing_an_interrupted_meeting_…` tests can move into the recovery tests once recovery has a public entry point.
- **For 013 (listing).** Add a list test with a same-minute pair: two entries, same title, different ids. Decide the order for meetings with the same `date` (two starts in one second, or a hand-edited date); folder name is a stable tie-break. A crash between creating the folder and writing `notes.md` leaves an empty `… Meeting` folder that holds the name, so the next meeting in that minute gets a number; the list will report it as a folder with no `notes.md`.
- **For 021.** Starting a meeting no longer needs to handle `FolderExists` as a normal case; it can only come back at the 999 bound, so a generic error message is enough. Two meetings started in one minute show the same title in the Home list ("Meeting, Oct 27 2:30 PM" twice); the frontend may want to tell them apart, for example by showing seconds or the folder name. Retitling still returns `FolderExists` when the name is taken, including the two cases listed under Rules; show it next to the title field. `pending_folder_name()` is there if the UI wants to say that the folder will be renamed at stop.
- **PRD 04**, Folder names: the sentence "Creating a meeting never fails on a name clash" could add "up to number 999". The PRD is outside this lane.
- **Rename and outside programs.** A rename checks that its target is free and then calls `rename`, which replaces an empty folder. Inside one store the lock closes that gap. An empty folder created by another program in that instant would be replaced. This is 010's behavior, unchanged; macOS `renamex_np` with `RENAME_EXCL` would close it if it ever matters.
- **`issues/README.md`** lists 026 as `in progress`; the table is outside this lane.
