//! Crash recovery through the public API, in temporary folders.
//!
//! "A crash" here is a meeting that was created and written to and never
//! finished, followed by a new `Store` on the same root, which is what the
//! app has when it starts again. One test kills a real process instead.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use mneme_core::{Segment, Speaker};
use mneme_store::{
    Error, MeetingId, MeetingStatus, MeetingSummary, NewMeeting, RecoveryReport, SkipReason, Store,
    TranscriptStats,
};
use tempfile::TempDir;

use Speaker::{Me, Them};

/// A store in a fresh temp folder. Keep the `TempDir` alive for the test.
fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    (dir, store)
}

/// The app quit, cleanly or not, and was started again: the old store is
/// gone and a new one opens the same root.
fn relaunch(store: Store) -> Store {
    let root = store.root().to_owned();
    drop(store);
    Store::open(root).unwrap()
}

fn create_at(store: &Store, start: &str) -> MeetingSummary {
    store
        .create_meeting(NewMeeting {
            started_at: start.parse().unwrap(),
            model: Some("large-v3-turbo".to_owned()),
            language: Some("auto".to_owned()),
        })
        .unwrap()
}

/// 2:30 PM in a zone one hour east of UTC.
const START: &str = "2026-10-27T14:30:00+01:00";

fn create(store: &Store) -> MeetingSummary {
    create_at(store, START)
}

fn segment(speaker: Speaker, start_ms: u64, text: &str) -> Segment {
    Segment {
        speaker,
        start_ms,
        end_ms: start_ms + 2_000,
        text: text.to_owned(),
    }
}

/// Appends segments the way a recording does, and then stops without
/// finalizing: no sort, no destructor.
fn record(store: &Store, id: &MeetingId, segments: &[(Speaker, u64, &str)]) {
    let mut writer = store.transcript_writer(id).unwrap();
    for &(speaker, start_ms, text) in segments {
        assert!(writer.append(&segment(speaker, start_ms, text)).unwrap());
    }
    std::mem::forget(writer);
}

/// Puts raw bytes at the end of a file, as a write cut short by a crash
/// leaves them.
fn append_raw(path: &Path, bytes: &str) {
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(bytes.as_bytes()).unwrap();
}

fn text_of(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn name_of(folder: &Path) -> &str {
    folder.file_name().unwrap().to_str().unwrap()
}

/// Names of everything directly under `dir`, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every file under `root` with its contents, to prove nothing changed.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path.clone());
                files.push((path, Vec::new()));
            } else {
                let contents = fs::read(&path).unwrap();
                files.push((path, contents));
            }
        }
    }
    files.sort();
    files
}

/// Which file each path names, to tell "the same bytes" from "not written".
#[cfg(unix)]
fn inodes(root: &Path) -> Vec<(PathBuf, u64)> {
    use std::os::unix::fs::MetadataExt;

    snapshot(root)
        .into_iter()
        .map(|(path, _)| {
            let inode = fs::metadata(&path).unwrap().ino();
            (path, inode)
        })
        .collect()
}

fn recovered_ids(report: &RecoveryReport) -> Vec<&MeetingId> {
    report
        .recovered
        .iter()
        .map(|recovered| &recovered.meeting.id)
        .collect()
}

/// The segments of the meeting most tests crash: out of time order, with
/// two in the same second.
const SEGMENTS: [(Speaker, u64, &str); 5] = [
    (Them, 83_000, "Let's start."),
    (Me, 310_000, "The last whole line."),
    (Me, 5_000, "Hello?"),
    (Them, 87_500, "Same second, arrived first."),
    (Me, 87_100, "Same second, arrived second."),
];

const SORTED: &str = "[00:00:05] Me: Hello?\n\
                      [00:01:23] Them: Let's start.\n\
                      [00:01:27] Them: Same second, arrived first.\n\
                      [00:01:27] Me: Same second, arrived second.\n\
                      [00:05:10] Me: The last whole line.\n";

// --- the repair --------------------------------------------------------------

