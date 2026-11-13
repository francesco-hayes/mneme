//! The transcript file through the public API, in temporary folders.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mneme_core::{Segment, Speaker};
use mneme_store::{
    Error, MeetingId, MeetingStatus, MeetingSummary, NewMeeting, Store, Transcript, TranscriptLine,
    TranscriptStats, TranscriptWriter, count_transcript_lines, format_transcript_line,
    is_temp_file_name,
};
use tempfile::TempDir;

use Speaker::{Me, Them};

/// A store in a fresh temp folder. Keep the `TempDir` alive for the test.
fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    (dir, store)
}

fn create(store: &Store) -> MeetingSummary {
    store
        .create_meeting(NewMeeting {
            started_at: "2026-10-27T14:30:00+01:00".parse().unwrap(),
            model: Some("large-v3-turbo".to_owned()),
            language: Some("auto".to_owned()),
        })
        .unwrap()
}

/// A meeting that is recording, with a writer on its transcript.
fn recording() -> (TempDir, Store, MeetingSummary, TranscriptWriter) {
    let (dir, store) = store();
    let meeting = create(&store);
    let writer = store.transcript_writer(&meeting.id).unwrap();
    (dir, store, meeting, writer)
}

fn segment(speaker: Speaker, start_ms: u64, text: &str) -> Segment {
    Segment {
        speaker,
        start_ms,
        end_ms: start_ms + 2_000,
        text: text.to_owned(),
    }
}

fn line(timestamp_ms: u64, speaker: Speaker, text: &str) -> TranscriptLine {
    TranscriptLine {
        timestamp_ms,
        speaker,
        text: text.to_owned(),
    }
}

fn text_of(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
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

// --- format, append, parse --------------------------------------------------

#[test]
fn append_writes_one_strict_line_per_segment() {
    let (_dir, _store, meeting, mut writer) = recording();

    assert!(
        writer
            .append(&segment(Them, 83_000, "Let's start."))
            .unwrap()
    );
    assert!(
        writer
            .append(&segment(Me, 87_900, "Sure, one second."))
            .unwrap()
    );

    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:01:23] Them: Let's start.\n[00:01:27] Me: Sure, one second.\n"
    );
    assert_eq!(writer.meeting_id(), &meeting.id);
    assert_eq!(writer.path(), meeting.transcript_path());
}

#[test]
fn every_line_is_in_the_file_when_append_returns() {
    let (_dir, _store, meeting, mut writer) = recording();
    let path = meeting.transcript_path();

    for n in 1..=5u64 {
        writer.append(&segment(Me, n * 1_000, "word")).unwrap();
        // Read through a different handle, with the writer still open.
        assert_eq!(text_of(&path).lines().count(), n as usize);
        assert!(text_of(&path).ends_with('\n'));
    }
}

#[test]
fn appended_segments_read_back_as_the_same_values() {
    let (_dir, store, meeting, mut writer) = recording();
    let texts = [
        "plain words",
        "[00:00:01] Me: a line inside a line",
        "] Them: starts like the end of a timestamp",
        "note: colons: everywhere",
        "[brackets] [more [nested]] ]] [[",
        "it is 10:30:00, the ratio is 3:1",
        "Me: Them: Me:",
        "ünïcödé, 日本語, emoji 🎙️",
    ];
    let mut expected = Vec::new();
    for (n, text) in texts.iter().enumerate() {
        let speaker = if n % 2 == 0 { Me } else { Them };
        let start_ms = n as u64 * 61_000 + 250;
        assert!(writer.append(&segment(speaker, start_ms, text)).unwrap());
        expected.push(line(n as u64 * 61_000, speaker, text));
    }

    let transcript = store.transcript(&meeting.id).unwrap();

    assert_eq!(transcript.lines, expected);
    assert_eq!(transcript.skipped, 0);
    assert_eq!(transcript.lines[1].timestamp(), "00:01:01");
    // The same answer from the path, and from parsing the bytes.
    let path = meeting.transcript_path();
    assert_eq!(Transcript::read(&path).unwrap(), transcript);
    assert_eq!(Transcript::parse(&fs::read(&path).unwrap()), transcript);
}

