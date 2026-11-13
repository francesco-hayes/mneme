//! Meeting folders through the public API, in temporary folders.
//!
//! Start times are fixed values with explicit UTC offsets, so nothing here
//! depends on the clock or on the `TZ` the tests run under.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use mneme_store::{
    Error, FrontmatterError, MeetingId, MeetingStatus, MeetingSummary, NewMeeting, StartTime, Store,
};
use saphyr::{LoadableYamlNode, YamlOwned};
use tempfile::TempDir;

/// A store in a fresh temp folder. Keep the `TempDir` alive for the test.
fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    (dir, store)
}

fn at(start: &str) -> NewMeeting {
    NewMeeting {
        started_at: start.parse().unwrap(),
        model: Some("large-v3-turbo".to_owned()),
        language: Some("auto".to_owned()),
    }
}

/// 2:30 PM in a zone one hour east of UTC.
const START: &str = "2026-10-27T14:30:00+01:00";

fn create(store: &Store) -> MeetingSummary {
    store.create_meeting(at(START)).unwrap()
}

fn notes(meeting: &MeetingSummary) -> String {
    fs::read_to_string(meeting.notes_path()).unwrap()
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

/// The YAML between the `---` lines of a `notes.md`, parsed by a real YAML
/// parser.
fn frontmatter_yaml(text: &str) -> YamlOwned {
    let block = text.strip_prefix("---\n").expect("starts with ---");
    let end = block.find("\n---\n").expect("closing ---") + 1;
    let mut docs = YamlOwned::load_from_str(&block[..end]).expect("valid YAML");
    assert_eq!(docs.len(), 1);
    docs.remove(0)
}

// --- Opening -------------------------------------------------------------

#[test]
fn open_creates_a_missing_root_and_accepts_an_existing_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("a").join("b").join("Mneme");

    let store = Store::open(&root).unwrap();
    assert!(root.is_dir());
    assert_eq!(store.root(), root);

    let meeting = create(&store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.meeting(&meeting.id).unwrap(), meeting);
}

#[test]
fn open_fails_when_the_root_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-folder");
    fs::write(&path, "x").unwrap();

    assert!(matches!(Store::open(&path), Err(Error::Io { .. })));
    assert_eq!(fs::read_to_string(&path).unwrap(), "x");
}

#[test]
fn a_store_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Store>();
}

// --- Create --------------------------------------------------------------

#[test]
fn create_makes_the_folder_and_its_three_files() {
    let (_dir, store) = store();

    let meeting = create(&store);

    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Meeting"]);
    assert_eq!(meeting.folder, store.root().join("2026-10-27 1430 Meeting"));
    assert_eq!(
        names_in(&meeting.folder),
        ["notes-timeline.jsonl", "notes.md", "transcript.md"]
    );
    assert_eq!(fs::read(meeting.transcript_path()).unwrap(), b"");
    assert_eq!(fs::read(meeting.timeline_path()).unwrap(), b"");
}

#[test]
fn create_writes_this_exact_notes_file() {
    let (_dir, store) = store();

    let meeting = create(&store);

    let expected = format!(
        "---\n\
         id: {}\n\
         title: Meeting, Oct 27 2:30 PM\n\
         title_edited: false\n\
         date: \"2026-10-27T14:30:00+01:00\"\n\
         duration:\n\
         status: recording\n\
         model: large-v3-turbo\n\
         language: auto\n\
         ---\n",
        meeting.id
    );
    assert_eq!(notes(&meeting), expected);
}

#[test]
fn create_returns_what_a_later_read_returns() {
    let (_dir, store) = store();

    let meeting = create(&store);

    assert_eq!(meeting.title, "Meeting, Oct 27 2:30 PM");
    assert!(!meeting.title_edited);
    assert_eq!(meeting.started_at, START.parse::<StartTime>().unwrap());
    assert_eq!(meeting.started_at.to_rfc3339(), START);
    assert_eq!(meeting.duration_secs, None);
    assert_eq!(meeting.status, MeetingStatus::Recording);
    assert_eq!(meeting.model.as_deref(), Some("large-v3-turbo"));
    assert_eq!(meeting.language.as_deref(), Some("auto"));
    assert_eq!(meeting.expected_folder_name(), "2026-10-27 1430 Meeting");
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);
}

#[test]
fn created_frontmatter_is_valid_yaml_with_the_right_types() {
    let (_dir, store) = store();
    let meeting = store
        .create_meeting(NewMeeting {
            started_at: START.parse().unwrap(),
            model: Some("large-v3-turbo".to_owned()),
            // Norwegian: a YAML 1.1 parser reads a bare `no` as false.
            language: Some("no".to_owned()),
        })
        .unwrap();

    let yaml = frontmatter_yaml(&notes(&meeting));

    let keys: Vec<&str> = yaml
        .as_mapping()
        .unwrap()
        .keys()
        .map(|key| key.as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "title",
            "title_edited",
            "date",
            "duration",
            "status",
            "model",
            "language"
        ]
    );
    assert_eq!(yaml["id"].as_str(), Some(meeting.id.as_str()));
    assert_eq!(yaml["title"].as_str(), Some("Meeting, Oct 27 2:30 PM"));
    assert_eq!(yaml["title_edited"].as_bool(), Some(false));
    assert_eq!(yaml["date"].as_str(), Some(START));
    assert!(yaml["duration"].is_null());
    assert_eq!(yaml["status"].as_str(), Some("recording"));
    assert_eq!(yaml["model"].as_str(), Some("large-v3-turbo"));
    assert_eq!(yaml["language"].as_str(), Some("no"));
    assert_eq!(
        store.meeting(&meeting.id).unwrap().language.as_deref(),
        Some("no")
    );
}

#[test]
fn create_without_model_or_language_leaves_them_empty() {
    let (_dir, store) = store();

    let meeting = store
        .create_meeting(NewMeeting {
            started_at: START.parse().unwrap(),
            model: None,
            language: Some(String::new()),
        })
        .unwrap();

    assert!(notes(&meeting).ends_with("status: recording\nmodel:\nlanguage:\n---\n"));
    let yaml = frontmatter_yaml(&notes(&meeting));
    assert!(yaml["model"].is_null());
    assert!(yaml["language"].is_null());
    let read = store.meeting(&meeting.id).unwrap();
    assert_eq!((read.model, read.language), (None, None));
}

