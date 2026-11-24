//! Collections (folders of meetings, issue 036) through the public API, in
//! temporary folders.

use std::fs;
use std::path::Path;

use mneme_store::{Error, MeetingStatus, MeetingSummary, NewMeeting, SkipReason, Store};
use tempfile::TempDir;

fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    (dir, store)
}

fn new_meeting(start: &str) -> NewMeeting {
    NewMeeting {
        started_at: start.parse().unwrap(),
        model: None,
        language: None,
    }
}

const START: &str = "2026-10-27T14:30:00+01:00";
const LATER: &str = "2026-10-28T09:00:00+01:00";

/// A finished meeting in `collection` (the root for `None`).
fn finished_in(store: &Store, start: &str, collection: Option<&str>) -> MeetingSummary {
    let meeting = store
        .create_meeting_in(new_meeting(start), collection)
        .unwrap();
    store.finish_meeting(&meeting.id, 60).unwrap().meeting
}

fn name_of(folder: &Path) -> &str {
    folder.file_name().unwrap().to_str().unwrap()
}

#[test]
fn a_collections_meetings_are_listed_found_and_know_their_collection() {
    let (_dir, store) = store();
    assert_eq!(store.create_collection("Acme").unwrap(), "Acme");
    let unfiled = finished_in(&store, START, None);
    let filed = finished_in(&store, LATER, Some("Acme"));

    assert_eq!(
        filed.folder,
        store.root().join("Acme/2026-10-28 0900 Meeting")
    );
    assert_eq!(filed.collection(store.root()), Some("Acme"));
    assert_eq!(unfiled.collection(store.root()), None);

    let list = store.list_meetings().unwrap();
    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(list.collections, ["Acme"]);
    let ids: Vec<_> = list.meetings.iter().map(|m| &m.id).collect();
    assert_eq!(ids, [&filed.id, &unfiled.id]);

    // A fresh store (a relaunch) finds it by scanning.
    let reopened = Store::open(store.root()).unwrap();
    assert_eq!(reopened.meeting(&filed.id).unwrap().folder, filed.folder);
    assert_eq!(reopened.load_meeting(&filed.id).unwrap().summary, filed);
}

#[test]
fn empty_collections_are_listed_sorted_ignoring_case() {
    let (_dir, store) = store();
    for name in ["beta", "Alpha", "Gamma"] {
        store.create_collection(name).unwrap();
    }
    // One made in Finder counts too.
    fs::create_dir(store.root().join("delta")).unwrap();

    assert_eq!(
        store.list_collections().unwrap(),
        ["Alpha", "beta", "delta", "Gamma"]
    );
}

#[test]
fn a_date_named_folder_without_notes_is_still_a_warning_not_a_collection() {
    let (_dir, store) = store();
    let leftover = store.root().join("2026-10-27 1430 Meeting");
    fs::create_dir(&leftover).unwrap();
    fs::create_dir(store.root().join(".obsidian")).unwrap();

    let list = store.list_meetings().unwrap();

    assert!(list.collections.is_empty());
    assert_eq!(list.warnings.len(), 1);
    assert_eq!(list.warnings[0].path, leftover);
    assert!(matches!(list.warnings[0].reason, SkipReason::NoNotesFile));
}

#[test]
fn inside_a_collection_other_files_pass_and_nested_folders_warn() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let acme = store.root().join("Acme");
    fs::write(acme.join("agenda.pdf"), b"%PDF").unwrap();
    fs::write(acme.join(".DS_Store"), b"").unwrap();
    fs::create_dir(acme.join("Nested")).unwrap();
    let meeting = finished_in(&store, START, Some("Acme"));

    let list = store.list_meetings().unwrap();

    assert_eq!(list.meetings, [meeting]);
    assert_eq!(list.warnings.len(), 1);
    assert_eq!(list.warnings[0].path, acme.join("Nested"));
    assert!(matches!(list.warnings[0].reason, SkipReason::NoNotesFile));
}

#[test]
fn recovery_reaches_meetings_in_collections() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let left_open = store
        .create_meeting_in(new_meeting(START), Some("Acme"))
        .unwrap();

    // A relaunch: the new store did not create it.
    let relaunched = Store::open(store.root()).unwrap();
    let report = relaunched.recover().unwrap();

    assert_eq!(report.recovered.len(), 1);
    assert_eq!(report.recovered[0].meeting.id, left_open.id);
    assert_eq!(
        relaunched.meeting(&left_open.id).unwrap().status,
        MeetingStatus::Interrupted
    );
}

#[test]
fn creating_in_a_missing_collection_fails_and_creates_nothing() {
    let (_dir, store) = store();

    let err = store
        .create_meeting_in(new_meeting(START), Some("Nope"))
        .unwrap_err();

    assert!(matches!(err, Error::CollectionNotFound { .. }), "{err:?}");
    assert_eq!(fs::read_dir(store.root()).unwrap().count(), 0);
}

