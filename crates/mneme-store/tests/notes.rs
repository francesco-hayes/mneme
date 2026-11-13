//! The notes body and the note timeline through the public API, in temporary
//! folders.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use mneme_core::{Segment, Speaker};
use mneme_store::{
    Error, MeetingId, MeetingStatus, MeetingSummary, NewMeeting, Store, is_temp_file_name,
};
use tempfile::TempDir;

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

/// A store with one meeting that is recording.
fn recording() -> (TempDir, Store, MeetingSummary) {
    let (dir, store) = store();
    let meeting = create(&store);
    (dir, store, meeting)
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

/// The timeline as `(t, text)` pairs. Fails the test unless the file is
/// whole lines, each a JSON object with exactly the keys `t` and `text`.
fn timeline(meeting: &MeetingSummary) -> Vec<(String, String)> {
    let text = text_of(&meeting.timeline_path());
    assert!(
        text.is_empty() || text.ends_with('\n'),
        "the timeline does not end in a newline: {text:?}"
    );
    text.split_terminator('\n')
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("not JSON ({err}): {line:?}"));
            let object = value.as_object().unwrap();
            assert_eq!(object.len(), 2, "{line:?}");
            (
                object["t"].as_str().unwrap().to_owned(),
                object["text"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn entry(t: &str, text: &str) -> (String, String) {
    (t.to_owned(), text.to_owned())
}

/// A number that changes when the file at `path` is replaced by a new file,
/// even one with the same bytes. A safe write always replaces the file.
#[cfg(unix)]
fn file_identity(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;

    fs::metadata(path).unwrap().ino()
}

#[cfg(not(unix))]
fn file_identity(_path: &Path) -> u64 {
    0
}

/// A `notes.md` as a person or another tool might leave it: a comment, keys
/// Mneme does not know (a list, a quoted string, a block scalar), odd
/// spacing, a comment after a value.
const HAND_EDITED_FRONTMATTER: &str = "\
---
# edited in Obsidian
id: 0b0e9a3c-1d8f-4f0e-9d5a-2f6f0f1c7e11
tags:
  - work
  - q4
title:   \"Q4: planning\"   # the big one
title_edited: true
aliases: [planning, 'q4 plan']
date: \"2026-10-27T14:30:00+01:00\"
duration:
status: recording
summary: |
  A block scalar
  ---
  with a line that looks like a delimiter, indented.
model: large-v3-turbo
language: \"no\"
future_key: {a: 1}
---
";

/// Puts `text` in place of a meeting's `notes.md` and returns its id.
fn replace_notes_file(meeting: &MeetingSummary, text: &str) -> MeetingId {
    fs::write(meeting.notes_path(), text).unwrap();
    MeetingId::new("0b0e9a3c-1d8f-4f0e-9d5a-2f6f0f1c7e11")
}

// --- reading and saving the body ---------------------------------------------

#[test]
fn a_new_meeting_has_an_empty_body() {
    let (_dir, store, meeting) = recording();

    assert_eq!(store.notes(&meeting.id).unwrap(), "");
}

#[test]
fn a_saved_body_is_read_back_and_sits_right_after_the_frontmatter() {
    let (_dir, store, meeting) = recording();
    let frontmatter = text_of(&meeting.notes_path());

    store
        .save_notes(&meeting.id, "# Standup\n\n- one\n- two\n")
        .unwrap();

    assert_eq!(
        text_of(&meeting.notes_path()),
        format!("{frontmatter}# Standup\n\n- one\n- two\n")
    );
    assert_eq!(
        store.notes(&meeting.id).unwrap(),
        "# Standup\n\n- one\n- two\n"
    );
    // Only the three files: no temp file is left behind.
    assert_eq!(
        names_in(&meeting.folder),
        ["notes-timeline.jsonl", "notes.md", "transcript.md"]
    );
    // The meeting itself is unchanged.
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);
}

#[test]
fn saving_the_body_keeps_the_frontmatter_byte_for_byte() {
    let (_dir, store, meeting) = recording();
    let id = replace_notes_file(&meeting, &format!("{HAND_EDITED_FRONTMATTER}old body\n"));

    store.save_notes(&id, "new body\n").unwrap();
    assert_eq!(
        text_of(&meeting.notes_path()),
        format!("{HAND_EDITED_FRONTMATTER}new body\n")
    );

    // Again with a time, again with nothing, and with the body removed.
    store.save_notes_at(&id, "- a\n- b\n", 3_000).unwrap();
    assert_eq!(
        text_of(&meeting.notes_path()),
        format!("{HAND_EDITED_FRONTMATTER}- a\n- b\n")
    );
    store.save_notes(&id, "").unwrap();
    assert_eq!(text_of(&meeting.notes_path()), HAND_EDITED_FRONTMATTER);
}

#[test]
fn frontmatter_with_windows_line_endings_or_a_byte_order_mark_is_kept() {
    let (_dir, store, meeting) = recording();

    let crlf = HAND_EDITED_FRONTMATTER.replace('\n', "\r\n");
    let id = replace_notes_file(&meeting, &crlf);
    store.save_notes(&id, "unix\nbody\n").unwrap();
    assert_eq!(
        text_of(&meeting.notes_path()),
        format!("{crlf}unix\nbody\n")
    );
    assert_eq!(store.notes(&id).unwrap(), "unix\nbody\n");

    let with_bom = format!("\u{feff}{HAND_EDITED_FRONTMATTER}");
    replace_notes_file(&meeting, &with_bom);
    store.save_notes(&id, "body").unwrap();
    assert_eq!(text_of(&meeting.notes_path()), format!("{with_bom}body"));
}

#[test]
fn a_body_with_delimiter_lines_survives() {
    let (_dir, store, meeting) = recording();
    let frontmatter = text_of(&meeting.notes_path());
    let body = "---\nintro\n---\n\n---\nkey: value\n---\nmore\n----\n--- \n";

    store.save_notes(&meeting.id, body).unwrap();

    assert_eq!(
        text_of(&meeting.notes_path()),
        format!("{frontmatter}{body}")
    );
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);

    // And through frontmatter rewrites, which must not touch the body.
    store.rename_meeting(&meeting.id, "Dashes").unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    let finished = store.finish_meeting(&meeting.id, 60).unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    assert!(text_of(&finished.meeting.notes_path()).ends_with(&format!("---\n{body}")));

    // A body that is nothing but a delimiter, and one that starts like
    // frontmatter of its own.
    for body in ["---", "---\n", "---\nid: other\nstatus: complete\n---\n"] {
        store.save_notes(&meeting.id, body).unwrap();
        assert_eq!(store.notes(&meeting.id).unwrap(), body);
        assert_eq!(store.meeting(&meeting.id).unwrap(), finished.meeting);
    }
}

#[test]
fn the_body_is_stored_exactly_as_given() {
    let (_dir, store, meeting) = recording();
    let frontmatter = text_of(&meeting.notes_path());

    for body in [
        "no final newline",
        "one final newline\n",
        "three final newlines\n\n\n",
        "\n\nblank lines first\n",
        "   spaces around   ",
        "trailing spaces on a line   \nand a tab\t\n",
        "windows\r\nline\r\nendings\r\n",
        "windows\r\nwithout the last one",
        "mixed\r\nline\nendings\rof every kind\n",
        "\r\n",
        "\n",
        " ",
        "ünïcödé, 日本語, emoji 🎙️\n",
        "\u{feff}a byte order mark that is part of the body\n",
        "a nul \0 and a form feed \u{c}\n",
        "",
    ] {
        store.save_notes(&meeting.id, body).unwrap();

        assert_eq!(
            fs::read(meeting.notes_path()).unwrap(),
            format!("{frontmatter}{body}").into_bytes(),
            "{body:?}"
        );
        assert_eq!(store.notes(&meeting.id).unwrap(), body, "{body:?}");
    }
}

#[test]
fn saving_the_body_that_was_read_changes_nothing_on_disk() {
    let (_dir, store, meeting) = recording();

    for file in [
        // As Mneme writes it.
        format!("{HAND_EDITED_FRONTMATTER}- a\n- b\n"),
        // No body, and no newline after the closing delimiter.
        HAND_EDITED_FRONTMATTER.trim_end().to_owned(),
        // A blank line between frontmatter and body, as most editors leave.
        format!("{HAND_EDITED_FRONTMATTER}\n# Title\n\ntext"),
        // Windows line endings throughout.
        format!("{HAND_EDITED_FRONTMATTER}- a\n- b\n").replace('\n', "\r\n"),
        // Spaces after the closing delimiter, a byte order mark.
        format!(
            "\u{feff}{}---  \nbody\n",
            HAND_EDITED_FRONTMATTER.strip_suffix("---\n").unwrap()
        ),
    ] {
        let id = replace_notes_file(&meeting, &file);
        let identity = file_identity(&meeting.notes_path());

        let body = store.notes(&id).unwrap();
        store.save_notes(&id, &body).unwrap();
        let saved = store.save_notes_at(&id, &body, 5_000).unwrap();

        assert_eq!(text_of(&meeting.notes_path()), file, "{file:?}");
        // Not even rewritten with the same bytes: it is still the same file.
        assert_eq!(file_identity(&meeting.notes_path()), identity);
        assert_eq!(saved.timeline_entries, 0);
        assert_eq!(text_of(&meeting.timeline_path()), "");
        assert_eq!(names_in(&meeting.folder).len(), 3);
    }
}

#[test]
fn a_body_can_be_added_to_a_file_that_ends_at_the_closing_delimiter() {
    let (_dir, store, meeting) = recording();
    let no_newline = HAND_EDITED_FRONTMATTER.trim_end();
    let id = replace_notes_file(&meeting, no_newline);

    // The body needs a line of its own; the newline added for that belongs
    // to the delimiter line, not to the body.
    for body in ["text", "\ntext\n"] {
        store.save_notes(&id, body).unwrap();
        assert_eq!(
            text_of(&meeting.notes_path()),
            format!("{HAND_EDITED_FRONTMATTER}{body}")
        );
        assert_eq!(store.notes(&id).unwrap(), body);
    }
}

#[test]
fn updating_frontmatter_never_touches_the_body() {
    let (_dir, store, meeting) = recording();
    let body = "\n# Notes\r\n\n- kept exactly   \n---\nno final newline";
    store.save_notes_at(&meeting.id, body, 1_000).unwrap();

    store.rename_meeting(&meeting.id, "Standup").unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), body);

    let finished = store.finish_meeting(&meeting.id, 90).unwrap().meeting;
    assert_eq!(store.notes(&meeting.id).unwrap(), body);

    let renamed = store.rename_meeting(&meeting.id, "Retro").unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    assert!(text_of(&renamed.notes_path()).ends_with(&format!("---\n{body}")));
    assert_eq!(renamed.status, MeetingStatus::Complete);
    assert_eq!(renamed.duration_secs, finished.duration_secs);
}