#[test]
fn folder_and_title_come_from_the_local_time_in_the_start_value() {
    let (_dir, store) = store();

    // One instant, written in three zones: three different local times.
    for (start, folder, title) in [
        (
            "2026-10-27T23:05:00-07:00",
            "2026-10-27 2305 Meeting",
            "Meeting, Oct 27 11:05 PM",
        ),
        (
            "2026-10-28T06:05:00+00:00",
            "2026-10-28 0605 Meeting",
            "Meeting, Oct 28 6:05 AM",
        ),
        (
            "2026-10-28T11:35:00+05:30",
            "2026-10-28 1135 Meeting",
            "Meeting, Oct 28 11:35 AM",
        ),
    ] {
        let meeting = store.create_meeting(at(start)).unwrap();
        assert_eq!(meeting.folder.file_name().unwrap(), folder);
        assert_eq!(meeting.title, title);
        assert!(notes(&meeting).contains(&format!("date: \"{start}\"\n")));
        assert_eq!(
            store.meeting(&meeting.id).unwrap().started_at.to_rfc3339(),
            start
        );
    }
}

#[test]
fn every_meeting_gets_its_own_id() {
    let (_dir, store) = store();

    let a = store
        .create_meeting(at("2026-10-27T09:00:00+01:00"))
        .unwrap();
    let b = store
        .create_meeting(at("2026-10-27T09:01:00+01:00"))
        .unwrap();

    assert_ne!(a.id, b.id);
    assert_eq!(store.meeting(&a.id).unwrap().folder, a.folder);
    assert_eq!(store.meeting(&b.id).unwrap().folder, b.folder);
}

// --- Two meetings in one minute --------------------------------------------

const PLAIN: &str = "2026-10-27 1430 Meeting";

/// The `n`th folder name for a meeting started at [`START`] that keeps its
/// default title.
fn numbered(n: usize) -> String {
    if n == 1 {
        PLAIN.to_owned()
    } else {
        format!("{PLAIN} {n}")
    }
}

fn folder_name(meeting: &MeetingSummary) -> &str {
    meeting.folder.file_name().unwrap().to_str().unwrap()
}