#[test]
fn a_meeting_left_recording_becomes_interrupted_with_a_sorted_transcript_and_its_duration() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let older = create_at(&store, "2026-10-26T09:00:00+01:00");
    store.finish_meeting(&older.id, 600).unwrap();

    record(&store, &meeting.id, &SEGMENTS);
    store
        .save_notes_at(&meeting.id, "# Agenda\n- ask Ana about the API\n", 90_000)
        .unwrap();
    // The user's own property in the frontmatter, and the line the app was
    // writing when it died: cut off inside its timestamp.
    fs::write(
        meeting.notes_path(),
        text_of(&meeting.notes_path())
            .replace("status: recording\n", "status: recording\ntags: [work]\n"),
    )
    .unwrap();
    append_raw(&meeting.transcript_path(), "[00:07:");
    let notes_before = text_of(&meeting.notes_path());
    let timeline_before = text_of(&meeting.timeline_path());
    assert!(timeline_before.contains("ask Ana"));

    let store = relaunch(store);
    // What the app finds: a meeting that claims to be recording.
    let found = store.meeting(&meeting.id).unwrap();
    assert_eq!(found.status, MeetingStatus::Recording);
    assert_eq!(found.duration_secs, None);

    let report = store.recover().unwrap();

    // One meeting repaired, nothing else to say.
    assert_eq!(recovered_ids(&report), [&meeting.id]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.in_progress.is_empty());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(!report.nothing_to_do());
    let recovered = &report.recovered[0];
    assert!(recovered.rename_error.is_none());
    assert!(recovered.removed_temp_files.is_empty());

    // The transcript: every whole line, in time order, ties in arrival
    // order; the cut-off line kept after them.
    assert_eq!(
        text_of(&meeting.transcript_path()),
        format!("{SORTED}[00:07:\n")
    );
    assert_eq!(
        recovered.transcript,
        TranscriptStats {
            lines: 5,
            malformed: 1,
            last_timestamp_ms: Some(310_000),
        }
    );

    // The duration is the last transcript timestamp, in whole seconds.
    let expected_secs = recovered.transcript.last_timestamp_ms.unwrap() / 1000;
    assert_eq!(expected_secs, 310);
    assert_eq!(recovered.meeting.duration_secs, Some(expected_secs));
    assert_eq!(recovered.meeting.status, MeetingStatus::Interrupted);
    // What the report says is what is on disk.
    assert_eq!(recovered.meeting, store.meeting(&meeting.id).unwrap());

    // In `notes.md` exactly two values changed. The user's key, the body
    // and the timeline are as they were.
    assert_eq!(
        text_of(&meeting.notes_path()),
        notes_before
            .replace("duration:\n", "duration: 310\n")
            .replace("status: recording\n", "status: interrupted\n")
    );
    assert_eq!(text_of(&meeting.timeline_path()), timeline_before);
    // The default-titled folder stays where it is.
    assert_eq!(recovered.meeting.folder, meeting.folder);

    // The meeting is in the list as interrupted, with its transcript.
    let list = store.list_meetings().unwrap();
    assert_eq!(list.meetings.len(), 2);
    assert_eq!(list.meetings[0].id, meeting.id);
    assert_eq!(list.meetings[0].status, MeetingStatus::Interrupted);
    assert_eq!(list.meetings[0].duration_secs, Some(310));
    let loaded = store.load_meeting(&meeting.id).unwrap();
    assert_eq!(loaded.transcript_line_count, 5);
    assert_eq!(loaded.notes, "# Agenda\n- ask Ana about the API\n");
    // The meeting that was finished properly was not touched.
    assert_eq!(list.meetings[1].status, MeetingStatus::Complete);
    assert_eq!(list.meetings[1].duration_secs, Some(600));
}

#[test]
fn a_cut_off_last_line_that_still_reads_as_a_line_counts_toward_the_duration() {
    let (_dir, store) = store();
    let meeting = create(&store);
    record(&store, &meeting.id, &SEGMENTS);
    // Cut off in the middle of its text: what is left is a whole line in
    // every way but the missing newline.
    append_raw(&meeting.transcript_path(), "[00:09:15] Them: and then we");
    let store = relaunch(store);

    let report = store.recover().unwrap();

    let recovered = &report.recovered[0];
    assert_eq!(
        text_of(&meeting.transcript_path()),
        format!("{SORTED}[00:09:15] Them: and then we\n")
    );
    assert_eq!(recovered.transcript.lines, 6);
    assert_eq!(recovered.transcript.malformed, 0);
    // The expected duration comes from what the sort reports.
    let expected_secs = recovered.transcript.last_timestamp_ms.unwrap() / 1000;
    assert_eq!(expected_secs, 9 * 60 + 15);
    assert_eq!(recovered.meeting.duration_secs, Some(expected_secs));
    assert_eq!(
        store.meeting(&meeting.id).unwrap().duration_secs,
        Some(expected_secs)
    );
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 6);
}

#[test]
fn the_duration_is_the_latest_timestamp_whatever_order_the_lines_arrived_in() {
    let (_dir, store) = store();
    let meeting = create(&store);
    // The latest line arrived first; the file ends on an early one.
    record(
        &store,
        &meeting.id,
        &[
            (Them, 3_723_999, "an hour in"),
            (Me, 1_000, "early"),
            (Them, 59_000, "the last to arrive"),
        ],
    );
    let store = relaunch(store);

    let report = store.recover().unwrap();

    // 01:02:03, cut to whole seconds, not rounded up.
    assert_eq!(report.recovered[0].meeting.duration_secs, Some(3_723));
    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:01] Me: early\n\
         [00:00:59] Them: the last to arrive\n\
         [01:02:03] Them: an hour in\n"
    );
}