#[test]
fn notes_of_an_unknown_meeting_are_not_found_and_nothing_is_created() {
    let (_dir, store, meeting) = recording();
    let unknown = MeetingId::new("no-such-meeting");

    for result in [
        store.notes(&unknown).map(|_| ()),
        store.save_notes(&unknown, "text"),
        store.save_notes_at(&unknown, "text", 1_000).map(|_| ()),
    ] {
        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::MeetingNotFound { id } if *id == unknown),
            "{err:?}"
        );
    }
    assert_eq!(names_in(store.root()).len(), 1);
    assert_eq!(store.notes(&meeting.id).unwrap(), "");
    assert_eq!(text_of(&meeting.timeline_path()), "");
}

#[test]
fn a_save_follows_the_meeting_to_its_renamed_folder() {
    let (_dir, store, meeting) = recording();
    store.rename_meeting(&meeting.id, "Standup").unwrap();
    store
        .save_notes_at(&meeting.id, "- during\n", 2_000)
        .unwrap();

    // Finish moves the folder; a save that was on its way lands in it.
    let finished = store.finish_meeting(&meeting.id, 30).unwrap().meeting;
    assert_ne!(finished.folder, meeting.folder);
    store
        .save_notes(&meeting.id, "- during\n- after\n")
        .unwrap();

    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert!(text_of(&finished.notes_path()).ends_with("---\n- during\n- after\n"));
    assert_eq!(timeline(&finished), [entry("00:00:02", "- during")]);

    // Moved by hand as well.
    let by_hand = store.root().join("moved by hand");
    fs::rename(&finished.folder, &by_hand).unwrap();
    store.save_notes(&meeting.id, "- found\n").unwrap();
    assert_eq!(names_in(store.root()), ["moved by hand"]);
    assert_eq!(store.notes(&meeting.id).unwrap(), "- found\n");
}