#[test]
fn text_with_line_breaks_is_written_as_one_line() {
    let (_dir, store, meeting, mut writer) = recording();

    writer
        .append(&segment(
            Them,
            5_000,
            "  first line\nsecond line\r\n\r\n\tthird    line \u{2028}fourth  ",
        ))
        .unwrap();
    writer.append(&segment(Me, 7_000, "next")).unwrap();

    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:05] Them: first line second line third line fourth\n[00:00:07] Me: next\n"
    );
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.lines.len(), 2);
    assert_eq!(
        transcript.lines[0].text,
        "first line second line third line fourth"
    );
}

#[test]
fn a_segment_with_no_text_is_not_written() {
    let (_dir, store, meeting, mut writer) = recording();

    assert!(!writer.append(&segment(Me, 1_000, "")).unwrap());
    assert!(!writer.append(&segment(Them, 2_000, " \n\t ")).unwrap());
    assert_eq!(format_transcript_line(&segment(Me, 1_000, " \n ")), None);

    assert_eq!(text_of(&meeting.transcript_path()), "");
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 0);
}

// --- finalize ---------------------------------------------------------------

#[test]
fn finalize_sorts_lines_that_arrived_out_of_order() {
    let (_dir, store, meeting, mut writer) = recording();
    // Two channels through one worker: Them's chunk from 0:20 is transcribed
    // before Me's chunk from 0:05.
    writer
        .append(&segment(Them, 20_000, "them, later"))
        .unwrap();
    writer.append(&segment(Me, 5_000, "me, earlier")).unwrap();
    writer.append(&segment(Me, 31_000, "me, last")).unwrap();
    writer.append(&segment(Them, 2_000, "them, first")).unwrap();
    // More than 99 hours in: sorted by time, not as text.
    writer
        .append(&segment(Me, 100 * 3_600_000, "far later"))
        .unwrap();
    writer
        .append(&segment(Me, 99 * 3_600_000, "far, but before"))
        .unwrap();

    let stats = writer.finalize().unwrap();

    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:02] Them: them, first\n\
         [00:00:05] Me: me, earlier\n\
         [00:00:20] Them: them, later\n\
         [00:00:31] Me: me, last\n\
         [99:00:00] Me: far, but before\n\
         [100:00:00] Me: far later\n"
    );
    assert_eq!(
        stats,
        TranscriptStats {
            lines: 6,
            malformed: 0,
            last_timestamp_ms: Some(100 * 3_600_000),
        }
    );
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.last_timestamp_ms(), stats.last_timestamp_ms);
    assert!(transcript.lines.is_sorted_by_key(|line| line.timestamp_ms));
    // Only the three files of a meeting are left: no temp file.
    let names = names_in(&meeting.folder);
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(!names.iter().any(|name| is_temp_file_name(name)));
}

#[test]
fn lines_with_the_same_timestamp_keep_the_order_they_arrived_in() {
    let (_dir, _store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 9_000, "after")).unwrap();
    // The same second. The file holds whole seconds, so these are equal
    // whatever their milliseconds say, and arrival order decides.
    writer.append(&segment(Me, 4_900, "arrived 1")).unwrap();
    writer.append(&segment(Them, 4_100, "arrived 2")).unwrap();
    writer.append(&segment(Me, 4_500, "arrived 3")).unwrap();
    writer.append(&segment(Them, 4_000, "arrived 4")).unwrap();
    writer.append(&segment(Me, 1_000, "before")).unwrap();

    writer.finalize().unwrap();

    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:01] Me: before\n\
         [00:00:04] Me: arrived 1\n\
         [00:00:04] Them: arrived 2\n\
         [00:00:04] Me: arrived 3\n\
         [00:00:04] Them: arrived 4\n\
         [00:00:09] Them: after\n"
    );
}