/// What a list of the root will be built from: the `(id, title)` in the
/// frontmatter of every folder, read by a YAML parser, in folder order.
fn ids_and_titles_on_disk(root: &Path) -> Vec<(String, String)> {
    names_in(root)
        .iter()
        .map(|name| {
            let text = fs::read_to_string(root.join(name).join("notes.md")).unwrap();
            let yaml = frontmatter_yaml(&text);
            (
                yaml["id"].as_str().unwrap().to_owned(),
                yaml["title"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn creates_in_the_same_minute_get_numbered_folders() {
    let (_dir, store) = store();
    let first = create(&store);
    fs::write(first.transcript_path(), "[00:00:01] Me: hello\n").unwrap();
    let first_before = snapshot(&first.folder);

    // Seconds differ, the minute does not.
    let second = store
        .create_meeting(at("2026-10-27T14:30:20+01:00"))
        .unwrap();
    let third = store
        .create_meeting(at("2026-10-27T14:30:59+01:00"))
        .unwrap();

    assert_eq!(folder_name(&first), "2026-10-27 1430 Meeting");
    assert_eq!(folder_name(&second), "2026-10-27 1430 Meeting 2");
    assert_eq!(folder_name(&third), "2026-10-27 1430 Meeting 3");
    assert_eq!(
        names_in(store.root()),
        [
            "2026-10-27 1430 Meeting",
            "2026-10-27 1430 Meeting 2",
            "2026-10-27 1430 Meeting 3"
        ]
    );
    for meeting in [&second, &third] {
        assert_eq!(meeting.folder.parent().unwrap(), store.root());
        assert_eq!(
            names_in(&meeting.folder),
            ["notes-timeline.jsonl", "notes.md", "transcript.md"]
        );
        assert_eq!(fs::read(meeting.transcript_path()).unwrap(), b"");
        assert_eq!(fs::read(meeting.timeline_path()).unwrap(), b"");
    }
    // The first meeting's folder was not touched by the later creates.
    assert_eq!(snapshot(&first.folder), first_before);
}

#[test]
fn same_minute_meetings_are_separate_meetings_with_the_same_title() {
    let (_dir, store) = store();

    let first = create(&store);
    let second = create(&store);
    let third = create(&store);

    // Three ids.
    assert_ne!(first.id, second.id);
    assert_ne!(first.id, third.id);
    assert_ne!(second.id, third.id);

    // The number is in the folder name and nowhere else: apart from the id,
    // the three notes files are the same, byte for byte.
    for meeting in [&first, &second, &third] {
        assert_eq!(meeting.title, "Meeting, Oct 27 2:30 PM");
        assert!(!meeting.title_edited);
        assert_eq!(
            notes(meeting),
            format!(
                "---\n\
                 id: {}\n\
                 title: Meeting, Oct 27 2:30 PM\n\
                 title_edited: false\n\
                 date: \"2026-10-27T14:30:00+01:00\"\n\
                 duration:\n\
                 status: recording\n\
                 model: large-v3-turbo\n\
                 language: auto\n\
                 ---\n",
                meeting.id
            )
        );
        // Each id leads to its own folder, and reads back as created.
        assert_eq!(store.meeting(&meeting.id).unwrap(), *meeting);
        // The title asks for the plain name, and the folder is fine as it
        // is: nothing is waiting to rename it.
        assert_eq!(meeting.expected_folder_name(), PLAIN);
        assert_eq!(meeting.pending_folder_name(), None);
    }

    // Scanning the root, as a list does, finds three meetings that differ
    // only by id.
    let same_title = |id: &MeetingId| (id.to_string(), "Meeting, Oct 27 2:30 PM".to_owned());
    assert_eq!(
        ids_and_titles_on_disk(store.root()),
        [
            same_title(&first.id),
            same_title(&second.id),
            same_title(&third.id)
        ]
    );
}

#[test]
fn a_new_meeting_takes_the_lowest_free_number() {
    let (_dir, store) = store();
    let first = create(&store);
    let second = create(&store);
    let third = create(&store);
    assert_eq!(folder_name(&third), numbered(3));

    // `Meeting` and `Meeting 3` are there, `Meeting 2` is not: the gap is
    // filled.
    fs::remove_dir_all(&second.folder).unwrap();
    let fourth = create(&store);
    assert_eq!(folder_name(&fourth), numbered(2));

    // No gap left: the next one goes after the highest.
    let fifth = create(&store);
    assert_eq!(folder_name(&fifth), numbered(4));

    // The plain name is a gap like any other.
    fs::remove_dir_all(&first.folder).unwrap();
    let sixth = create(&store);
    assert_eq!(folder_name(&sixth), numbered(1));

    assert_eq!(
        names_in(store.root()),
        [numbered(1), numbered(2), numbered(3), numbered(4)]
    );
    // Whoever had a name keeps it.
    for meeting in [&third, &fourth, &fifth, &sixth] {
        assert_eq!(store.meeting(&meeting.id).unwrap(), *meeting);
    }
}

#[test]
fn names_taken_by_things_that_are_not_meetings_are_skipped_and_left_alone() {
    let (_dir, store) = store();
    // A plain file, a folder with someone's file in it, and a link that
    // points nowhere.
    fs::write(store.root().join(numbered(1)), "a file").unwrap();
    fs::create_dir(store.root().join(numbered(2))).unwrap();
    fs::write(store.root().join(numbered(2)).join("keep.txt"), "mine").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("nowhere", store.root().join(numbered(3))).unwrap();
    #[cfg(not(unix))]
    fs::create_dir(store.root().join(numbered(3))).unwrap();

    let meeting = create(&store);

    assert_eq!(folder_name(&meeting), numbered(4));
    assert_eq!(
        names_in(store.root()),
        [numbered(1), numbered(2), numbered(3), numbered(4)]
    );
    assert_eq!(
        fs::read_to_string(store.root().join(numbered(1))).unwrap(),
        "a file"
    );
    assert_eq!(names_in(&store.root().join(numbered(2))), ["keep.txt"]);
    assert_eq!(
        fs::read_to_string(store.root().join(numbered(2)).join("keep.txt")).unwrap(),
        "mine"
    );
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(store.root().join(numbered(3))).unwrap(),
        Path::new("nowhere")
    );
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);
}

#[test]
fn a_name_taken_in_other_capitals_counts_as_taken_where_the_disk_says_so() {
    let (_dir, store) = store();
    let shouting = store.root().join("2026-10-27 1430 MEETING");
    fs::create_dir(&shouting).unwrap();
    // On a disk that ignores case the plain name is now taken; on one that
    // does not, it is free.
    let ignores_case = store.root().join(PLAIN).exists();

    let meeting = create(&store);

    // Either way the meeting has a folder of its own.
    let expected = if ignores_case {
        numbered(2)
    } else {
        numbered(1)
    };
    assert_eq!(folder_name(&meeting), expected);
    assert_eq!(names_in(store.root()).len(), 2);
    assert!(names_in(&shouting).is_empty());
    assert_eq!(names_in(&meeting.folder).len(), 3);
}

#[test]
fn finish_leaves_a_numbered_folder_where_it_is() {
    let (_dir, store) = store();
    let first = create(&store);
    let second = create(&store);
    assert_eq!(folder_name(&second), numbered(2));
    let created = notes(&second);

    let finished = store.finish_meeting(&second.id, 75).unwrap();

    assert!(finished.rename_error.is_none());
    let done = finished.meeting;
    assert_eq!(done.folder, second.folder);
    assert_eq!(done.title, "Meeting, Oct 27 2:30 PM");
    assert!(!done.title_edited);
    assert_eq!(done.status, MeetingStatus::Complete);
    assert_eq!(done.pending_folder_name(), None);
    assert_eq!(
        notes(&done),
        created
            .replace("duration:\n", "duration: 75\n")
            .replace("status: recording\n", "status: complete\n")
    );
    assert_eq!(names_in(store.root()), [numbered(1), numbered(2)]);
    assert_eq!(store.meeting(&second.id).unwrap(), done);

    // Not even when the plain name is free by the time the meeting stops:
    // the number is never "corrected" away.
    let third = create(&store);
    assert_eq!(folder_name(&third), numbered(3));
    fs::remove_dir_all(&first.folder).unwrap();

    let finished = store.finish_meeting(&third.id, 5).unwrap();

    assert!(finished.rename_error.is_none());
    assert_eq!(finished.meeting.folder, third.folder);
    assert_eq!(names_in(store.root()), [numbered(2), numbered(3)]);
    assert_eq!(store.meeting(&second.id).unwrap().folder, second.folder);
}

#[test]
fn editing_the_title_moves_a_numbered_folder_to_the_name_without_a_number() {
    let (_dir, store) = store();
    let first = create(&store);
    let second = create(&store);
    let third = create(&store);
    fs::write(second.transcript_path(), "[00:00:01] Me: hello\n").unwrap();

    // After completion: moved by the rename itself.
    store.finish_meeting(&second.id, 60).unwrap();
    let renamed = store.rename_meeting(&second.id, "Standup").unwrap();

    assert_eq!(folder_name(&renamed), "2026-10-27 1430 Standup");
    assert_eq!(renamed.id, second.id);
    assert_eq!(renamed.title, "Standup");
    assert!(renamed.title_edited);
    assert_eq!(renamed.pending_folder_name(), None);
    assert_eq!(
        fs::read_to_string(renamed.transcript_path()).unwrap(),
        "[00:00:01] Me: hello\n"
    );
    assert_eq!(
        names_in(store.root()),
        [
            numbered(1),
            numbered(3),
            "2026-10-27 1430 Standup".to_owned()
        ]
    );
    assert_eq!(store.meeting(&second.id).unwrap(), renamed);

    // During recording: the title is saved, the numbered folder waits, and
    // finish moves it to the name without a number.
    let retitled = store.rename_meeting(&third.id, "Design review").unwrap();
    assert_eq!(retitled.folder, third.folder);
    assert_eq!(
        retitled.pending_folder_name().as_deref(),
        Some("2026-10-27 1430 Design review")
    );
    let finished = store.finish_meeting(&third.id, 30).unwrap();
    assert!(finished.rename_error.is_none());
    assert_eq!(
        folder_name(&finished.meeting),
        "2026-10-27 1430 Design review"
    );
    assert_eq!(finished.meeting.id, third.id);

    // The untouched first meeting is where it was.
    assert_eq!(store.meeting(&first.id).unwrap(), first);
    assert_eq!(
        names_in(store.root()),
        [
            "2026-10-27 1430 Design review",
            "2026-10-27 1430 Meeting",
            "2026-10-27 1430 Standup"
        ]
    );
}

#[test]
fn a_title_whose_folder_name_is_taken_is_still_refused_not_numbered() {
    let (_dir, store) = store();
    let first = create(&store);
    let second = create(&store);
    store.finish_meeting(&first.id, 60).unwrap();
    store.rename_meeting(&first.id, "Standup").unwrap();
    let third = create(&store);
    assert_eq!(folder_name(&third), numbered(1));
    let before = snapshot(store.root());
    let taken = |name: &str| store.root().join(name);

    // The second meeting cannot take the first one's title, while recording
    // or after: no `Standup 2` is made for a name the user chose.
    for round in 0..2 {
        let err = store.rename_meeting(&second.id, "Standup").unwrap_err();
        assert!(
            matches!(&err, Error::FolderExists { path } if *path == taken("2026-10-27 1430 Standup")),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "a folder named \"2026-10-27 1430 Standup\" already exists"
        );
        // Nor can it be titled "Meeting", or a title that falls back to
        // `Meeting`, while another folder has the plain name.
        for title in ["Meeting", "???"] {
            let err = store.rename_meeting(&second.id, title).unwrap_err();
            assert!(
                matches!(&err, Error::FolderExists { path } if *path == taken(PLAIN)),
                "{title:?}: {err:?}"
            );
        }
        // And a title that spells out another meeting's numbered folder
        // name is refused the same way.
        let err = store.rename_meeting(&third.id, "Meeting 2").unwrap_err();
        assert!(
            matches!(&err, Error::FolderExists { path } if *path == taken(&numbered(2))),
            "{err:?}"
        );

        if round == 0 {
            // Nothing changed, in any folder or file.
            assert_eq!(snapshot(store.root()), before);
            let after = store.meeting(&second.id).unwrap();
            assert_eq!(after, second);
            assert_eq!(folder_name(&after), numbered(2));
            store.finish_meeting(&second.id, 5).unwrap();
        }
    }

    let after = store.meeting(&second.id).unwrap();
    assert_eq!(after.title, "Meeting, Oct 27 2:30 PM");
    assert!(!after.title_edited);
    assert_eq!(folder_name(&after), numbered(2));
    assert_eq!(
        names_in(store.root()),
        [
            numbered(1),
            numbered(2),
            "2026-10-27 1430 Standup".to_owned()
        ]
    );

    // A free title still works, and moves the folder to it without a number.
    let renamed = store.rename_meeting(&second.id, "Retro").unwrap();
    assert_eq!(folder_name(&renamed), "2026-10-27 1430 Retro");
}

#[test]
fn a_title_that_ends_in_a_number_is_kept_whole() {
    let (_dir, store) = store();
    let meeting = create(&store);

    // Typed during recording, applied at finish.
    store.rename_meeting(&meeting.id, "Sprint 2").unwrap();
    let done = store.finish_meeting(&meeting.id, 60).unwrap().meeting;
    assert_eq!(folder_name(&done), "2026-10-27 1430 Sprint 2");
    assert_eq!(done.title, "Sprint 2");
    assert_eq!(done.pending_folder_name(), None);

    // Saving the same title again changes nothing: the `2` is not taken for
    // a number Mneme added.
    let before = snapshot(store.root());
    assert_eq!(store.rename_meeting(&meeting.id, "Sprint 2").unwrap(), done);
    assert_eq!(snapshot(store.root()), before);

    // A new meeting in that minute is not confused by it either.
    let other = create(&store);
    assert_eq!(folder_name(&other), numbered(1));
    assert_eq!(store.meeting(&meeting.id).unwrap(), done);

    // Dropping the number from the title drops it from the folder, and
    // putting another one in puts it there.
    let renamed = store.rename_meeting(&meeting.id, "Sprint").unwrap();
    assert_eq!(folder_name(&renamed), "2026-10-27 1430 Sprint");
    let renamed = store.rename_meeting(&meeting.id, "Sprint 3").unwrap();
    assert_eq!(folder_name(&renamed), "2026-10-27 1430 Sprint 3");
    assert_eq!(
        names_in(store.root()),
        [numbered(1), "2026-10-27 1430 Sprint 3".to_owned()]
    );
}

#[test]
fn a_meeting_titled_like_a_numbered_default_is_treated_as_titled() {
    let (_dir, store) = store();
    // The user really did call it "Meeting 2".
    let titled = create(&store);
    store.finish_meeting(&titled.id, 60).unwrap();
    let titled = store.rename_meeting(&titled.id, "Meeting 2").unwrap();
    assert_eq!(folder_name(&titled), numbered(2));
    assert_eq!(names_in(store.root()), [numbered(2)]);

    // New meetings in that minute go around the name it holds.
    let a = create(&store);
    let b = create(&store);
    assert_eq!(folder_name(&a), numbered(1));
    assert_eq!(folder_name(&b), numbered(3));

    // It is still the titled meeting: same id, title and folder.
    let read = store.meeting(&titled.id).unwrap();
    assert_eq!(read, titled);
    assert_eq!(read.title, "Meeting 2");
    assert!(read.title_edited);

    // Unlike a numbered default folder, it follows its title: a new title
    // moves it, and the name it leaves is free for the next new meeting.
    let renamed = store
        .rename_meeting(&titled.id, "Meeting 2 (final)")
        .unwrap();
    assert_eq!(folder_name(&renamed), "2026-10-27 1430 Meeting 2 (final)");
    let c = create(&store);
    assert_eq!(folder_name(&c), numbered(2));

    // And the default-titled ones do not follow anything: plain or
    // numbered, they stay where they are through finish.
    for meeting in [&a, &b, &c] {
        let done = store.finish_meeting(&meeting.id, 1).unwrap();
        assert!(done.rename_error.is_none());
        assert_eq!(done.meeting.folder, meeting.folder);
    }
    assert_eq!(
        names_in(store.root()),
        [
            numbered(1),
            "2026-10-27 1430 Meeting 2".to_owned(),
            "2026-10-27 1430 Meeting 2 (final)".to_owned(),
            numbered(3)
        ]
    );
}

#[test]
fn create_gives_up_cleanly_when_every_number_is_taken() {
    const HIGHEST: usize = 999;
    let (_dir, store) = store();
    for n in 1..=HIGHEST {
        fs::create_dir(store.root().join(numbered(n))).unwrap();
    }

    let err = store.create_meeting(at(START)).unwrap_err();

    assert!(
        matches!(&err, Error::FolderExists { path } if *path == store.root().join(PLAIN)),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "a folder named \"2026-10-27 1430 Meeting\" already exists"
    );
    // Nothing was created: no folder past the highest number, and nothing
    // inside the ones that are there.
    let names = names_in(store.root());
    assert_eq!(names.len(), HIGHEST);
    assert!(!store.root().join(numbered(HIGHEST + 1)).exists());
    for name in &names {
        assert!(names_in(&store.root().join(name)).is_empty(), "{name}");
    }

    // Another minute is not affected.
    let other = store
        .create_meeting(at("2026-10-27T14:31:00+01:00"))
        .unwrap();
    assert_eq!(folder_name(&other), "2026-10-27 1431 Meeting");

    // One free name anywhere in the range is found, the highest included.
    fs::remove_dir(store.root().join(numbered(HIGHEST))).unwrap();
    let last = create(&store);
    assert_eq!(folder_name(&last), numbered(HIGHEST));
    assert_eq!(store.meeting(&last.id).unwrap(), last);
    assert!(matches!(
        store.create_meeting(at(START)),
        Err(Error::FolderExists { .. })
    ));
}

// --- Looking up ----------------------------------------------------------

#[test]
fn an_unknown_id_is_not_found() {
    let (_dir, store) = store();
    create(&store);

    let id = MeetingId::new("no-such-id");
    let err = store.meeting(&id).unwrap_err();

    assert!(matches!(&err, Error::MeetingNotFound { id: missing } if *missing == id));
    assert_eq!(err.to_string(), "no meeting with id no-such-id");
}

#[test]
fn a_meeting_is_found_by_id_after_its_folder_is_renamed_by_hand() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let moved = store.root().join("Renamed in Finder");
    fs::rename(&meeting.folder, &moved).unwrap();

    let found = store.meeting(&meeting.id).unwrap();

    assert_eq!(found.folder, moved);
    assert_eq!(found.id, meeting.id);
}

#[test]
fn junk_in_the_root_is_ignored() {
    let (_dir, store) = store();
    let meeting = create(&store);
    fs::write(store.root().join(".DS_Store"), [0u8, 1, 2]).unwrap();
    fs::write(store.root().join("stray.md"), "---\nid: x\n---\n").unwrap();
    fs::create_dir(store.root().join("empty folder")).unwrap();
    fs::create_dir(store.root().join("not a meeting")).unwrap();
    fs::write(
        store.root().join("not a meeting").join("notes.md"),
        "no frontmatter\n",
    )
    .unwrap();
    fs::create_dir(store.root().join("binary notes")).unwrap();
    fs::write(
        store.root().join("binary notes").join("notes.md"),
        [0xFFu8, 0xFE],
    )
    .unwrap();

    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);
    assert!(matches!(
        store.meeting(&MeetingId::new("x")),
        Err(Error::MeetingNotFound { .. })
    ));
}

// --- Frontmatter survives hand edits -------------------------------------

/// What a user might do to `notes.md` in another editor: reorder nothing,
/// but add comments, properties of their own, and notes that use `---`.
fn hand_edit(meeting: &MeetingSummary) -> (String, String) {
    let frontmatter = format!(
        "---\n\
         # edited in Obsidian\n\
         id: {id}\n\
         tags:\n  - work\n  - planning\n\
         title: Meeting, Oct 27 2:30 PM\n\
         title_edited: false\n\
         aliases: [kickoff, \"q4\"]\n\
         date: \"2026-10-27T14:30:00+01:00\"\n\
         duration:\n\
         \n\
         status: recording   # still going\n\
         summary: |\n  First line.\n  ---\n  Last line.\n\
         model: large-v3-turbo\n\
         language: auto\n\
         x-future-key: {{a: 1}}\n\
         ---\n",
        id = meeting.id
    );
    let body =
        "# Notes\n\n- before the rule\n\n---\n\n- after the rule\n---\nid: not-frontmatter\n---\n";
    fs::write(meeting.notes_path(), format!("{frontmatter}{body}")).unwrap();
    (frontmatter, body.to_owned())
}

#[test]
fn rewrites_keep_unknown_keys_their_order_and_the_body() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let (frontmatter, body) = hand_edit(&meeting);

    // The edited file still reads as the same meeting.
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);

    store.rename_meeting(&meeting.id, "Q4: kickoff").unwrap();
    let finished = store.finish_meeting(&meeting.id, 1805).unwrap();
    assert!(finished.rename_error.is_none());

    // Only the four values Mneme changed differ; every other byte, the
    // user's keys and comments and the whole body, is as it was.
    let expected = frontmatter
        .replace(
            "title: Meeting, Oct 27 2:30 PM\n",
            "title: \"Q4: kickoff\"\n",
        )
        .replace("title_edited: false\n", "title_edited: true\n")
        .replace("duration:\n", "duration: 1805\n")
        .replace(
            "status: recording   # still going\n",
            "status: complete # still going\n",
        )
        + &body;
    assert_eq!(notes(&finished.meeting), expected);

    let yaml = frontmatter_yaml(&expected);
    assert_eq!(yaml["title"].as_str(), Some("Q4: kickoff"));
    assert_eq!(yaml["tags"][0].as_str(), Some("work"));
    assert_eq!(yaml["aliases"][1].as_str(), Some("q4"));
    assert_eq!(
        yaml["summary"].as_str(),
        Some("First line.\n---\nLast line.\n")
    );
    assert_eq!(yaml["x-future-key"]["a"].as_integer(), Some(1));
    assert_eq!(yaml["duration"].as_integer(), Some(1805));
}