#[test]
fn a_meeting_with_no_transcript_lines_gets_duration_zero() {
    let (_dir, store) = store();
    let empty = create_at(&store, "2026-10-27T09:00:00+01:00");
    let only_junk = create_at(&store, "2026-10-27T10:00:00+01:00");
    let missing = create_at(&store, "2026-10-27T11:00:00+01:00");
    append_raw(&only_junk.transcript_path(), "not a line\n[00:00:0");
    fs::remove_file(missing.transcript_path()).unwrap();
    let store = relaunch(store);

    let report = store.recover().unwrap();

    assert_eq!(report.recovered.len(), 3);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    for recovered in &report.recovered {
        assert_eq!(recovered.transcript.lines, 0);
        assert_eq!(recovered.transcript.last_timestamp_ms, None);
        assert_eq!(recovered.meeting.duration_secs, Some(0));
        assert_eq!(recovered.meeting.status, MeetingStatus::Interrupted);
        assert!(text_of(&recovered.meeting.notes_path()).contains("\nduration: 0\n"));
    }
    assert_eq!(text_of(&empty.transcript_path()), "");
    assert_eq!(
        text_of(&only_junk.transcript_path()),
        "not a line\n[00:00:0\n"
    );
    // A transcript that is not there is not created.
    assert!(!missing.transcript_path().exists());
}

#[test]
fn recovering_twice_changes_nothing_the_second_time() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let titled = create_at(&store, "2026-10-27T16:00:00+01:00");
    store.rename_meeting(&titled.id, "Standup").unwrap();
    record(&store, &meeting.id, &SEGMENTS);
    record(&store, &titled.id, &SEGMENTS);
    append_raw(&meeting.transcript_path(), "[00:07:");
    fs::write(meeting.folder.join(".notes.md.4242-7.tmp"), "half a save").unwrap();
    let store = relaunch(store);

    let first = store.recover().unwrap();
    assert_eq!(first.recovered.len(), 2);
    let after_first = snapshot(store.root());
    #[cfg(unix)]
    let files_after_first = inodes(store.root());

    for store in [&store, &relaunch(Store::open(store.root()).unwrap())] {
        let again = store.recover().unwrap();

        assert!(again.nothing_to_do());
        assert!(again.recovered.is_empty());
        assert!(again.failed.is_empty());
        assert!(again.in_progress.is_empty());
        assert!(again.warnings.is_empty());
        // Not the same bytes written again: nothing written at all.
        assert_eq!(snapshot(store.root()), after_first);
        #[cfg(unix)]
        assert_eq!(inodes(store.root()), files_after_first);
    }

    // The first pass left both as it reported them.
    assert_eq!(
        names_in(store.root()),
        ["2026-10-27 1430 Meeting", "2026-10-27 1600 Standup"]
    );
    for recovered in &first.recovered {
        assert_eq!(
            store.meeting(&recovered.meeting.id).unwrap(),
            recovered.meeting
        );
        assert_eq!(recovered.meeting.duration_secs, Some(310));
    }
}

#[test]
fn a_launch_with_nothing_left_open_has_nothing_to_do() {
    let (_dir, store) = store();
    // An empty root.
    assert!(store.recover().unwrap().nothing_to_do());

    let meeting = create(&store);
    record(&store, &meeting.id, &SEGMENTS);
    store.finish_meeting(&meeting.id, 400).unwrap();
    let store = relaunch(store);
    let before = snapshot(store.root());

    let report = store.recover().unwrap();

    assert!(report.nothing_to_do());
    assert!(report.warnings.is_empty());
    assert_eq!(snapshot(store.root()), before);
}

// --- what recovery leaves alone ----------------------------------------------

#[test]
fn only_meetings_marked_recording_are_touched() {
    let (_dir, store) = store();
    let complete = create_at(&store, "2026-10-27T09:00:00+01:00");
    let interrupted = create_at(&store, "2026-10-27T10:00:00+01:00");
    let crashed = create_at(&store, "2026-10-27T11:00:00+01:00");
    for meeting in [&complete, &interrupted, &crashed] {
        record(&store, &meeting.id, &SEGMENTS);
        // Leftovers that only a recovered meeting has cleaned up.
        fs::write(meeting.folder.join(".notes.md.4242-7.tmp"), "half a save").unwrap();
    }
    // Finished without a sort, so its transcript is out of order and its
    // duration is not the transcript's.
    store.finish_meeting(&complete.id, 12).unwrap();
    // Recovered by an earlier launch, then edited by hand: an unsorted
    // transcript, a duration of its own, and a title that would move the
    // folder if anything acted on it.
    fs::write(
        interrupted.notes_path(),
        text_of(&interrupted.notes_path())
            .replace("status: recording", "status: interrupted")
            .replace("duration:\n", "duration: 7\n")
            .replace("title_edited: false", "title_edited: true"),
    )
    .unwrap();
    let store = relaunch(store);
    let untouched = |root: &Path| -> Vec<(PathBuf, Vec<u8>)> {
        snapshot(root)
            .into_iter()
            .filter(|(path, _)| !path.starts_with(&crashed.folder))
            .collect()
    };
    let before = untouched(store.root());

    let report = store.recover().unwrap();

    assert_eq!(recovered_ids(&report), [&crashed.id]);
    assert_eq!(untouched(store.root()), before);
    assert_eq!(store.meeting(&complete.id).unwrap().duration_secs, Some(12));
    assert_eq!(
        store.meeting(&interrupted.id).unwrap().duration_secs,
        Some(7)
    );
    assert_eq!(
        report.recovered[0].removed_temp_files,
        [crashed.folder.join(".notes.md.4242-7.tmp")]
    );
}