#[test]
fn finalize_keeps_malformed_lines_at_the_end() {
    let (_dir, store, meeting, mut writer) = recording();
    let path = meeting.transcript_path();
    fs::write(
        &path,
        "# typed by hand\n[00:00:30] Them: late\n\nnot a line\n[00:00:10] Me: early\n",
    )
    .unwrap();
    writer.append(&segment(Me, 20_000, "middle")).unwrap();

    let stats = writer.finalize().unwrap();

    assert_eq!(
        text_of(&path),
        "[00:00:10] Me: early\n\
         [00:00:20] Me: middle\n\
         [00:00:30] Them: late\n\
         # typed by hand\n\
         not a line\n"
    );
    assert_eq!((stats.lines, stats.malformed), (3, 2));
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!((transcript.lines.len(), transcript.skipped), (3, 2));
}

#[test]
fn finalize_twice_changes_nothing_the_second_time() {
    let (_dir, store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 20_000, "b")).unwrap();
    writer.append(&segment(Me, 5_000, "a")).unwrap();

    let first = writer.finalize().unwrap();
    let after_first = fs::metadata(meeting.transcript_path()).unwrap();
    let text = text_of(&meeting.transcript_path());
    let second = writer.finalize().unwrap();
    let third = store.finalize_transcript(&meeting.id).unwrap();

    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(text_of(&meeting.transcript_path()), text);
    // Not even rewritten: it is still the same file on disk.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let after_third = fs::metadata(meeting.transcript_path()).unwrap();
        assert_eq!(after_first.ino(), after_third.ino());
    }
    let _ = after_first;
}

#[test]
fn appending_after_finalize_still_works() {
    let (_dir, _store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 20_000, "b")).unwrap();
    writer.append(&segment(Me, 5_000, "a")).unwrap();
    writer.finalize().unwrap();

    // A segment that was still being transcribed when the sort ran.
    writer.append(&segment(Me, 12_000, "late arrival")).unwrap();
    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:05] Me: a\n[00:00:20] Them: b\n[00:00:12] Me: late arrival\n"
    );

    writer.finalize().unwrap();
    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:05] Me: a\n[00:00:12] Me: late arrival\n[00:00:20] Them: b\n"
    );
}

#[test]
fn a_transcript_can_be_finalized_without_a_writer() {
    let (_dir, store, meeting) = {
        let (dir, store) = store();
        let meeting = create(&store);
        (dir, store, meeting)
    };
    // What a crash leaves: out of order, the last line cut off.
    fs::write(
        meeting.transcript_path(),
        "[00:05:00] Them: second\n[00:01:00] Me: first\n[00:07:3",
    )
    .unwrap();

    let stats = store.finalize_transcript(&meeting.id).unwrap();

    assert_eq!(
        stats,
        TranscriptStats {
            lines: 2,
            malformed: 1,
            last_timestamp_ms: Some(300_000),
        }
    );
    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:01:00] Me: first\n[00:05:00] Them: second\n[00:07:3\n"
    );
}

#[test]
fn finalize_of_an_empty_transcript_leaves_it_empty() {
    let (_dir, _store, meeting, mut writer) = recording();

    assert_eq!(writer.finalize().unwrap(), TranscriptStats::default());
    assert_eq!(text_of(&meeting.transcript_path()), "");
}

// --- cut-off lines and the line count ---------------------------------------

#[test]
fn a_cut_off_last_line_is_skipped_and_the_next_append_starts_a_new_line() {
    let (_dir, store, meeting) = {
        let (dir, store) = store();
        let meeting = create(&store);
        (dir, store, meeting)
    };
    let path = meeting.transcript_path();
    // A crash in the middle of the third append.
    fs::write(
        &path,
        "[00:00:01] Me: one\n[00:00:02] Them: two\n[00:00:03] Me: thr",
    )
    .unwrap();

    let before = store.transcript(&meeting.id).unwrap();
    assert_eq!(
        before.lines,
        [line(1_000, Me, "one"), line(2_000, Them, "two")]
    );
    assert_eq!(before.skipped, 1);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 2);

    // The app starts again and the meeting goes on.
    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    // Opening a writer does not touch the file.
    assert!(text_of(&path).ends_with("Me: thr"));
    writer.append(&segment(Them, 4_000, "four")).unwrap();
    writer.append(&segment(Me, 5_000, "five")).unwrap();

    assert_eq!(
        text_of(&path),
        "[00:00:01] Me: one\n\
         [00:00:02] Them: two\n\
         [00:00:03] Me: thr\n\
         [00:00:04] Them: four\n\
         [00:00:05] Me: five\n"
    );
    // The new lines are whole. What was left of the cut line has its newline
    // now, and since it still reads as a line it is kept, short as it is.
    let after = store.transcript(&meeting.id).unwrap();
    assert_eq!(
        after.lines,
        [
            line(1_000, Me, "one"),
            line(2_000, Them, "two"),
            line(3_000, Me, "thr"),
            line(4_000, Them, "four"),
            line(5_000, Me, "five"),
        ]
    );
    assert_eq!(after.skipped, 0);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 5);
}