// --- the timeline --------------------------------------------------------------

#[test]
fn three_lines_typed_over_three_saves_give_three_entries_with_their_times() {
    let (_dir, store, meeting) = recording();

    let first = store
        .save_notes_at(&meeting.id, "- intro\n", 12_000)
        .unwrap();
    let second = store
        .save_notes_at(&meeting.id, "- intro\n- ask Ana about the API\n", 252_999)
        .unwrap();
    let third = store
        .save_notes_at(
            &meeting.id,
            "- intro\n- ask Ana about the API\n- ship on Friday\n",
            3_725_000,
        )
        .unwrap();

    for saved in [&first, &second, &third] {
        assert_eq!(saved.timeline_entries, 1);
        assert!(saved.timeline_error.is_none());
    }
    // The file itself, byte for byte: the format the PRD gives.
    assert_eq!(
        text_of(&meeting.timeline_path()),
        "{\"t\":\"00:00:12\",\"text\":\"- intro\"}\n\
         {\"t\":\"00:04:12\",\"text\":\"- ask Ana about the API\"}\n\
         {\"t\":\"01:02:05\",\"text\":\"- ship on Friday\"}\n"
    );
    assert_eq!(
        store.notes(&meeting.id).unwrap(),
        "- intro\n- ask Ana about the API\n- ship on Friday\n"
    );
}