#[test]
fn a_file_with_windows_line_endings_keeps_them() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let crlf = notes(&meeting).replace('\n', "\r\n") + "line one\r\n---\r\nline two\r\n";
    fs::write(meeting.notes_path(), &crlf).unwrap();

    let finished = store.finish_meeting(&meeting.id, 60).unwrap();

    let expected = crlf
        .replace("duration:\r\n", "duration: 60\r\n")
        .replace("status: recording\r\n", "status: complete\r\n");
    assert_eq!(notes(&finished.meeting), expected);
}

#[test]
fn broken_frontmatter_is_an_error_for_that_meeting_only() {
    let (_dir, store) = store();
    let good = store
        .create_meeting(at("2026-10-27T09:00:00+01:00"))
        .unwrap();
    let bad_status = store
        .create_meeting(at("2026-10-27T10:00:00+01:00"))
        .unwrap();
    let bad_date = store
        .create_meeting(at("2026-10-27T11:00:00+01:00"))
        .unwrap();
    let unclosed = store
        .create_meeting(at("2026-10-27T12:00:00+01:00"))
        .unwrap();
    let stripped = store
        .create_meeting(at("2026-10-27T13:00:00+01:00"))
        .unwrap();

    let edit = |meeting: &MeetingSummary, from: &str, to: &str| {
        let text = notes(meeting);
        assert!(text.contains(from));
        fs::write(meeting.notes_path(), text.replace(from, to)).unwrap();
    };
    edit(&bad_status, "status: recording", "status: paused");
    edit(
        &bad_date,
        "date: \"2026-10-27T11:00:00+01:00\"",
        "date: last Tuesday",
    );
    edit(&unclosed, "language: auto\n---\n", "language: auto\n");
    fs::write(stripped.notes_path(), "Just my notes, no frontmatter.\n").unwrap();
    let before = snapshot(store.root());

    // The id is readable, the meeting is not: the error names the file and
    // the key.
    for (meeting, key) in [(&bad_status, "status"), (&bad_date, "date")] {
        for result in [
            store.meeting(&meeting.id).map(drop),
            store.rename_meeting(&meeting.id, "New title").map(drop),
            store.finish_meeting(&meeting.id, 10).map(drop),
        ] {
            let err = result.unwrap_err();
            assert!(
                matches!(
                    &err,
                    Error::Frontmatter { path, source: FrontmatterError::InvalidValue { key: k, .. } }
                        if *path == meeting.notes_path() && k == key
                ),
                "{err:?}"
            );
            assert!(err.to_string().contains("notes.md"), "{err}");
            assert!(err.to_string().contains(key), "{err}");
        }
    }

    // No readable id: the folder cannot be matched to the meeting at all.
    for meeting in [&unclosed, &stripped] {
        assert!(matches!(
            store.meeting(&meeting.id),
            Err(Error::MeetingNotFound { .. })
        ));
        assert!(matches!(
            store.rename_meeting(&meeting.id, "New title"),
            Err(Error::MeetingNotFound { .. })
        ));
    }

    // None of the failed calls wrote anything, and the good meeting works.
    assert_eq!(snapshot(store.root()), before);
    assert_eq!(store.meeting(&good.id).unwrap(), good);
    let renamed = store.rename_meeting(&good.id, "Still fine").unwrap();
    assert_eq!(renamed.title, "Still fine");
    assert!(store.finish_meeting(&good.id, 5).is_ok());
}