#[test]
fn names_that_could_leave_the_root_or_are_meetings_are_not_collections() {
    let (_dir, store) = store();
    let meeting = finished_in(&store, START, None);
    let meeting_name = name_of(&meeting.folder).to_owned();
    fs::write(store.root().join("file"), b"").unwrap();

    for name in [
        "",
        ".",
        "..",
        "../x",
        "a/b",
        ".hidden",
        "file",
        &meeting_name,
    ] {
        let err = store.collection_folder(name).unwrap_err();
        assert!(
            matches!(err, Error::CollectionNotFound { .. }),
            "{name}: {err:?}"
        );
    }
}

#[test]
fn a_meeting_moves_in_between_and_out_keeping_its_id_and_files() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    store.create_collection("Hiring").unwrap();
    let meeting = finished_in(&store, START, None);
    store.save_notes(&meeting.id, "- ask Ana").unwrap();

    let moved = store.move_meeting(&meeting.id, Some("Acme")).unwrap();
    assert_eq!(
        moved.folder,
        store.root().join("Acme").join(name_of(&meeting.folder))
    );
    assert!(!meeting.folder.exists());
    assert_eq!(store.notes(&meeting.id).unwrap(), "- ask Ana");

    let moved = store.move_meeting(&meeting.id, Some("Hiring")).unwrap();
    assert_eq!(moved.collection(store.root()), Some("Hiring"));

    let moved = store.move_meeting(&meeting.id, None).unwrap();
    assert_eq!(moved.folder, meeting.folder);
    assert_eq!(moved.id, meeting.id);
    assert!(
        store
            .root()
            .join("Acme")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );

    // To where it already is: nothing to do.
    assert_eq!(store.move_meeting(&meeting.id, None).unwrap(), moved);
}

#[test]
fn a_recording_meeting_cannot_move() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let live = store.create_meeting(new_meeting(START)).unwrap();

    let err = store.move_meeting(&live.id, Some("Acme")).unwrap_err();

    assert!(matches!(err, Error::Recording { .. }), "{err:?}");
    assert!(live.folder.is_dir());
}

#[test]
fn a_move_onto_a_taken_name_fails_and_changes_nothing() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let meeting = finished_in(&store, START, None);
    let clash = store.root().join("Acme").join(name_of(&meeting.folder));
    fs::create_dir(&clash).unwrap();

    let err = store.move_meeting(&meeting.id, Some("Acme")).unwrap_err();

    assert!(matches!(err, Error::FolderExists { .. }), "{err:?}");
    assert!(meeting.folder.join("notes.md").is_file());
    assert!(clash.read_dir().unwrap().next().is_none());
}

#[test]
fn collection_names_are_made_safe_and_bad_ones_refused() {
    let (_dir, store) = store();

    assert_eq!(store.create_collection("  Acme / Q4  ").unwrap(), "Acme Q4");
    assert!(matches!(
        store.create_collection("Acme Q4").unwrap_err(),
        Error::FolderExists { .. }
    ));
    for bad in ["", "   ", "2026-10-27 1430 Standup"] {
        assert!(
            matches!(
                store.create_collection(bad).unwrap_err(),
                Error::InvalidCollectionName { .. }
            ),
            "{bad:?}"
        );
    }
    assert_eq!(store.list_collections().unwrap(), ["Acme Q4"]);
}

#[test]
fn a_renamed_collection_takes_its_meetings_along() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    store.create_collection("Other").unwrap();
    let meeting = finished_in(&store, START, Some("Acme"));

    assert!(matches!(
        store.rename_collection("Acme", "Other").unwrap_err(),
        Error::FolderExists { .. }
    ));
    assert!(matches!(
        store.rename_collection("Nope", "New").unwrap_err(),
        Error::CollectionNotFound { .. }
    ));

    assert_eq!(
        store.rename_collection("Acme", "Acme Corp").unwrap(),
        "Acme Corp"
    );

    let found = store.meeting(&meeting.id).unwrap();
    assert_eq!(found.collection(store.root()), Some("Acme Corp"));
    assert_eq!(store.list_collections().unwrap(), ["Acme Corp", "Other"]);
}

#[test]
fn a_collection_cannot_be_renamed_while_a_meeting_in_it_records() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let live = store
        .create_meeting_in(new_meeting(START), Some("Acme"))
        .unwrap();

    assert!(store.collection_is_recording("Acme").unwrap());
    let err = store.rename_collection("Acme", "New").unwrap_err();
    assert!(matches!(err, Error::CollectionRecording { .. }), "{err:?}");

    store.finish_meeting(&live.id, 5).unwrap();
    assert!(!store.collection_is_recording("Acme").unwrap());
    store.rename_collection("Acme", "New").unwrap();
}

#[test]
fn a_title_rename_keeps_a_meeting_in_its_collection() {
    let (_dir, store) = store();
    store.create_collection("Acme").unwrap();
    let meeting = finished_in(&store, START, Some("Acme"));

    let renamed = store.rename_meeting(&meeting.id, "Kickoff").unwrap();

    assert_eq!(
        renamed.folder,
        store.root().join("Acme/2026-10-27 1430 Kickoff")
    );
}