#[test]
fn editing_one_line_gives_one_entry_with_the_new_text() {
    let (_dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n", 1_000)
        .unwrap();

    let saved = store
        .save_notes_at(&meeting.id, "- one\n- two, but later\n- three\n", 9_000)
        .unwrap();

    assert_eq!(saved.timeline_entries, 1);
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:01", "- one"),
            entry("00:00:01", "- two"),
            entry("00:00:01", "- three"),
            entry("00:00:09", "- two, but later"),
        ]
    );
}

#[test]
fn reordering_lines_gives_no_entry() {
    let (_dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "# Plan\n- one\n- two\n- three\n", 1_000)
        .unwrap();
    let before = text_of(&meeting.timeline_path());

    for body in [
        "- three\n# Plan\n- one\n- two\n",
        "- two\n- one\n\n\n- three\n# Plan",
        "# Plan\n- one\n- two\n- three\n",
    ] {
        let saved = store.save_notes_at(&meeting.id, body, 20_000).unwrap();
        assert_eq!(saved.timeline_entries, 0, "{body:?}");
        assert_eq!(store.notes(&meeting.id).unwrap(), body);
    }
    assert_eq!(text_of(&meeting.timeline_path()), before);
}

#[test]
fn saving_the_same_body_twice_gives_no_entry_the_second_time() {
    let (_dir, store, meeting) = recording();
    let body = "- one\n- two\n";

    let first = store.save_notes_at(&meeting.id, body, 1_000).unwrap();
    let before = text_of(&meeting.timeline_path());
    let second = store.save_notes_at(&meeting.id, body, 2_000).unwrap();
    let third = store.save_notes_at(&meeting.id, body, 3_000).unwrap();

    assert_eq!(first.timeline_entries, 2);
    assert_eq!(second.timeline_entries, 0);
    assert_eq!(third.timeline_entries, 0);
    assert_eq!(text_of(&meeting.timeline_path()), before);
    assert_eq!(timeline(&meeting).len(), 2);
}