#[test]
fn a_line_cut_off_inside_its_timestamp_stays_out_of_the_transcript() {
    let (_dir, store, meeting) = {
        let (dir, store) = store();
        let meeting = create(&store);
        (dir, store, meeting)
    };
    let path = meeting.transcript_path();
    fs::write(&path, "[00:00:01] Me: one\n[00:00:0").unwrap();

    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    writer.append(&segment(Them, 4_000, "four")).unwrap();

    assert_eq!(
        text_of(&path),
        "[00:00:01] Me: one\n[00:00:0\n[00:00:04] Them: four\n"
    );
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(
        transcript.lines,
        [line(1_000, Me, "one"), line(4_000, Them, "four")]
    );
    assert_eq!(transcript.skipped, 1);
}

#[test]
fn the_line_count_matches_the_parsed_lines() {
    let (_dir, store, meeting, mut writer) = recording();
    let path = meeting.transcript_path();
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 0);

    for n in 0..40u64 {
        let speaker = if n % 3 == 0 { Them } else { Me };
        writer
            .append(&segment(speaker, (40 - n) * 3_000, "words"))
            .unwrap();
        assert_eq!(
            store.transcript_line_count(&meeting.id).unwrap(),
            n as usize + 1
        );
    }
    writer.finalize().unwrap();
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 40);

    // Malformed lines, blank lines and a cut-off last line are not counted,
    // by the count or by the parser.
    let mut text = text_of(&path);
    text.insert_str(0, "# heading\n\n");
    text.push_str("junk\n[00:00:99] Me: bad seconds\n[00:09:00] Them: cut o");
    fs::write(&path, &text).unwrap();

    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.lines.len(), 40);
    assert_eq!(transcript.skipped, 4);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 40);
    assert_eq!(count_transcript_lines(&path).unwrap(), 40);
}

#[test]
fn reading_never_fails_on_a_file_that_is_not_a_transcript() {
    let (_dir, store, meeting) = {
        let (dir, store) = store();
        let meeting = create(&store);
        (dir, store, meeting)
    };
    let path = meeting.transcript_path();

    fs::write(
        &path,
        b"\xff\xfe\x00binary\x00\n\x80\n[00:00:01] Me: ok\n\xf0\x9f",
    )
    .unwrap();
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.lines, [line(1_000, Me, "ok")]);
    assert_eq!(transcript.skipped, 3);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 1);

    // Deleted by hand: an empty transcript, not an error.
    fs::remove_file(&path).unwrap();
    assert_eq!(
        store.transcript(&meeting.id).unwrap(),
        Transcript::default()
    );
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 0);
    assert_eq!(
        store.finalize_transcript(&meeting.id).unwrap(),
        TranscriptStats::default()
    );
    assert!(!path.exists());
}

#[test]
fn an_unknown_meeting_is_not_found() {
    let (_dir, store) = store();
    create(&store);
    let id = MeetingId::new("no-such-meeting");

    assert!(matches!(
        store.transcript_writer(&id),
        Err(Error::MeetingNotFound { .. })
    ));
    assert!(matches!(
        store.transcript(&id),
        Err(Error::MeetingNotFound { .. })
    ));
    assert!(matches!(
        store.transcript_line_count(&id),
        Err(Error::MeetingNotFound { .. })
    ));
    assert!(matches!(
        store.finalize_transcript(&id),
        Err(Error::MeetingNotFound { .. })
    ));
}

// --- crashes ----------------------------------------------------------------