#[test]
fn a_meeting_this_store_is_recording_is_never_treated_as_a_leftover() {
    let (_dir, store) = store();
    let leftover = create_at(&store, "2026-10-27T09:00:00+01:00");
    record(&store, &leftover.id, &SEGMENTS);
    let store = relaunch(store);

    // The app is running and a meeting is being recorded.
    let live = create(&store);
    let mut writer = store.transcript_writer(&live.id).unwrap();
    writer.append(&segment(Them, 20_000, "second")).unwrap();
    writer.append(&segment(Me, 5_000, "first")).unwrap();
    store.rename_meeting(&live.id, "Standup").unwrap();
    store.save_notes_at(&live.id, "- a note\n", 21_000).unwrap();
    fs::write(live.folder.join(".notes.md.4242-7.tmp"), "half a save").unwrap();
    let live_before: Vec<_> = snapshot(&live.folder);

    // Recovery is called at the wrong time, twice.
    for round in 0..2 {
        let report = store.recover().unwrap();

        // The leftover of the earlier run is repaired, once.
        if round == 0 {
            assert_eq!(recovered_ids(&report), [&leftover.id]);
        } else {
            assert!(report.nothing_to_do());
        }
        // The live meeting is named and left exactly as it is.
        assert_eq!(report.in_progress.len(), 1);
        assert_eq!(report.in_progress[0].id, live.id);
        assert_eq!(report.in_progress[0].status, MeetingStatus::Recording);
        assert_eq!(snapshot(&live.folder), live_before);
    }

    // The recording carries on and ends normally: unsorted file sorted by
    // its own writer, the real duration, the waiting rename applied.
    writer.append(&segment(Them, 30_000, "third")).unwrap();
    writer.finalize().unwrap();
    let finished = store.finish_meeting(&live.id, 45).unwrap();
    assert_eq!(finished.meeting.status, MeetingStatus::Complete);
    assert_eq!(finished.meeting.duration_secs, Some(45));
    assert_eq!(name_of(&finished.meeting.folder), "2026-10-27 1430 Standup");
    assert_eq!(
        text_of(&finished.meeting.transcript_path()),
        "[00:00:05] Me: first\n[00:00:20] Them: second\n[00:00:30] Them: third\n"
    );

    // Had the app died instead, the next launch would have repaired it.
    let crashed_later = create_at(&store, "2026-10-27T17:00:00+01:00");
    assert_eq!(store.recover().unwrap().in_progress.len(), 1);
    let store = relaunch(store);
    let report = store.recover().unwrap();
    assert_eq!(recovered_ids(&report), [&crashed_later.id]);
    assert!(report.in_progress.is_empty());
}

#[test]
fn recovery_called_during_recordings_on_other_threads_never_closes_one() {
    let (_dir, store) = store();
    let done = AtomicBool::new(false);

    thread::scope(|scope| {
        let recorders: Vec<_> = (0..3)
            .map(|thread_no| {
                let store = &store;
                scope.spawn(move || {
                    let mut finished = Vec::new();
                    for n in 0..4 {
                        let meeting =
                            create_at(store, &format!("2026-10-2{thread_no}T1{n}:00:00+01:00"));
                        let mut writer = store.transcript_writer(&meeting.id).unwrap();
                        for line in [3_u64, 1, 2] {
                            writer
                                .append(&segment(Them, line * 1_000, "talking"))
                                .unwrap();
                        }
                        writer.finalize().unwrap();
                        let closed = store.finish_meeting(&meeting.id, 99).unwrap();
                        finished.push(closed.meeting);
                    }
                    finished
                })
            })
            .collect();
        let recoverer = scope.spawn(|| {
            let mut calls = 0;
            while !done.load(Ordering::Relaxed) {
                let report = store.recover().unwrap();
                assert!(report.nothing_to_do(), "{report:?}");
                calls += 1;
            }
            calls
        });

        let finished: Vec<MeetingSummary> = recorders
            .into_iter()
            .flat_map(|recorder| recorder.join().unwrap())
            .collect();
        done.store(true, Ordering::Relaxed);
        assert!(recoverer.join().unwrap() > 0);

        // Every meeting ended the way its recorder ended it.
        assert_eq!(finished.len(), 12);
        for meeting in &finished {
            let on_disk = store.meeting(&meeting.id).unwrap();
            assert_eq!(on_disk.status, MeetingStatus::Complete);
            assert_eq!(on_disk.duration_secs, Some(99));
        }
    });
}