// --- Rename --------------------------------------------------------------

#[test]
fn rename_after_completion_updates_frontmatter_and_moves_the_folder() {
    let (_dir, store) = store();
    let meeting = create(&store);
    fs::write(meeting.transcript_path(), "[00:00:01] Me: hello\n").unwrap();
    let finished = store.finish_meeting(&meeting.id, 90).unwrap().meeting;
    assert_eq!(finished.folder, meeting.folder);

    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();

    assert_eq!(renamed.id, meeting.id);
    assert_eq!(renamed.title, "Standup");
    assert!(renamed.title_edited);
    assert_eq!(renamed.folder, store.root().join("2026-10-27 1430 Standup"));
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(
        names_in(&renamed.folder),
        ["notes-timeline.jsonl", "notes.md", "transcript.md"]
    );
    assert_eq!(
        fs::read_to_string(renamed.transcript_path()).unwrap(),
        "[00:00:01] Me: hello\n"
    );
    assert!(notes(&renamed).contains("\ntitle: Standup\ntitle_edited: true\n"));
    assert_eq!(store.meeting(&meeting.id).unwrap(), renamed);
    assert_eq!(renamed.duration_secs, Some(90));
    assert_eq!(renamed.status, MeetingStatus::Complete);
}

#[test]
fn rename_can_be_repeated_and_the_id_never_changes() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let id = meeting.id.clone();
    let id_line = format!("\nid: {id}\n");

    // During recording, twice.
    for title in ["Draft name", "Better name"] {
        let renamed = store.rename_meeting(&id, title).unwrap();
        assert_eq!(renamed.id, id);
        assert!(notes(&renamed).contains(&id_line));
    }
    // At stop.
    let finished = store.finish_meeting(&id, 300).unwrap().meeting;
    assert_eq!(finished.id, id);
    assert_eq!(
        finished.folder.file_name().unwrap(),
        "2026-10-27 1430 Better name"
    );
    // After completion, several times, including back to an earlier title.
    for title in ["Final", "Final v2", "Better name", "Final"] {
        let renamed = store.rename_meeting(&id, title).unwrap();
        assert_eq!(renamed.id, id);
        assert_eq!(renamed.title, title);
        assert_eq!(
            renamed.folder.file_name().unwrap().to_str().unwrap(),
            format!("2026-10-27 1430 {title}")
        );
        assert!(notes(&renamed).contains(&id_line));
        assert_eq!(store.meeting(&id).unwrap(), renamed);
    }
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Final"]);
}