#[test]
fn deleted_lines_blank_lines_and_indentation_give_no_entry() {
    let (_dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n", 1_000)
        .unwrap();
    let before = text_of(&meeting.timeline_path());

    for body in [
        // A line deleted.
        "- one\n- three\n",
        // Blank lines added.
        "\n- one\n\n   \n\t\n- three\n\n",
        // Indented, trailing spaces, Windows line endings.
        "- one\r\n    - three   \r\n",
        // Everything deleted.
        "",
    ] {
        let saved = store.save_notes_at(&meeting.id, body, 30_000).unwrap();
        assert_eq!(saved.timeline_entries, 0, "{body:?}");
    }
    assert_eq!(text_of(&meeting.timeline_path()), before);
}

#[test]
fn saves_while_not_recording_give_no_entry() {
    let (_dir, store, meeting) = recording();

    // A plain save during the recording: no time, so no entry.
    store.save_notes(&meeting.id, "- typed early\n").unwrap();
    assert_eq!(text_of(&meeting.timeline_path()), "");

    store
        .save_notes_at(&meeting.id, "- typed early\n- recorded\n", 4_000)
        .unwrap();
    let during = text_of(&meeting.timeline_path());
    assert_eq!(timeline(&meeting), [entry("00:00:04", "- recorded")]);

    // After the recording stops, neither kind of save adds anything, even
    // when a time is passed: the status on disk decides.
    store.finish_meeting(&meeting.id, 10).unwrap();
    store
        .save_notes(&meeting.id, "- typed early\n- recorded\n- afterwards\n")
        .unwrap();
    let late = store
        .save_notes_at(&meeting.id, "- a save that was on its way\n", 11_000)
        .unwrap();

    assert_eq!(late.timeline_entries, 0);
    assert!(late.timeline_error.is_none());
    assert_eq!(text_of(&meeting.timeline_path()), during);
    // The notes were saved all the same.
    assert_eq!(
        store.notes(&meeting.id).unwrap(),
        "- a save that was on its way\n"
    );
}

#[test]
fn an_interrupted_meeting_gets_no_entries() {
    let (_dir, store, meeting) = recording();
    let interrupted = text_of(&meeting.notes_path())
        .replace("status: recording", "status: interrupted")
        .replace("duration:", "duration: 7");
    fs::write(meeting.notes_path(), interrupted).unwrap();
    assert_eq!(
        store.meeting(&meeting.id).unwrap().status,
        MeetingStatus::Interrupted
    );

    let saved = store.save_notes_at(&meeting.id, "- text\n", 8_000).unwrap();

    assert_eq!(saved.timeline_entries, 0);
    assert_eq!(text_of(&meeting.timeline_path()), "");
    assert_eq!(store.notes(&meeting.id).unwrap(), "- text\n");
}

#[test]
fn every_timeline_line_is_json_and_awkward_text_round_trips() {
    let (_dir, store, meeting) = recording();
    let lines = [
        r#"- she said "ship it" and 'maybe'"#,
        r"- a back\slash, a \n that is not a newline, and a \u0041 that is not an A",
        "- a tab\tinside",
        "- ünïcödé, 日本語, עברית, emoji 🎙️👩‍💻",
        r#"{"t":"00:00:00","text":"looks like an entry"}"#,
        "- control \u{1b} and nul \0 characters",
        "- a line\u{2028}separator",
        "</script> & <b>html</b>",
    ];
    let body = lines.join("\n");

    let saved = store.save_notes_at(&meeting.id, &body, 754_000).unwrap();

    assert_eq!(saved.timeline_entries, lines.len());
    let raw = text_of(&meeting.timeline_path());
    // One line of the file per entry, nothing else.
    assert_eq!(raw.matches('\n').count(), lines.len());
    assert!(!raw.contains('\r'));
    let entries = timeline(&meeting);
    assert_eq!(entries.len(), lines.len());
    for ((t, text), line) in entries.iter().zip(lines) {
        assert_eq!(t, "00:12:34");
        assert_eq!(text, line);
    }
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
}

#[test]
fn the_text_of_an_entry_is_the_line_without_the_whitespace_around_it() {
    let (_dir, store, meeting) = recording();

    store
        .save_notes_at(
            &meeting.id,
            "# Title  \r\n\r\n    - nested item\r\n\ttabbed\t\r\nlast line, no newline  ",
            5_000,
        )
        .unwrap();

    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:05", "# Title"),
            entry("00:00:05", "- nested item"),
            entry("00:00:05", "tabbed"),
            entry("00:00:05", "last line, no newline"),
        ]
    );
}