#[test]
fn lines_appended_before_a_crash_are_all_on_disk() {
    let (_dir, store, meeting, mut writer) = recording();
    let mut expected = String::new();
    for n in 0..25u64 {
        // Out of order, as two channels would deliver them.
        let start_ms = if n % 2 == 0 {
            n * 4_000
        } else {
            (n * 4_000).saturating_sub(6_000)
        };
        let segment = segment(
            if n % 2 == 0 { Me } else { Them },
            start_ms,
            "said something",
        );
        writer.append(&segment).unwrap();
        expected.push_str(&format_transcript_line(&segment).unwrap());
        expected.push('\n');
    }

    // The crash: the writer goes away with no finalize and no cleanup of any
    // kind, not even its destructor.
    std::mem::forget(writer);

    assert_eq!(text_of(&meeting.transcript_path()), expected);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 25);

    // The same when it is dropped normally.
    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    writer.append(&segment(Me, 200_000, "one more")).unwrap();
    drop(writer);
    expected.push_str("[00:03:20] Me: one more\n");
    assert_eq!(text_of(&meeting.transcript_path()), expected);

    // What recovery does next: every line survives the sort.
    let stats = store.finalize_transcript(&meeting.id).unwrap();
    assert_eq!((stats.lines, stats.malformed), (26, 0));
    let mut before: Vec<&str> = expected.lines().collect();
    let after_text = text_of(&meeting.transcript_path());
    let mut after: Vec<&str> = after_text.lines().collect();
    before.sort_unstable();
    after.sort_unstable();
    assert_eq!(before, after);
}

const CRASH_ROOT: &str = "MNEME_TEST_CRASH_ROOT";
const CRASH_ID: &str = "MNEME_TEST_CRASH_MEETING";
const CRASH_LINES: u64 = 30;

/// Not a test of its own: the body of the process that
/// `lines_survive_a_killed_process` starts and kills. Without the
/// environment variables it does nothing.
#[test]
fn crash_child_appends_and_waits_to_be_killed() {
    let (Some(root), Ok(id)) = (std::env::var_os(CRASH_ROOT), std::env::var(CRASH_ID)) else {
        return;
    };
    let store = Store::open(&root).unwrap();
    let mut writer = store.transcript_writer(&MeetingId::new(id)).unwrap();
    for n in 0..CRASH_LINES {
        writer
            .append(&segment(Them, n * 2_000, "from the child"))
            .unwrap();
    }
    // Tell the parent every append has returned, then hold the writer open
    // until the kill arrives.
    fs::write(Path::new(&root).with_extension("ready"), "").unwrap();
    thread::sleep(Duration::from_secs(60));
    std::process::exit(1);
}

#[cfg(unix)]
#[test]
fn lines_survive_a_killed_process() {
    use std::os::unix::process::ExitStatusExt;

    let (_dir, store) = store();
    let meeting = create(&store);
    let ready = store.root().with_extension("ready");

    // This same test binary, running only the helper above.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child_appends_and_waits_to_be_killed"])
        .args(["--nocapture", "--test-threads=1"])
        .env(CRASH_ROOT, store.root())
        .env(CRASH_ID, meeting.id.as_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let started = Instant::now();
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the child ended on its own: {status}");
        }
        if started.elapsed() > Duration::from_secs(50) {
            let _ = child.kill();
            panic!("the child never finished its appends");
        }
        thread::sleep(Duration::from_millis(10));
    }
    // SIGKILL: no destructors, no flushing, nothing.
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(9), "{status}");

    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.skipped, 0);
    assert_eq!(transcript.lines.len(), CRASH_LINES as usize);
    for (n, got) in transcript.lines.iter().enumerate() {
        assert_eq!(got, &line(n as u64 * 2_000, Them, "from the child"));
    }
}

// --- renamed folders --------------------------------------------------------

/// A recording with a title typed during the meeting, so that finishing it
/// moves the folder, and two lines appended out of order.
fn recording_with_a_pending_rename() -> (TempDir, Store, MeetingSummary, TranscriptWriter) {
    let (dir, store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 20_000, "second")).unwrap();
    writer.append(&segment(Me, 5_000, "first")).unwrap();
    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();
    assert_eq!(renamed.folder, meeting.folder, "no move while recording");
    (dir, store, meeting, writer)
}