#[test]
fn rename_during_recording_is_saved_but_the_folder_waits_for_finish() {
    let (_dir, store) = store();
    let meeting = create(&store);

    let renamed = store.rename_meeting(&meeting.id, "Design review").unwrap();

    // Title saved, folder untouched.
    assert_eq!(renamed.title, "Design review");
    assert!(renamed.title_edited);
    assert_eq!(renamed.status, MeetingStatus::Recording);
    assert_eq!(renamed.folder, meeting.folder);
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Meeting"]);
    assert!(notes(&renamed).contains("\ntitle: Design review\ntitle_edited: true\n"));
    assert_eq!(
        renamed.expected_folder_name(),
        "2026-10-27 1430 Design review"
    );
    assert_eq!(store.meeting(&meeting.id).unwrap(), renamed);

    // Files in the folder can still be appended to at the old path.
    fs::write(meeting.transcript_path(), "[00:00:05] Them: hi\n").unwrap();

    let finished = store.finish_meeting(&meeting.id, 1234).unwrap();

    assert!(finished.rename_error.is_none());
    let done = finished.meeting;
    assert_eq!(
        done.folder,
        store.root().join("2026-10-27 1430 Design review")
    );
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Design review"]);
    assert_eq!(done.id, meeting.id);
    assert_eq!(done.title, "Design review");
    assert_eq!(done.duration_secs, Some(1234));
    assert_eq!(done.status, MeetingStatus::Complete);
    assert!(notes(&done).contains("\nduration: 1234\nstatus: complete\n"));
    assert_eq!(
        fs::read_to_string(done.transcript_path()).unwrap(),
        "[00:00:05] Them: hi\n"
    );
    assert_eq!(store.meeting(&meeting.id).unwrap(), done);
}

#[test]
fn rename_onto_an_existing_folder_fails_and_changes_nothing() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    // Something else already has the name the rename would use.
    let taken = store.root().join("2026-10-27 1430 Standup");
    fs::create_dir(&taken).unwrap();
    fs::write(taken.join("keep.txt"), "mine").unwrap();
    let before = snapshot(store.root());

    let err = store.rename_meeting(&meeting.id, "Standup").unwrap_err();

    assert!(
        matches!(&err, Error::FolderExists { path } if *path == taken),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "a folder named \"2026-10-27 1430 Standup\" already exists"
    );
    assert_eq!(snapshot(store.root()), before);
    let after = store.meeting(&meeting.id).unwrap();
    assert_eq!(after.title, "Meeting, Oct 27 2:30 PM");
    assert!(!after.title_edited);
    assert_eq!(after.folder, meeting.folder);

    // A plain file with that name blocks the rename just the same.
    fs::write(store.root().join("2026-10-27 1430 Retro"), "a file").unwrap();
    let before = snapshot(store.root());
    assert!(matches!(
        store.rename_meeting(&meeting.id, "Retro"),
        Err(Error::FolderExists { .. })
    ));
    assert_eq!(snapshot(store.root()), before);

    // A free name still works afterwards.
    let renamed = store.rename_meeting(&meeting.id, "Planning").unwrap();
    assert_eq!(
        renamed.folder.file_name().unwrap(),
        "2026-10-27 1430 Planning"
    );
}

#[test]
fn rename_during_recording_onto_an_existing_folder_is_refused_up_front() {
    let (_dir, store) = store();
    let meeting = create(&store);
    fs::create_dir(store.root().join("2026-10-27 1430 Standup")).unwrap();
    let before = snapshot(store.root());

    let err = store.rename_meeting(&meeting.id, "Standup").unwrap_err();

    assert!(matches!(err, Error::FolderExists { .. }));
    assert_eq!(snapshot(store.root()), before);
}