#[test]
fn a_line_that_grows_between_saves_is_recorded_at_each_save() {
    let (_dir, store, meeting) = recording();

    for (body, time_ms) in [
        ("- intro\n- ask", 60_000),
        ("- intro\n- ask ", 61_000),
        ("- intro\n- ask Ana", 62_000),
        ("- intro\n- ask Ana about the API", 63_000),
        ("- intro\n- ask Ana about the API\n", 64_000),
        ("- intro\n- ask Ana about the API\n- ", 65_000),
    ] {
        store.save_notes_at(&meeting.id, body, time_ms).unwrap();
    }

    // Each state a save saw, once. The space typed at 61 s and the newline
    // at 64 s change no text. The earliest entry that the finished line
    // starts with is when the line was begun.
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:01:00", "- intro"),
            entry("00:01:00", "- ask"),
            entry("00:01:02", "- ask Ana"),
            entry("00:01:03", "- ask Ana about the API"),
            entry("00:01:05", "-"),
        ]
    );
}

#[test]
fn copies_of_a_line_are_counted_not_located() {
    let (_dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "- todo\nA\n- todo\nB\n", 1_000)
        .unwrap();

    // A third copy is a new line.
    let third = store
        .save_notes_at(&meeting.id, "- todo\nA\n- todo\nB\n- todo\n", 2_000)
        .unwrap();
    assert_eq!(third.timeline_entries, 1);

    // The copies moved around, one indented: nothing.
    let moved = store
        .save_notes_at(&meeting.id, "B\n- todo\n- todo\n  - todo\nA\n", 3_000)
        .unwrap();
    assert_eq!(moved.timeline_entries, 0);

    // One copy deleted, then typed again: recorded again.
    store
        .save_notes_at(&meeting.id, "B\n- todo\n- todo\nA\n", 4_000)
        .unwrap();
    let again = store
        .save_notes_at(&meeting.id, "B\n- todo\n- todo\nA\n- todo\n", 5_000)
        .unwrap();
    assert_eq!(again.timeline_entries, 1);

    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:01", "- todo"),
            entry("00:00:01", "A"),
            entry("00:00:01", "- todo"),
            entry("00:00:01", "B"),
            entry("00:00:02", "- todo"),
            entry("00:00:05", "- todo"),
        ]
    );
}

#[test]
fn a_line_edited_back_to_an_earlier_text_is_recorded_again() {
    let (_dir, store, meeting) = recording();

    store
        .save_notes_at(&meeting.id, "- call Ana\n", 1_000)
        .unwrap();
    store
        .save_notes_at(&meeting.id, "- call Bo\n", 2_000)
        .unwrap();
    store
        .save_notes_at(&meeting.id, "- call Ana\n", 3_000)
        .unwrap();

    // The earliest entry for a text is when it was first typed.
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:01", "- call Ana"),
            entry("00:00:02", "- call Bo"),
            entry("00:00:03", "- call Ana"),
        ]
    );
}

#[test]
fn the_timeline_is_only_ever_appended_to() {
    let (_dir, store, meeting) = recording();
    let mut seen = String::new();

    for (body, time_ms) in [
        ("- one\n", 1_000),
        ("- one\n- two\n", 2_000),
        ("- two\n", 3_000),
        ("", 4_000),
        ("- one\n- three\n", 5_000),
        ("- three\n- one\n", 6_000),
    ] {
        store.save_notes_at(&meeting.id, body, time_ms).unwrap();
        let now = text_of(&meeting.timeline_path());
        assert!(now.starts_with(&seen), "{now:?} does not extend {seen:?}");
        seen = now;
    }
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:01", "- one"),
            entry("00:00:02", "- two"),
            entry("00:00:05", "- one"),
            entry("00:00:05", "- three"),
        ]
    );
}

#[test]
fn the_first_save_after_a_restart_logs_only_what_changed() {
    let (dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n", 10_000)
        .unwrap();
    drop(store);

    // A new store on the same root: nothing is remembered but the files.
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    let same = store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n", 50_000)
        .unwrap();
    let one_more = store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n- four\n", 51_000)
        .unwrap();

    assert_eq!(same.timeline_entries, 0);
    assert_eq!(one_more.timeline_entries, 1);
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:10", "- one"),
            entry("00:00:10", "- two"),
            entry("00:00:10", "- three"),
            entry("00:00:51", "- four"),
        ]
    );
}