const SORTED_TWO: &str = "[00:00:05] Me: first\n[00:00:20] Them: second\n";

#[test]
fn finalize_then_finish_leaves_a_sorted_transcript_in_the_renamed_folder() {
    let (_dir, store, meeting, mut writer) = recording_with_a_pending_rename();

    writer.finalize().unwrap();
    let finished = store.finish_meeting(&meeting.id, 25).unwrap();

    assert!(finished.rename_error.is_none());
    let folder = &finished.meeting.folder;
    assert_ne!(folder, &meeting.folder);
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(text_of(&finished.meeting.transcript_path()), SORTED_TWO);
    assert_eq!(names_in(folder).len(), 3);
}

#[test]
fn finish_then_finalize_sorts_the_transcript_in_the_renamed_folder() {
    let (_dir, store, meeting, mut writer) = recording_with_a_pending_rename();

    let finished = store.finish_meeting(&meeting.id, 25).unwrap();
    assert_ne!(finished.meeting.folder, meeting.folder);
    // The writer was opened on the old folder name.
    assert_eq!(writer.path(), meeting.transcript_path());
    let stats = writer.finalize().unwrap();

    assert_eq!(stats.lines, 2);
    assert_eq!(text_of(&finished.meeting.transcript_path()), SORTED_TWO);
    assert_eq!(writer.path(), finished.meeting.transcript_path());
    // Nothing was put back under the old name.
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(names_in(&finished.meeting.folder).len(), 3);
}

#[test]
fn appends_after_the_folder_was_renamed_land_in_the_renamed_folder() {
    let (_dir, store, meeting, mut writer) = recording_with_a_pending_rename();

    let finished = store.finish_meeting(&meeting.id, 25).unwrap();
    // The backlog is still draining after the meeting was closed.
    writer.append(&segment(Me, 12_000, "late")).unwrap();
    writer.append(&segment(Them, 1_000, "later still")).unwrap();

    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(
        text_of(&finished.meeting.transcript_path()),
        "[00:00:20] Them: second\n\
         [00:00:05] Me: first\n\
         [00:00:12] Me: late\n\
         [00:00:01] Them: later still\n"
    );

    writer.finalize().unwrap();
    assert_eq!(
        text_of(&finished.meeting.transcript_path()),
        "[00:00:01] Them: later still\n\
         [00:00:05] Me: first\n\
         [00:00:12] Me: late\n\
         [00:00:20] Them: second\n"
    );
    assert_eq!(
        store.meeting(&meeting.id).unwrap().status,
        MeetingStatus::Complete
    );
}

#[test]
fn same_minute_meetings_keep_their_transcripts_apart_and_a_writer_follows_its_own() {
    let (_dir, store) = store();
    let first = create(&store);
    let second = create(&store);
    assert_eq!(
        names_in(store.root()),
        ["2026-10-27 1430 Meeting", "2026-10-27 1430 Meeting 2"]
    );
    let mut first_writer = store.transcript_writer(&first.id).unwrap();
    let mut second_writer = store.transcript_writer(&second.id).unwrap();
    assert_eq!(second_writer.path(), second.transcript_path());

    first_writer
        .append(&segment(Me, 1_000, "in the first"))
        .unwrap();
    second_writer
        .append(&segment(Them, 2_000, "in the second"))
        .unwrap();

    assert_eq!(
        text_of(&first.transcript_path()),
        "[00:00:01] Me: in the first\n"
    );
    assert_eq!(
        text_of(&second.transcript_path()),
        "[00:00:02] Them: in the second\n"
    );

    // Stopped with its default title, the numbered folder stays and the
    // writer has nothing to follow.
    let first_done = store.finish_meeting(&first.id, 3).unwrap().meeting;
    first_writer.finalize().unwrap();
    assert_eq!(first_done.folder, first.folder);

    // Given a title, the numbered folder moves at finish to the name
    // without a number, and its writer follows it there.
    store.rename_meeting(&second.id, "Standup").unwrap();
    let second_done = store.finish_meeting(&second.id, 3).unwrap().meeting;
    second_writer.append(&segment(Me, 500, "late")).unwrap();
    second_writer.finalize().unwrap();

    assert_eq!(
        names_in(store.root()),
        ["2026-10-27 1430 Meeting", "2026-10-27 1430 Standup"]
    );
    assert_eq!(second_writer.path(), second_done.transcript_path());
    assert_eq!(
        text_of(&second_done.transcript_path()),
        "[00:00:00] Me: late\n[00:00:02] Them: in the second\n"
    );
    assert_eq!(
        text_of(&first.transcript_path()),
        "[00:00:01] Me: in the first\n"
    );
    assert_eq!(store.transcript(&first.id).unwrap().lines.len(), 1);
    assert_eq!(store.transcript(&second.id).unwrap().lines.len(), 2);
}