// --- folder renames ----------------------------------------------------------

#[test]
fn a_title_typed_before_the_crash_moves_the_folder_and_a_default_one_stays() {
    let (_dir, store) = store();
    let first = create(&store);
    let numbered = create(&store);
    let titled = create(&store);
    assert_eq!(name_of(&numbered.folder), "2026-10-27 1430 Meeting 2");
    assert_eq!(name_of(&titled.folder), "2026-10-27 1430 Meeting 3");
    store.rename_meeting(&titled.id, "Q3/Q4: review").unwrap();
    record(&store, &titled.id, &SEGMENTS);
    // The plain name is free again; the numbered folder must not take it.
    fs::remove_dir_all(&first.folder).unwrap();
    let store = relaunch(store);

    let report = store.recover().unwrap();

    assert_eq!(recovered_ids(&report), [&numbered.id, &titled.id]);
    assert!(report.recovered.iter().all(|r| r.rename_error.is_none()));
    assert_eq!(
        names_in(store.root()),
        ["2026-10-27 1430 Meeting 2", "2026-10-27 1430 Q3 Q4 review"]
    );
    // The report has the folder as it is now, and so does a lookup.
    let recovered = &report.recovered[1].meeting;
    assert_eq!(name_of(&recovered.folder), "2026-10-27 1430 Q3 Q4 review");
    assert_eq!(recovered.title, "Q3/Q4: review");
    assert_eq!(&store.meeting(&titled.id).unwrap(), recovered);
    assert_eq!(text_of(&recovered.transcript_path()), SORTED);
    assert_eq!(report.recovered[0].meeting.folder, numbered.folder);
    assert!(!report.recovered[0].meeting.title_edited);
}

#[test]
fn a_rename_that_is_blocked_is_reported_and_the_meeting_is_still_recovered() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.rename_meeting(&meeting.id, "Standup").unwrap();
    record(&store, &meeting.id, &SEGMENTS);
    // Something took the name between the crash and the relaunch.
    let squatter = store.root().join("2026-10-27 1430 Standup");
    fs::create_dir(&squatter).unwrap();
    fs::write(squatter.join("keep.txt"), "not ours").unwrap();
    let store = relaunch(store);

    let report = store.recover().unwrap();

    assert!(report.failed.is_empty());
    let recovered = &report.recovered[0];
    assert!(
        matches!(&recovered.rename_error, Some(Error::FolderExists { path }) if *path == squatter),
        "{:?}",
        recovered.rename_error
    );
    assert_eq!(recovered.meeting.folder, meeting.folder);
    assert_eq!(recovered.meeting.status, MeetingStatus::Interrupted);
    assert_eq!(recovered.meeting.duration_secs, Some(310));
    assert_eq!(recovered.meeting.title, "Standup");
    assert_eq!(text_of(&squatter.join("keep.txt")), "not ours");
    assert_eq!(text_of(&meeting.transcript_path()), SORTED);

    // Recovery does not come back to it; a later rename can.
    assert!(store.recover().unwrap().nothing_to_do());
    fs::remove_dir_all(&squatter).unwrap();
    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();
    assert_eq!(renamed.folder, squatter);
}

// --- leftovers ---------------------------------------------------------------

#[test]
fn temp_files_of_interrupted_saves_are_removed_and_nothing_else_is() {
    let (_dir, store) = store();
    let meeting = create(&store);
    record(&store, &meeting.id, &SEGMENTS);
    store.save_notes(&meeting.id, "saved\n").unwrap();
    let folder = &meeting.folder;
    // Ours: the names the safe write helper gives its temp files.
    let ours = [
        ".notes.md.4242-7.tmp",
        ".notes.md.1-0.tmp",
        ".transcript.md.99999-123.tmp",
    ];
    // Not ours, however alike they look.
    let theirs = [
        ".notes.md.tmp",
        ".notes.md.swp",
        ".notes.mdx.1-0.tmp",
        ".other.md.4242-7.tmp",
        ".notes-timeline.jsonl.1-0.tmp",
        "notes.md.4242-7.tmp",
        ".DS_Store",
        "attachment.pdf",
    ];
    for name in ours.iter().chain(&theirs) {
        fs::write(folder.join(name), "a newer save that never landed").unwrap();
    }
    // A folder with one of our names is not a file of ours.
    fs::create_dir(folder.join(".notes.md.5-5.tmp")).unwrap();
    let store = relaunch(store);

    let report = store.recover().unwrap();

    let mut expected: Vec<PathBuf> = ours.iter().map(|name| folder.join(name)).collect();
    expected.sort();
    assert_eq!(report.recovered[0].removed_temp_files, expected);
    let mut left: Vec<&str> = theirs.to_vec();
    left.extend([
        ".notes.md.5-5.tmp",
        "notes.md",
        "transcript.md",
        "notes-timeline.jsonl",
    ]);
    left.sort();
    assert_eq!(names_in(folder), left);
    // The notes are the ones that were saved, not the ones in a temp file.
    assert_eq!(store.notes(&meeting.id).unwrap(), "saved\n");
}