#[test]
fn notes_edited_in_another_editor_are_the_body_the_next_save_compares_with() {
    let (_dir, store, meeting) = recording();
    store.save_notes_at(&meeting.id, "- one\n", 1_000).unwrap();

    // Another editor adds a line and saves the file.
    let edited = format!("{}- from Obsidian\n", text_of(&meeting.notes_path()));
    fs::write(meeting.notes_path(), edited).unwrap();
    assert_eq!(
        store.notes(&meeting.id).unwrap(),
        "- one\n- from Obsidian\n"
    );

    store
        .save_notes_at(&meeting.id, "- one\n- from Obsidian\n- two\n", 9_000)
        .unwrap();

    assert_eq!(
        timeline(&meeting),
        [entry("00:00:01", "- one"), entry("00:00:09", "- two")]
    );
}

#[test]
fn entries_after_one_cut_off_by_a_crash_start_on_a_fresh_line() {
    let (_dir, store, meeting) = recording();
    store.save_notes_at(&meeting.id, "- one\n", 1_000).unwrap();
    // A crash in the middle of writing the next entry.
    let cut_off = format!(
        "{}{}",
        text_of(&meeting.timeline_path()),
        "{\"t\":\"00:00:02\",\"text\":\"- tw"
    );
    fs::write(meeting.timeline_path(), &cut_off).unwrap();

    store
        .save_notes_at(&meeting.id, "- one\n- two\n- three\n", 3_000)
        .unwrap();

    // The cut-off entry stays as it is, on a line of its own; every other
    // line is a whole entry.
    assert_eq!(
        text_of(&meeting.timeline_path()),
        format!(
            "{cut_off}\n\
             {{\"t\":\"00:00:03\",\"text\":\"- two\"}}\n\
             {{\"t\":\"00:00:03\",\"text\":\"- three\"}}\n"
        )
    );
}

#[test]
fn a_timeline_that_cannot_be_written_never_blocks_the_notes() {
    let (_dir, store, meeting) = recording();
    // Something that cannot be opened for appending in the timeline's place.
    fs::remove_file(meeting.timeline_path()).unwrap();
    fs::create_dir(meeting.timeline_path()).unwrap();

    let saved = store
        .save_notes_at(&meeting.id, "- important\n", 1_000)
        .unwrap();

    assert_eq!(store.notes(&meeting.id).unwrap(), "- important\n");
    assert_eq!(saved.timeline_entries, 0);
    let err = saved.timeline_error.expect("the failure is reported");
    assert!(
        matches!(&err, Error::Io { path, .. } if *path == meeting.timeline_path()),
        "{err:?}"
    );

    // Nothing to log, nothing to report.
    let again = store
        .save_notes_at(&meeting.id, "- important\n", 2_000)
        .unwrap();
    assert!(again.timeline_error.is_none());
}

#[test]
fn a_deleted_timeline_is_created_again() {
    let (_dir, store, meeting) = recording();
    store.save_notes_at(&meeting.id, "- one\n", 1_000).unwrap();
    fs::remove_file(meeting.timeline_path()).unwrap();

    // No entry to write: the file is not created for nothing.
    store
        .save_notes_at(&meeting.id, "- one\n\n", 2_000)
        .unwrap();
    assert!(!meeting.timeline_path().exists());

    store
        .save_notes_at(&meeting.id, "- one\n- two\n", 3_000)
        .unwrap();
    assert_eq!(timeline(&meeting), [entry("00:00:03", "- two")]);
}

#[test]
fn a_large_paste_is_saved_and_logged_in_one_go() {
    let (_dir, store, meeting) = recording();
    store
        .save_notes_at(&meeting.id, "- before\n", 1_000)
        .unwrap();
    let pasted: String = (0..5_000)
        .map(|n| format!("- pasted line number {n} with \"quotes\"\n"))
        .collect();
    let body = format!("- before\n{pasted}");

    let saved = store.save_notes_at(&meeting.id, &body, 600_000).unwrap();

    assert_eq!(saved.timeline_entries, 5_000);
    let entries = timeline(&meeting);
    assert_eq!(entries.len(), 5_001);
    assert_eq!(
        entries[1],
        entry("00:10:00", "- pasted line number 0 with \"quotes\"")
    );
    assert_eq!(
        entries[5_000],
        entry("00:10:00", "- pasted line number 4999 with \"quotes\"")
    );
    assert_eq!(store.notes(&meeting.id).unwrap(), body);

    // One more line after it is one more entry, not another five thousand.
    let next = store
        .save_notes_at(&meeting.id, &format!("{body}- after\n"), 601_000)
        .unwrap();
    assert_eq!(next.timeline_entries, 1);
}