#[test]
fn a_folder_renamed_by_hand_during_recording_does_not_lose_lines() {
    let (_dir, store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 20_000, "second")).unwrap();

    let moved = store.root().join("moved in Finder");
    fs::rename(&meeting.folder, &moved).unwrap();
    writer.append(&segment(Me, 5_000, "first")).unwrap();
    writer.finalize().unwrap();

    assert_eq!(names_in(store.root()), ["moved in Finder"]);
    assert_eq!(text_of(&moved.join("transcript.md")), SORTED_TWO);
    assert_eq!(store.transcript_line_count(&meeting.id).unwrap(), 2);
}

#[test]
fn appends_reach_the_file_even_when_the_meeting_cannot_be_found_by_id() {
    let (_dir, store, meeting, mut writer) = recording();
    writer.append(&segment(Them, 20_000, "second")).unwrap();

    // The folder is moved and its notes.md is broken, so no lookup by id can
    // find it. The open file is still the transcript.
    let moved = store.root().join("moved in Finder");
    fs::rename(&meeting.folder, &moved).unwrap();
    fs::write(moved.join("notes.md"), "no frontmatter here\n").unwrap();

    assert!(writer.append(&segment(Me, 5_000, "first")).unwrap());
    assert_eq!(
        text_of(&moved.join("transcript.md")),
        "[00:00:20] Them: second\n[00:00:05] Me: first\n"
    );
    // Sorting needs the path, and there is no way to learn it.
    assert!(matches!(
        writer.finalize(),
        Err(Error::MeetingNotFound { .. })
    ));
    assert_eq!(names_in(store.root()), ["moved in Finder"]);
}

// --- the file changed by another program -------------------------------------

#[test]
fn appends_follow_a_file_that_an_editor_replaced() {
    let (_dir, _store, meeting, mut writer) = recording();
    let path = meeting.transcript_path();
    writer.append(&segment(Me, 1_000, "one")).unwrap();

    // An editor saves the way most do: a new file renamed over the old one.
    // Some keep the old file as a backup, so it does not even go away.
    let edited = meeting.folder.join("transcript.md.new");
    fs::write(&edited, "[00:00:01] Me: one, corrected by hand\n").unwrap();
    fs::hard_link(&path, meeting.folder.join("transcript.md~")).unwrap();
    fs::rename(&edited, &path).unwrap();

    writer.append(&segment(Them, 2_000, "two")).unwrap();

    assert_eq!(
        text_of(&path),
        "[00:00:01] Me: one, corrected by hand\n[00:00:02] Them: two\n"
    );
    assert_eq!(
        text_of(&meeting.folder.join("transcript.md~")),
        "[00:00:01] Me: one\n"
    );
}

#[test]
fn appends_recreate_a_transcript_that_was_deleted() {
    let (_dir, _store, meeting, mut writer) = recording();
    let path = meeting.transcript_path();
    writer.append(&segment(Me, 1_000, "one")).unwrap();

    fs::remove_file(&path).unwrap();
    writer.append(&segment(Them, 2_000, "two")).unwrap();

    assert_eq!(text_of(&path), "[00:00:02] Them: two\n");
}

#[test]
fn appends_fail_once_the_meeting_folder_is_gone() {
    let (_dir, _store, meeting, mut writer) = recording();
    writer.append(&segment(Me, 1_000, "one")).unwrap();

    fs::remove_dir_all(&meeting.folder).unwrap();

    assert!(matches!(
        writer.append(&segment(Them, 2_000, "two")),
        Err(Error::MeetingNotFound { .. })
    ));
    assert!(matches!(
        writer.finalize(),
        Err(Error::MeetingNotFound { .. })
    ));
}