#[test]
fn folders_that_are_not_readable_meetings_are_reported_and_left_alone() {
    let (_dir, store) = store();
    let good = create_at(&store, "2026-10-27T09:00:00+01:00");
    let broken = create_at(&store, "2026-10-27T10:00:00+01:00");
    record(&store, &good.id, &SEGMENTS);
    record(&store, &broken.id, &SEGMENTS);
    // Still says `recording`, but the date cannot be read.
    fs::write(
        broken.notes_path(),
        text_of(&broken.notes_path()).replace("date: \"2026-10-27T10:00:00+01:00\"", "date: soon"),
    )
    .unwrap();
    // A crash between creating a meeting's folder and writing its notes.md.
    let stillborn = store.root().join("2026-10-27 1100 Meeting");
    fs::create_dir(&stillborn).unwrap();
    fs::write(stillborn.join("transcript.md"), "").unwrap();
    fs::write(store.root().join("stray.txt"), "x").unwrap();
    let store = relaunch(store);
    let others = |root: &Path| -> Vec<(PathBuf, Vec<u8>)> {
        snapshot(root)
            .into_iter()
            .filter(|(path, _)| !path.starts_with(&good.folder))
            .collect()
    };
    let before = others(store.root());

    let report = store.recover().unwrap();

    assert_eq!(recovered_ids(&report), [&good.id]);
    assert!(report.failed.is_empty());
    assert_eq!(others(store.root()), before);
    let warned: Vec<&PathBuf> = report
        .warnings
        .iter()
        .map(|warning| &warning.path)
        .collect();
    assert_eq!(
        warned,
        [&broken.folder, &stillborn, &store.root().join("stray.txt")]
    );
    assert!(matches!(
        report.warnings[0].reason,
        SkipReason::Unreadable(Error::Frontmatter { .. })
    ));
    assert!(matches!(report.warnings[1].reason, SkipReason::NoNotesFile));
    assert!(matches!(report.warnings[2].reason, SkipReason::NotAFolder));

    // The same warnings a list gives, and the empty folder is still there
    // for the user to deal with.
    assert_eq!(store.list_meetings().unwrap().warnings.len(), 3);
    assert!(stillborn.is_dir());

    // Once the date is fixed by hand, the next launch repairs that one too.
    fs::write(
        broken.notes_path(),
        text_of(&broken.notes_path()).replace("date: soon", "date: 2026-10-27T10:00:00+01:00"),
    )
    .unwrap();
    let report = relaunch(store).recover().unwrap();
    assert_eq!(recovered_ids(&report), [&broken.id]);
}

#[test]
fn two_folders_with_one_id_are_each_repaired_and_the_id_stays_ambiguous() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.rename_meeting(&meeting.id, "Standup").unwrap();
    record(&store, &meeting.id, &SEGMENTS);
    let copy = store.root().join("A copy made in Finder");
    fs::create_dir(&copy).unwrap();
    for name in names_in(&meeting.folder) {
        fs::copy(meeting.folder.join(&name), copy.join(&name)).unwrap();
    }
    let store = relaunch(store);

    let report = store.recover().unwrap();

    // Both are closed. Only one can have the folder name the title asks
    // for: the first by name gets it, and the other says why it did not.
    assert_eq!(report.recovered.len(), 2);
    assert!(report.failed.is_empty());
    let standup = store.root().join("2026-10-27 1430 Standup");
    assert_eq!(report.recovered[0].meeting.folder, standup);
    assert!(report.recovered[0].rename_error.is_none());
    assert_eq!(report.recovered[1].meeting.folder, copy);
    assert!(matches!(
        report.recovered[1].rename_error,
        Some(Error::FolderExists { .. })
    ));
    assert_eq!(names_in(store.root()), [name_of(&standup), name_of(&copy)]);
    for folder in [&standup, &copy] {
        assert_eq!(text_of(&folder.join("transcript.md")), SORTED);
        assert!(text_of(&folder.join("notes.md")).contains("\nstatus: interrupted\n"));
    }
    // The duplicate is reported, and repairing did not settle which folder
    // the id means.
    assert_eq!(report.warnings.len(), 1);
    assert!(matches!(
        &report.warnings[0].reason,
        SkipReason::DuplicateId { id, .. } if *id == meeting.id
    ));
    let err = store.meeting(&meeting.id).unwrap_err();
    assert!(
        matches!(&err, Error::DuplicateId { folders, .. } if folders.len() == 2),
        "{err:?}"
    );
}