// --- with the rest of the store ------------------------------------------------

#[test]
fn saves_and_renames_from_two_threads_both_survive() {
    let (_dir, store, meeting) = recording();
    let store = Arc::new(store);
    const ROUNDS: usize = 40;

    // One thread retitles the meeting while another saves its notes. Each
    // rewrites notes.md from what it read; without the store lock one would
    // put back the other's old half.
    let renamer = {
        let (store, id) = (Arc::clone(&store), meeting.id.clone());
        thread::spawn(move || {
            for round in 0..ROUNDS {
                store
                    .rename_meeting(&id, &format!("Title {round}"))
                    .unwrap();
                let now = store.meeting(&id).unwrap();
                assert_eq!(now.title, format!("Title {round}"));
            }
        })
    };
    let saver = {
        let (store, id) = (Arc::clone(&store), meeting.id.clone());
        thread::spawn(move || {
            let mut body = String::new();
            for round in 0..ROUNDS {
                body.push_str(&format!("- line {round}\n"));
                let saved = store
                    .save_notes_at(&id, &body, round as u64 * 1_000)
                    .unwrap();
                assert_eq!(saved.timeline_entries, 1);
                assert_eq!(store.notes(&id).unwrap(), body);
            }
            body
        })
    };
    renamer.join().unwrap();
    let body = saver.join().unwrap();

    let now = store.meeting(&meeting.id).unwrap();
    assert_eq!(now.title, format!("Title {}", ROUNDS - 1));
    assert!(now.title_edited);
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    assert_eq!(timeline(&meeting).len(), ROUNDS);
    assert_eq!(names_in(&meeting.folder).len(), 3);
}

#[test]
fn a_save_racing_the_stop_keeps_the_notes_and_the_final_status() {
    let (_dir, store, meeting) = recording();
    let store = Arc::new(store);
    store.rename_meeting(&meeting.id, "Standup").unwrap();

    let saver = {
        let (store, id) = (Arc::clone(&store), meeting.id.clone());
        thread::spawn(move || {
            let mut body = String::new();
            for round in 0..30u64 {
                body.push_str(&format!("- line {round}\n"));
                store.save_notes_at(&id, &body, round * 1_000).unwrap();
            }
            body
        })
    };
    let finished = store.finish_meeting(&meeting.id, 42).unwrap();
    let body = saver.join().unwrap();

    assert!(finished.rename_error.is_none());
    let now = store.meeting(&meeting.id).unwrap();
    assert_eq!(now.status, MeetingStatus::Complete);
    assert_eq!(now.duration_secs, Some(42));
    assert_eq!(now.title, "Standup");
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(store.notes(&meeting.id).unwrap(), body);
    // Saves that came after the stop logged nothing; the ones before it
    // logged a line each, in order.
    let entries = timeline(&now);
    assert!(entries.len() <= 30);
    for (round, (_, text)) in entries.iter().enumerate() {
        assert_eq!(text, &format!("- line {round}"));
    }
    assert!(
        names_in(&now.folder)
            .iter()
            .all(|name| !is_temp_file_name(name))
    );
}

#[test]
fn notes_and_the_transcript_do_not_disturb_each_other() {
    let (_dir, store, meeting) = recording();
    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    let segment = |start_ms, text: &str| Segment {
        speaker: Speaker::Them,
        start_ms,
        end_ms: start_ms + 1_000,
        text: text.to_owned(),
    };

    writer.append(&segment(5_000, "second")).unwrap();
    store
        .save_notes_at(&meeting.id, "- a note\n", 6_000)
        .unwrap();
    writer.append(&segment(1_000, "first")).unwrap();
    store
        .save_notes_at(&meeting.id, "- a note\n- another\n", 7_000)
        .unwrap();
    writer.finalize().unwrap();
    store.finish_meeting(&meeting.id, 8).unwrap();

    assert_eq!(
        text_of(&meeting.transcript_path()),
        "[00:00:01] Them: first\n[00:00:05] Them: second\n"
    );
    assert_eq!(
        timeline(&meeting),
        [
            entry("00:00:06", "- a note"),
            entry("00:00:07", "- another")
        ]
    );
    assert_eq!(store.notes(&meeting.id).unwrap(), "- a note\n- another\n");
}