// --- threads ----------------------------------------------------------------

#[test]
fn a_writer_can_be_moved_to_another_thread() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<TranscriptWriter>();
}

#[test]
fn a_worker_thread_appends_while_the_meeting_is_renamed_and_read() {
    let (_dir, store, meeting, mut writer) = recording();
    let store = Arc::new(store);
    const LINES: u64 = 60;

    // The pipeline's worker: the writer lives on that thread, and the store
    // can even be gone by the time it finishes.
    let worker = thread::spawn(move || {
        for n in 0..LINES {
            // Them runs three seconds behind Me, so lines arrive unsorted.
            let (speaker, start_ms) = if n % 2 == 0 {
                (Me, n * 1_000)
            } else {
                (Them, n.saturating_sub(3) * 1_000)
            };
            writer
                .append(&segment(speaker, start_ms, &format!("line {n}")))
                .unwrap();
        }
        writer.finalize().unwrap()
    });

    // Meanwhile the UI thread renames the meeting, reads it, and stops it.
    let ui = {
        let store = Arc::clone(&store);
        let id = meeting.id.clone();
        thread::spawn(move || {
            for n in 0..20 {
                store.rename_meeting(&id, &format!("Title {n}")).unwrap();
                let count = store.transcript_line_count(&id).unwrap();
                let transcript = store.transcript(&id).unwrap();
                // Only whole lines are ever seen, and never fewer than a
                // moment ago.
                assert_eq!(transcript.skipped, 0);
                assert!(transcript.lines.len() >= count);
            }
            store.finish_meeting(&id, 60).unwrap()
        })
    };

    let finished = ui.join().unwrap();
    let stats = worker.join().unwrap();

    assert_eq!(stats.lines, LINES as usize);
    assert_eq!(stats.malformed, 0);
    let now = store.meeting(&meeting.id).unwrap();
    assert_eq!(now.folder, finished.meeting.folder);
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Title 19"]);
    let transcript = store.transcript(&meeting.id).unwrap();
    assert_eq!(transcript.lines.len(), LINES as usize);
    assert!(transcript.lines.is_sorted_by_key(|line| line.timestamp_ms));
    let mut texts: Vec<&str> = transcript
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    texts.sort_unstable();
    texts.dedup();
    assert_eq!(texts.len(), LINES as usize, "every line exactly once");
    assert_eq!(names_in(&now.folder).len(), 3);
}

#[test]
fn two_writers_on_one_transcript_never_tear_or_lose_a_line() {
    let (_dir, store, meeting) = {
        let (dir, store) = store();
        let meeting = create(&store);
        (dir, store, meeting)
    };
    const EACH: u64 = 30;

    let workers: Vec<_> = [Me, Them]
        .into_iter()
        .map(|speaker| {
            let mut writer = store.transcript_writer(&meeting.id).unwrap();
            // One sorts often, the other appends many lines between sorts.
            let sort_every = if speaker == Me { 2 } else { 7 };
            thread::spawn(move || {
                // Counting down, so the file is never in order and every
                // sort rewrites it.
                for n in (0..EACH).rev() {
                    let text = format!("{speaker} says {n} {}", "word ".repeat(40));
                    writer.append(&segment(speaker, n * 1_000, &text)).unwrap();
                    // Sorting in the middle replaces the file under the
                    // other writer, which must notice and follow.
                    if n % sort_every == 0 {
                        writer.finalize().unwrap();
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }

    let stats = store.finalize_transcript(&meeting.id).unwrap();
    assert_eq!((stats.lines, stats.malformed), (2 * EACH as usize, 0));
    let transcript = store.transcript(&meeting.id).unwrap();
    for speaker in [Me, Them] {
        let numbers: Vec<u64> = transcript
            .lines
            .iter()
            .filter(|line| line.speaker == speaker)
            .map(|line| line.timestamp_ms / 1_000)
            .collect();
        assert_eq!(numbers, (0..EACH).collect::<Vec<_>>(), "{speaker}");
    }
    assert_eq!(names_in(&meeting.folder).len(), 3);
}