// --- failures ----------------------------------------------------------------

#[test]
fn a_meeting_that_cannot_be_repaired_does_not_stop_the_others() {
    let (_dir, store) = store();
    let before_it = create_at(&store, "2026-10-27T09:00:00+01:00");
    let stuck = create_at(&store, "2026-10-27T10:00:00+01:00");
    let after_it = create_at(&store, "2026-10-27T11:00:00+01:00");
    for meeting in [&before_it, &stuck, &after_it] {
        record(&store, &meeting.id, &SEGMENTS);
    }
    store.save_notes(&stuck.id, "my notes\n").unwrap();
    // The transcript cannot be read as a file: it was replaced by a folder
    // that holds the real one.
    let unsorted = text_of(&stuck.transcript_path());
    fs::remove_file(stuck.transcript_path()).unwrap();
    fs::create_dir(stuck.transcript_path()).unwrap();
    fs::write(stuck.transcript_path().join("moved here.md"), &unsorted).unwrap();
    let store = relaunch(store);
    let stuck_before = snapshot(&stuck.folder);

    let report = store.recover().unwrap();

    // The ones on either side are repaired.
    assert_eq!(recovered_ids(&report), [&before_it.id, &after_it.id]);
    for recovered in &report.recovered {
        assert_eq!(text_of(&recovered.meeting.transcript_path()), SORTED);
        assert_eq!(recovered.meeting.duration_secs, Some(310));
    }
    // The stuck one is reported with the file that is in the way, and is
    // exactly as it was: still recording, nothing lost.
    assert_eq!(report.failed.len(), 1);
    assert!(!report.nothing_to_do());
    let failure = &report.failed[0];
    assert_eq!(failure.meeting.id, stuck.id);
    assert_eq!(failure.meeting.status, MeetingStatus::Recording);
    assert!(
        matches!(&failure.error, Error::Io { path, .. } if *path == stuck.transcript_path()),
        "{:?}",
        failure.error
    );
    assert_eq!(snapshot(&stuck.folder), stuck_before);
    assert_eq!(
        store.meeting(&stuck.id).unwrap().status,
        MeetingStatus::Recording
    );

    // Trying again fails the same way and still harms nothing.
    let report = store.recover().unwrap();
    assert!(report.recovered.is_empty());
    assert_eq!(report.failed.len(), 1);
    assert!(!report.nothing_to_do(), "a failure is something to do");
    assert_eq!(snapshot(&stuck.folder), stuck_before);

    // Once the user puts the file back, the next launch finishes the job.
    fs::remove_dir_all(stuck.transcript_path()).unwrap();
    fs::write(stuck.transcript_path(), &unsorted).unwrap();
    let store = relaunch(store);
    let report = store.recover().unwrap();
    assert_eq!(recovered_ids(&report), [&stuck.id]);
    assert_eq!(text_of(&stuck.transcript_path()), SORTED);
    assert_eq!(store.notes(&stuck.id).unwrap(), "my notes\n");
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_written_keeps_its_meeting_whole_until_it_can() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, store) = store();
    // One whose transcript needs sorting, one whose transcript is already
    // in order: they fail at different steps.
    let unsorted = create_at(&store, "2026-10-27T09:00:00+01:00");
    let in_order = create_at(&store, "2026-10-27T10:00:00+01:00");
    let fine = create_at(&store, "2026-10-27T11:00:00+01:00");
    record(&store, &unsorted.id, &SEGMENTS);
    record(
        &store,
        &in_order.id,
        &[(Me, 1_000, "one"), (Them, 2_000, "two")],
    );
    record(&store, &fine.id, &SEGMENTS);
    let store = relaunch(store);
    let before = [snapshot(&unsorted.folder), snapshot(&in_order.folder)];

    let writable = fs::metadata(&unsorted.folder).unwrap().permissions();
    for meeting in [&unsorted, &in_order] {
        fs::set_permissions(&meeting.folder, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let report = store.recover();
    for meeting in [&unsorted, &in_order] {
        fs::set_permissions(&meeting.folder, writable.clone()).unwrap();
    }
    let report = report.unwrap();

    if report.failed.is_empty() {
        // Running as root: permissions do not apply, nothing to test.
        return;
    }
    assert_eq!(recovered_ids(&report), [&fine.id]);
    let failed: Vec<&MeetingId> = report.failed.iter().map(|f| &f.meeting.id).collect();
    assert_eq!(failed, [&unsorted.id, &in_order.id]);
    for failure in &report.failed {
        assert!(
            matches!(failure.error, Error::Io { .. }),
            "{:?}",
            failure.error
        );
    }
    // Neither lost a byte, neither changed status, and no temp file stayed.
    assert_eq!(
        [snapshot(&unsorted.folder), snapshot(&in_order.folder)],
        before
    );

    // With the folders writable again, the next call repairs both.
    let report = store.recover().unwrap();
    assert_eq!(recovered_ids(&report), [&unsorted.id, &in_order.id]);
    assert!(report.failed.is_empty());
    assert_eq!(text_of(&unsorted.transcript_path()), SORTED);
    assert_eq!(store.meeting(&in_order.id).unwrap().duration_secs, Some(2));
}

#[cfg(unix)]
#[test]
fn a_crash_in_the_middle_of_recovery_is_finished_by_the_next_launch() {
    let (_dir, store) = store();
    let meeting = create(&store);
    record(&store, &meeting.id, &SEGMENTS);
    // Recovery got as far as sorting the transcript and died before it
    // changed the status.
    store.finalize_transcript(&meeting.id).unwrap();
    assert_eq!(text_of(&meeting.transcript_path()), SORTED);
    let store = relaunch(store);
    let transcript_file = inodes(&meeting.folder)
        .into_iter()
        .find(|(path, _)| *path == meeting.transcript_path())
        .unwrap();

    let report = store.recover().unwrap();

    assert_eq!(recovered_ids(&report), [&meeting.id]);
    assert_eq!(report.recovered[0].meeting.duration_secs, Some(310));
    assert_eq!(
        report.recovered[0].meeting.status,
        MeetingStatus::Interrupted
    );
    // The sorted transcript was not written a second time.
    assert!(inodes(&meeting.folder).contains(&transcript_file));
    assert_eq!(text_of(&meeting.transcript_path()), SORTED);
}

// --- a real process, killed --------------------------------------------------

const CRASH_ROOT: &str = "MNEME_RECOVERY_CRASH_ROOT";

/// Not a test of its own: the child process that
/// `a_killed_process_leaves_a_meeting_that_the_next_launch_recovers` starts
/// and kills. Without the environment variable it does nothing.
#[test]
fn crash_child_records_a_meeting_and_waits_to_be_killed() {
    let Some(root) = std::env::var_os(CRASH_ROOT) else {
        return;
    };
    let store = Store::open(&root).unwrap();
    let meeting = create(&store);
    store.rename_meeting(&meeting.id, "Weekly sync").unwrap();
    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    for (speaker, start_ms, text) in SEGMENTS {
        writer.append(&segment(speaker, start_ms, text)).unwrap();
    }
    store
        .save_notes_at(&meeting.id, "- typed before the crash\n", 100_000)
        .unwrap();
    // Tell the parent everything above has returned, then keep the meeting
    // open until the kill arrives.
    fs::write(
        Path::new(&root).with_extension("ready"),
        meeting.id.as_str(),
    )
    .unwrap();
    thread::sleep(Duration::from_secs(60));
    std::process::exit(1);
}

#[cfg(unix)]
#[test]
fn a_killed_process_leaves_a_meeting_that_the_next_launch_recovers() {
    use std::os::unix::process::ExitStatusExt;

    let (_dir, store) = store();
    let ready = store.root().with_extension("ready");

    // This same test binary, running only the helper above.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_child_records_a_meeting_and_waits_to_be_killed",
        ])
        .args(["--nocapture", "--test-threads=1"])
        .env(CRASH_ROOT, store.root())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !ready.exists() || text_of(&ready).is_empty() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the child ended on its own: {status}");
        }
        if started.elapsed() > Duration::from_secs(50) {
            let _ = child.kill();
            panic!("the child never got its meeting going");
        }
        thread::sleep(Duration::from_millis(10));
    }
    // SIGKILL: no destructors, no flushing, no stop.
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9), "{status}");
    let id = MeetingId::new(text_of(&ready));

    // The next launch. First what it finds, then recovery, then the list.
    let store = relaunch(store);
    let found = store.meeting(&id).unwrap();
    assert_eq!(found.status, MeetingStatus::Recording);
    assert_eq!(name_of(&found.folder), "2026-10-27 1430 Meeting");

    let report = store.recover().unwrap();

    assert_eq!(recovered_ids(&report), [&id]);
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.in_progress.is_empty());
    let list = store.list_meetings().unwrap();
    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(list.meetings.len(), 1);
    let meeting = &list.meetings[0];
    assert_eq!(meeting.id, id);
    assert_eq!(meeting.status, MeetingStatus::Interrupted);
    assert_eq!(meeting.duration_secs, Some(310));
    assert_eq!(meeting.title, "Weekly sync");
    assert_eq!(name_of(&meeting.folder), "2026-10-27 1430 Weekly sync");
    assert_eq!(text_of(&meeting.transcript_path()), SORTED);
    let loaded = store.load_meeting(&id).unwrap();
    assert_eq!(loaded.notes, "- typed before the crash\n");
    assert_eq!(loaded.transcript_line_count, 5);
    assert_eq!(
        text_of(&meeting.timeline_path()),
        "{\"t\":\"00:01:40\",\"text\":\"- typed before the crash\"}\n"
    );
    assert!(store.recover().unwrap().nothing_to_do());
}