#[test]
fn finish_still_completes_when_the_waiting_rename_is_blocked() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.rename_meeting(&meeting.id, "Standup").unwrap();
    // The name is taken after the rename was accepted, before stop.
    let taken = store.root().join("2026-10-27 1430 Standup");
    fs::create_dir(&taken).unwrap();

    let finished = store.finish_meeting(&meeting.id, 42).unwrap();

    assert!(
        matches!(&finished.rename_error, Some(Error::FolderExists { path }) if *path == taken),
        "{:?}",
        finished.rename_error
    );
    let done = finished.meeting;
    assert_eq!(done.status, MeetingStatus::Complete);
    assert_eq!(done.duration_secs, Some(42));
    assert_eq!(done.title, "Standup");
    assert_eq!(done.folder, meeting.folder);
    assert_eq!(store.meeting(&meeting.id).unwrap(), done);
    assert_eq!(names_in(&taken), Vec::<String>::new());

    // Once the name is free, renaming again moves the folder.
    fs::remove_dir(&taken).unwrap();
    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();
    assert_eq!(renamed.folder, taken);
}

#[test]
fn rename_to_the_same_folder_name_only_updates_the_title() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    let first = store.rename_meeting(&meeting.id, "Q3/Q4").unwrap();
    assert_eq!(first.folder.file_name().unwrap(), "2026-10-27 1430 Q3 Q4");

    // A different title that sanitizes to the same folder name.
    let second = store.rename_meeting(&meeting.id, "Q3: Q4").unwrap();

    assert_eq!(second.title, "Q3: Q4");
    assert_eq!(second.folder, first.folder);
    assert!(notes(&second).contains("\ntitle: \"Q3: Q4\"\n"));

    // And the very same title again is a no-op that succeeds.
    let before = snapshot(store.root());
    assert_eq!(store.rename_meeting(&meeting.id, "Q3: Q4").unwrap(), second);
    assert_eq!(snapshot(store.root()), before);
}

#[test]
fn rename_that_only_changes_capitals_works() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    store.rename_meeting(&meeting.id, "standup").unwrap();

    // On a case-insensitive disk the new name "exists": it is this folder.
    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();

    assert_eq!(renamed.title, "Standup");
    assert_eq!(names_in(store.root()), ["2026-10-27 1430 Standup"]);
    assert_eq!(store.meeting(&meeting.id).unwrap().title, "Standup");
}

#[test]
fn nasty_titles_are_kept_in_frontmatter_and_made_safe_in_the_folder_name() {
    let long = "long ".repeat(100);
    let cases: [(&str, &str, &str); 12] = [
        ("../../etc/passwd", "../../etc/passwd", "etc passwd"),
        ("/absolute", "/absolute", "absolute"),
        (r"C:\Users\me", r"C:\Users\me", "C Users me"),
        (".hidden", ".hidden", "hidden"),
        ("..", "..", "Meeting"),
        ("???", "???", "Meeting"),
        ("  spaced   out  ", "spaced out", "spaced out"),
        (
            "two\nlines\tand a tab",
            "two lines and a tab",
            "two lines and a tab",
        ),
        (
            "Q4: \"plans\" #1 | <draft>",
            "Q4: \"plans\" #1 | <draft>",
            "Q4 plans #1 draft",
        ),
        ("ends with dots...", "ends with dots...", "ends with dots"),
        (
            "nul\0 and \u{202E}override",
            "nul and override",
            "nul and override",
        ),
        (
            "Réunion d'équipe 🚀",
            "Réunion d'équipe 🚀",
            "Réunion d'équipe 🚀",
        ),
    ];

    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();

    for (typed, title, folder_title) in cases {
        let renamed = store.rename_meeting(&meeting.id, typed).unwrap();

        assert_eq!(renamed.title, title, "{typed:?}");
        let name = renamed.folder.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, format!("2026-10-27 1430 {folder_title}"), "{typed:?}");
        // One folder, directly under the root, and nothing escaped it.
        assert_eq!(renamed.folder.parent().unwrap(), store.root());
        assert_eq!(names_in(store.root()), [name], "{typed:?}");
        // The title reads back exactly, by this crate and by a YAML parser.
        assert_eq!(store.meeting(&meeting.id).unwrap().title, title);
        let yaml = frontmatter_yaml(&notes(&renamed));
        assert_eq!(yaml["title"].as_str(), Some(title), "{typed:?}");
        assert_eq!(yaml["id"].as_str(), Some(meeting.id.as_str()));
    }

    let renamed = store.rename_meeting(&meeting.id, &long).unwrap();
    assert_eq!(renamed.title, long.trim_end());
    let name = renamed.folder.file_name().unwrap().to_str().unwrap();
    assert_eq!(name.chars().count(), "2026-10-27 1430 ".len() + 79);
    assert!(name.ends_with("long long"));
    assert_eq!(names_in(store.root()), [name]);
}

#[test]
fn an_empty_title_is_refused_and_changes_nothing() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let before = snapshot(store.root());

    for title in ["", "   ", "\n\t", "\u{0}\u{202E}"] {
        let err = store.rename_meeting(&meeting.id, title).unwrap_err();
        assert!(matches!(err, Error::EmptyTitle), "{title:?}: {err:?}");
    }

    assert_eq!(snapshot(store.root()), before);
}

#[test]
fn rename_and_finish_of_an_unknown_id_are_not_found() {
    let (_dir, store) = store();
    create(&store);
    let before = snapshot(store.root());
    let id = MeetingId::new("missing");

    assert!(matches!(
        store.rename_meeting(&id, "Title"),
        Err(Error::MeetingNotFound { .. })
    ));
    assert!(matches!(
        store.finish_meeting(&id, 1),
        Err(Error::MeetingNotFound { .. })
    ));
    assert_eq!(snapshot(store.root()), before);
}

#[cfg(unix)]
#[test]
fn a_folder_rename_that_fails_puts_the_old_frontmatter_back() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    let before = snapshot(store.root());

    // A read-only root: files inside the meeting folder can still be
    // written, but the folder itself cannot be renamed.
    let writable = fs::metadata(store.root()).unwrap().permissions();
    fs::set_permissions(store.root(), fs::Permissions::from_mode(0o555)).unwrap();
    let result = store.rename_meeting(&meeting.id, "Standup");
    fs::set_permissions(store.root(), writable).unwrap();

    if result.is_ok() {
        // Running as root: permissions do not apply, nothing to test.
        return;
    }
    assert!(matches!(result, Err(Error::Io { .. })), "{result:?}");
    assert_eq!(snapshot(store.root()), before);
    assert_eq!(
        store.meeting(&meeting.id).unwrap().title,
        "Meeting, Oct 27 2:30 PM"
    );
}

// --- Finish --------------------------------------------------------------

#[test]
fn finish_sets_duration_and_status_and_keeps_the_default_folder() {
    let (_dir, store) = store();
    let meeting = create(&store);

    let finished = store.finish_meeting(&meeting.id, 0).unwrap();

    assert!(finished.rename_error.is_none());
    let done = finished.meeting;
    assert_eq!(done.duration_secs, Some(0));
    assert_eq!(done.status, MeetingStatus::Complete);
    assert_eq!(done.folder, meeting.folder);
    assert_eq!(done.title, meeting.title);
    assert!(!done.title_edited);
    let expected = notes(&meeting)
        .replace("duration:\n", "duration: 0\n")
        .replace("status: recording\n", "status: complete\n");
    assert_eq!(notes(&done), expected);
    assert_eq!(store.meeting(&meeting.id).unwrap(), done);
}

#[test]
fn finish_leaves_a_hand_renamed_folder_alone_when_the_title_was_not_edited() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let moved = store.root().join("2026-10-27 1430 Meeting (important)");
    fs::rename(&meeting.folder, &moved).unwrap();

    let finished = store.finish_meeting(&meeting.id, 10).unwrap();

    assert!(finished.rename_error.is_none());
    assert_eq!(finished.meeting.folder, moved);
    assert_eq!(
        names_in(store.root()),
        ["2026-10-27 1430 Meeting (important)"]
    );
}

#[test]
fn finish_twice_is_refused_and_keeps_the_first_duration() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 100).unwrap();
    let before = snapshot(store.root());

    let err = store.finish_meeting(&meeting.id, 999).unwrap_err();

    assert!(
        matches!(
            &err,
            Error::NotRecording { id, status: MeetingStatus::Complete } if *id == meeting.id
        ),
        "{err:?}"
    );
    assert_eq!(snapshot(store.root()), before);
    assert_eq!(store.meeting(&meeting.id).unwrap().duration_secs, Some(100));
}

#[test]
fn the_folder_name_follows_the_offset_saved_in_the_file_not_the_machine() {
    let (_dir, store) = store();
    // Recorded at 23:05 in a zone seven hours behind UTC. Wherever this test
    // runs, a rename must keep that local date and time in the folder name.
    let meeting = store
        .create_meeting(at("2026-10-27T23:05:00-07:00"))
        .unwrap();
    store.finish_meeting(&meeting.id, 60).unwrap();

    let renamed = store.rename_meeting(&meeting.id, "Late call").unwrap();

    assert_eq!(
        renamed.folder.file_name().unwrap(),
        "2026-10-27 2305 Late call"
    );
}

// --- Concurrency ---------------------------------------------------------

#[test]
fn concurrent_renames_and_reads_never_corrupt_the_meeting() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    let body = "notes that must survive\n---\nevery rewrite\n";
    fs::write(meeting.notes_path(), notes(&meeting) + body).unwrap();

    std::thread::scope(|scope| {
        for thread in 0..6 {
            let (store, id) = (&store, &meeting.id);
            scope.spawn(move || {
                for round in 0..15 {
                    let title = format!("Thread {thread} round {round}");
                    let renamed = store.rename_meeting(id, &title).unwrap();
                    assert_eq!(renamed.id, *id);
                    // Another thread may already have renamed it again, but
                    // every read sees a whole, valid meeting.
                    let read = store.meeting(id).unwrap();
                    assert!(read.title.starts_with("Thread "), "{}", read.title);
                    assert!(read.title_edited);
                }
            });
        }
    });

    let last = store.meeting(&meeting.id).unwrap();
    assert_eq!(
        names_in(store.root()),
        [format!("2026-10-27 1430 {}", last.title)]
    );
    assert_eq!(
        names_in(&last.folder),
        ["notes-timeline.jsonl", "notes.md", "transcript.md"]
    );
    assert!(notes(&last).ends_with(&format!("---\n{body}")));
    assert_eq!(last.duration_secs, Some(60));
}

#[test]
fn concurrent_creates_in_the_same_minute_all_succeed_in_folders_of_their_own() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 3;
    let (_dir, store) = store();

    let created: Vec<MeetingSummary> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    (0..PER_THREAD)
                        .map(|_| store.create_meeting(at(START)).unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    assert_same_minute_meetings_are_whole_and_distinct(&store, &created, THREADS * PER_THREAD);
}

#[test]
fn concurrent_creates_through_separate_stores_on_one_root_never_share_a_folder() {
    // Separate stores do not share a lock, as two processes would not. All
    // that keeps their creates apart is that a folder name is claimed by
    // creating it, so this fails if create ever checks first and creates
    // second.
    const STORES: usize = 6;
    const PER_STORE: usize = 4;
    let (_dir, first) = store();
    let stores: Vec<Store> = (0..STORES)
        .map(|_| Store::open(first.root()).unwrap())
        .collect();

    let created: Vec<MeetingSummary> = std::thread::scope(|scope| {
        let handles: Vec<_> = stores
            .iter()
            .map(|store| {
                scope.spawn(move || {
                    (0..PER_STORE)
                        .map(|_| store.create_meeting(at(START)).unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    assert_same_minute_meetings_are_whole_and_distinct(&first, &created, STORES * PER_STORE);
}

/// `created` are `count` meetings made in the minute of [`START`] in an
/// otherwise empty root: every one has its own folder and id, the folders
/// are exactly `Meeting`, `Meeting 2` … `Meeting <count>`, and each folder
/// holds the files of the one meeting that was returned for it.
fn assert_same_minute_meetings_are_whole_and_distinct(
    store: &Store,
    created: &[MeetingSummary],
    count: usize,
) {
    assert_eq!(created.len(), count);

    let ids: BTreeSet<&MeetingId> = created.iter().map(|meeting| &meeting.id).collect();
    assert_eq!(ids.len(), count, "ids differ");
    let folders: BTreeSet<&PathBuf> = created.iter().map(|meeting| &meeting.folder).collect();
    assert_eq!(folders.len(), count, "folders differ");

    let mut expected: Vec<String> = (1..=count).map(numbered).collect();
    expected.sort();
    assert_eq!(names_in(store.root()), expected);

    for meeting in created {
        assert_eq!(meeting.title, "Meeting, Oct 27 2:30 PM");
        assert_eq!(
            names_in(&meeting.folder),
            ["notes-timeline.jsonl", "notes.md", "transcript.md"]
        );
        // The folder holds this meeting's id and nobody else's.
        assert!(notes(meeting).contains(&format!("\nid: {}\n", meeting.id)));
        assert_eq!(store.meeting(&meeting.id).unwrap(), *meeting);
    }
}
