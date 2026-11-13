//! Listing meetings, finding one by id and loading one in full, through the
//! public API, in temporary folders.
//!
//! Start times are fixed values with explicit UTC offsets, so nothing here
//! depends on the clock or on the `TZ` the tests run under.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use mneme_core::{Segment, Speaker};
use mneme_store::{
    Error, FrontmatterError, ListWarning, MeetingId, MeetingList, MeetingStatus, MeetingSummary,
    NewMeeting, SkipReason, Store,
};
use tempfile::TempDir;

/// A store in a fresh temp folder. Keep the `TempDir` alive for the test.
fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("Mneme")).unwrap();
    (dir, store)
}

/// A second `Store` on the same root: what the app has after a restart. It
/// knows nothing the first one learned.
fn reopened(store: &Store) -> Store {
    Store::open(store.root()).unwrap()
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

fn name_of(folder: &Path) -> &str {
    folder.file_name().unwrap().to_str().unwrap()
}

fn ids(list: &MeetingList) -> Vec<&MeetingId> {
    list.meetings.iter().map(|meeting| &meeting.id).collect()
}

fn folder_names(list: &MeetingList) -> Vec<&str> {
    list.meetings
        .iter()
        .map(|meeting| name_of(&meeting.folder))
        .collect()
}

/// Rewrites a meeting's `notes.md` with `from` replaced by `to`, the way
/// another editor would: in place, with no help from the store.
fn edit_notes(meeting: &MeetingSummary, from: &str, to: &str) {
    let text = fs::read_to_string(meeting.notes_path()).unwrap();
    assert!(text.contains(from), "{from:?} not in {text:?}");
    fs::write(meeting.notes_path(), text.replacen(from, to, 1)).unwrap();
}

/// Copies a folder and the files in it, as Finder's Duplicate does.
fn copy_folder(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

/// Every file under `root` with its contents, to prove nothing changed.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let kind = fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_dir() {
                pending.push(path.clone());
                files.push((path, Vec::new()));
            } else if kind.is_symlink() {
                let target = fs::read_link(&path).unwrap();
                files.push((path, target.to_string_lossy().into_owned().into_bytes()));
            } else {
                let contents = fs::read(&path).unwrap();
                files.push((path, contents));
            }
        }
    }
    files.sort();
    files
}

/// The warning about `path`, which must be the only one about it.
fn warning_for<'a>(warnings: &'a [ListWarning], path: &Path) -> &'a ListWarning {
    let mut found = warnings.iter().filter(|warning| warning.path == path);
    let warning = found
        .next()
        .unwrap_or_else(|| panic!("no warning for {}: {warnings:?}", path.display()));
    assert!(
        found.next().is_none(),
        "two warnings for {}",
        path.display()
    );
    warning
}

// --- the list ----------------------------------------------------------------

#[test]
fn an_empty_root_lists_nothing() {
    let (_dir, store) = store();

    let list = store.list_meetings().unwrap();

    assert!(list.meetings.is_empty());
    assert!(list.warnings.is_empty());
}

#[test]
fn the_list_holds_what_reading_each_meeting_returns() {
    let (_dir, store) = store();
    let recording = create_at(&store, "2026-10-27T09:00:00+01:00");
    let finished = create_at(&store, "2026-10-27T10:00:00+01:00");
    let titled = create_at(&store, "2026-10-27T11:00:00+01:00");
    store.finish_meeting(&finished.id, 1800).unwrap();
    store.finish_meeting(&titled.id, 95).unwrap();
    store.rename_meeting(&titled.id, "Q4: planning #2").unwrap();

    let list = store.list_meetings().unwrap();

    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(ids(&list), [&titled.id, &finished.id, &recording.id]);
    for listed in &list.meetings {
        assert_eq!(listed, &store.meeting(&listed.id).unwrap());
    }

    // Every field the Home screen shows, for one of them.
    let listed = &list.meetings[0];
    assert_eq!(listed.title, "Q4: planning #2");
    assert!(listed.title_edited);
    assert_eq!(listed.started_at.to_rfc3339(), "2026-10-27T11:00:00+01:00");
    assert_eq!(listed.duration_secs, Some(95));
    assert_eq!(listed.status, MeetingStatus::Complete);
    assert_eq!(
        listed.folder,
        store.root().join("2026-10-27 1100 Q4 planning #2")
    );
    assert_eq!(list.meetings[2].status, MeetingStatus::Recording);
    assert_eq!(list.meetings[2].duration_secs, None);

    // A store that has never seen these folders gives the same list.
    let again = reopened(&store).list_meetings().unwrap();
    assert_eq!(again.meetings, list.meetings);
}

#[test]
fn meetings_are_ordered_by_date_not_by_folder_name() {
    let (_dir, store) = store();
    let oldest = create_at(&store, "2026-10-25T09:00:00+01:00");
    let middle = create_at(&store, "2026-10-26T09:00:00+01:00");
    let newest = create_at(&store, "2026-10-27T09:00:00+01:00");

    // Folder names that sort the other way round, in both directions of the
    // alphabet, so no ordering by name can give the right answer by luck.
    let rename = |meeting: &MeetingSummary, name: &str| {
        fs::rename(&meeting.folder, store.root().join(name)).unwrap();
    };
    rename(&oldest, "9999-12-31 2359 zz newest by name");
    rename(&middle, "0000-01-01 0000 aa oldest by name");
    rename(&newest, "2026-10-26 1200 in the middle by name");

    let list = store.list_meetings().unwrap();

    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(ids(&list), [&newest.id, &middle.id, &oldest.id]);
    assert_eq!(
        folder_names(&list),
        [
            "2026-10-26 1200 in the middle by name",
            "0000-01-01 0000 aa oldest by name",
            "9999-12-31 2359 zz newest by name",
        ]
    );

    // The date in the file decides, also when it was changed by hand and the
    // folder still carries the old one.
    edit_notes(
        &store.meeting(&oldest.id).unwrap(),
        "date: \"2026-10-25T09:00:00+01:00\"",
        "date: 2026-11-01T08:00:00+01:00",
    );
    let list = store.list_meetings().unwrap();
    assert_eq!(ids(&list), [&oldest.id, &newest.id, &middle.id]);
}

#[test]
fn a_meeting_recorded_in_another_time_zone_is_placed_by_when_it_happened() {
    let (_dir, store) = store();
    // By the wall clock in the folder name and in the text of `date`:
    // tokyo (23:30) is after oslo (14:30) is after seattle (09:00).
    // By the instant: seattle (16:00 UTC), tokyo (14:30 UTC), oslo (13:30 UTC).
    let oslo = create_at(&store, "2026-10-27T14:30:00+01:00");
    let seattle = create_at(&store, "2026-10-27T09:00:00-07:00");
    let tokyo = create_at(&store, "2026-10-27T23:30:00+09:00");
    assert_eq!(name_of(&seattle.folder), "2026-10-27 0900 Meeting");
    assert_eq!(name_of(&tokyo.folder), "2026-10-27 2330 Meeting");

    let list = store.list_meetings().unwrap();

    assert_eq!(ids(&list), [&seattle.id, &tokyo.id, &oslo.id]);

    // Across a date line too: 00:10 on the 28th at +14:00 is 10:10 UTC on
    // the 27th, earlier than all three.
    let kiritimati = create_at(&store, "2026-10-28T00:10:00+14:00");
    assert_eq!(name_of(&kiritimati.folder), "2026-10-28 0010 Meeting");
    let list = store.list_meetings().unwrap();
    assert_eq!(
        ids(&list),
        [&seattle.id, &tokyo.id, &oslo.id, &kiritimati.id]
    );
}

#[test]
fn two_meetings_in_one_minute_are_both_listed_by_their_seconds() {
    let (_dir, store) = store();
    let first = create_at(&store, "2026-10-27T14:30:05+01:00");
    let second = create_at(&store, "2026-10-27T14:30:40+01:00");
    assert_eq!(name_of(&first.folder), "2026-10-27 1430 Meeting");
    assert_eq!(name_of(&second.folder), "2026-10-27 1430 Meeting 2");

    let list = store.list_meetings().unwrap();

    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(ids(&list), [&second.id, &first.id]);
    // Two entries, the same title, different ids.
    assert_eq!(list.meetings[0].title, list.meetings[1].title);
    assert_ne!(list.meetings[0].id, list.meetings[1].id);

    // The folder number is not the order. Here the later meeting has the
    // plain name and the earlier one the number.
    let (_dir, store) = self::store();
    let later = create_at(&store, "2026-10-27T14:30:40+01:00");
    let earlier = create_at(&store, "2026-10-27T14:30:05+01:00");
    assert_eq!(name_of(&later.folder), "2026-10-27 1430 Meeting");
    assert_eq!(name_of(&earlier.folder), "2026-10-27 1430 Meeting 2");

    let list = store.list_meetings().unwrap();

    assert_eq!(ids(&list), [&later.id, &earlier.id]);
}

#[test]
fn meetings_with_the_same_date_are_ordered_by_folder_name_last_first() {
    let (_dir, store) = store();
    let a = create(&store);
    let b = create(&store);
    let c = create(&store);
    // An older and a newer meeting around them.
    let older = create_at(&store, "2026-10-27T14:29:59+01:00");
    let newer = create_at(&store, "2026-10-27T14:30:01+01:00");
    // The same instant written in another zone is still the same instant.
    let same_instant = create_at(&store, "2026-10-27T13:30:00+00:00");
    assert_eq!(name_of(&same_instant.folder), "2026-10-27 1330 Meeting");

    let list = store.list_meetings().unwrap();

    assert_eq!(
        folder_names(&list),
        [
            // One second newer, and the fourth folder of its minute.
            "2026-10-27 1430 Meeting 4",
            "2026-10-27 1430 Meeting 3",
            "2026-10-27 1430 Meeting 2",
            "2026-10-27 1430 Meeting",
            "2026-10-27 1330 Meeting",
            "2026-10-27 1429 Meeting",
        ]
    );
    assert_eq!(
        ids(&list),
        [&newer.id, &c.id, &b.id, &a.id, &same_instant.id, &older.id]
    );

    // The same order every time, from any store.
    for _ in 0..5 {
        assert_eq!(store.list_meetings().unwrap().meetings, list.meetings);
        assert_eq!(
            reopened(&store).list_meetings().unwrap().meetings,
            list.meetings
        );
    }
}

// --- what is not a meeting ---------------------------------------------------

#[test]
fn junk_in_the_root_is_skipped_with_a_warning_for_each_entry() {
    let (_dir, store) = store();
    let root = store.root();
    let good = create_at(&store, "2026-10-27T09:00:00+01:00");
    let bad_status = create_at(&store, "2026-10-27T10:00:00+01:00");
    let bad_date = create_at(&store, "2026-10-27T11:00:00+01:00");
    let unclosed = create_at(&store, "2026-10-27T12:00:00+01:00");
    let stripped = create_at(&store, "2026-10-27T13:00:00+01:00");
    let twice = create_at(&store, "2026-10-27T14:00:00+01:00");
    edit_notes(&bad_status, "status: recording", "status: paused");
    edit_notes(
        &bad_date,
        "date: \"2026-10-27T11:00:00+01:00\"",
        "date: last Tuesday",
    );
    edit_notes(&unclosed, "language: auto\n---\n", "language: auto\n");
    fs::write(stripped.notes_path(), "Just my notes, no frontmatter.\n").unwrap();
    edit_notes(
        &twice,
        "title_edited: false\n",
        "title_edited: false\ntitle: again\n",
    );

    // Plain files in the root.
    let stray = root.join("stray.md");
    fs::write(
        &stray,
        "---\nid: x\ndate: 2026-10-27T09:00:00Z\nstatus: complete\n---\n",
    )
    .unwrap();
    let export = root.join("export.zip");
    fs::write(&export, [0u8, 159, 146, 150]).unwrap();
    // Folders that are not meetings.
    let empty = root.join("2026-10-27 1500 Meeting");
    fs::create_dir(&empty).unwrap();
    let no_notes = root.join("Attachments");
    fs::create_dir(&no_notes).unwrap();
    fs::write(no_notes.join("transcript.md"), "[00:00:01] Me: hi\n").unwrap();
    let binary = root.join("binary notes");
    fs::create_dir(&binary).unwrap();
    fs::write(binary.join("notes.md"), [0xFFu8, 0xFE, 0x00]).unwrap();
    let notes_is_a_folder = root.join("notes is a folder");
    fs::create_dir_all(notes_is_a_folder.join("notes.md")).unwrap();
    // A link to a real meeting folder is not a second meeting.
    #[cfg(unix)]
    let link = {
        let link = root.join("link to a meeting");
        std::os::unix::fs::symlink(&good.folder, &link).unwrap();
        link
    };
    // What macOS and Obsidian leave lying around: nothing to report.
    fs::write(root.join(".DS_Store"), [0u8, 1, 2]).unwrap();
    fs::write(root.join(".hidden.md"), "x").unwrap();
    fs::create_dir(root.join(".obsidian")).unwrap();
    fs::write(root.join(".obsidian").join("app.json"), "{}").unwrap();
    fs::create_dir(root.join(".trash")).unwrap();

    let before = snapshot(root);

    let list = store.list_meetings().unwrap();

    // The one good meeting is listed, exactly as a read by id returns it.
    assert_eq!(list.meetings, [store.meeting(&good.id).unwrap()]);

    // One warning per bad entry, sorted by path, each naming its path.
    let mut expected = vec![
        bad_status.folder.clone(),
        bad_date.folder.clone(),
        unclosed.folder.clone(),
        stripped.folder.clone(),
        twice.folder.clone(),
        stray.clone(),
        export.clone(),
        empty.clone(),
        no_notes.clone(),
        binary.clone(),
        notes_is_a_folder.clone(),
    ];
    #[cfg(unix)]
    expected.push(link.clone());
    expected.sort();
    let warned: Vec<&PathBuf> = list.warnings.iter().map(|warning| &warning.path).collect();
    assert_eq!(warned, expected.iter().collect::<Vec<_>>());
    for warning in &list.warnings {
        let line = warning.to_string();
        assert!(
            line.starts_with(&format!("{}: ", warning.path.display())),
            "{line}"
        );
        assert!(!line.contains('\n'), "{line}");
    }

    // Each with its own reason.
    let reason = |path: &Path| &warning_for(&list.warnings, path).reason;
    for path in [&stray, &export] {
        assert!(matches!(reason(path), SkipReason::NotAFolder), "{path:?}");
    }
    #[cfg(unix)]
    assert!(matches!(reason(&link), SkipReason::NotAFolder));
    for path in [&empty, &no_notes] {
        assert!(matches!(reason(path), SkipReason::NoNotesFile), "{path:?}");
    }
    for path in [&binary, &notes_is_a_folder] {
        assert!(
            matches!(
                reason(path),
                SkipReason::Unreadable(Error::Io { path: file, .. }) if *file == path.join("notes.md")
            ),
            "{:?}",
            reason(path)
        );
    }
    for (meeting, key) in [(&bad_status, "status"), (&bad_date, "date")] {
        assert!(
            matches!(
                reason(&meeting.folder),
                SkipReason::Unreadable(Error::Frontmatter {
                    path,
                    source: FrontmatterError::InvalidValue { key: k, .. },
                }) if *path == meeting.notes_path() && k == key
            ),
            "{:?}",
            reason(&meeting.folder)
        );
        let line = warning_for(&list.warnings, &meeting.folder).to_string();
        assert!(line.contains(key), "{line}");
    }
    assert!(matches!(
        reason(&unclosed.folder),
        SkipReason::Unreadable(Error::Frontmatter {
            source: FrontmatterError::Unterminated,
            ..
        })
    ));
    assert!(matches!(
        reason(&stripped.folder),
        SkipReason::Unreadable(Error::Frontmatter {
            source: FrontmatterError::Missing,
            ..
        })
    ));
    assert!(matches!(
        reason(&twice.folder),
        SkipReason::Unreadable(Error::Frontmatter {
            source: FrontmatterError::DuplicateKey { key },
            ..
        }) if key == "title"
    ));

    // The words a log would show.
    assert_eq!(
        warning_for(&list.warnings, &empty).to_string(),
        format!("{}: the folder has no notes.md", empty.display())
    );
    assert_eq!(
        warning_for(&list.warnings, &stray).to_string(),
        format!("{}: not a folder, so not a meeting", stray.display())
    );
    assert_eq!(
        warning_for(&list.warnings, &bad_status.folder).to_string(),
        format!(
            "{}: notes.md: frontmatter key `status` must be recording, complete or interrupted",
            bad_status.folder.display()
        )
    );

    // Listing reads; it never writes, tidies or deletes.
    assert_eq!(snapshot(root), before);

    // The same from a store that starts cold.
    let again = reopened(&store).list_meetings().unwrap();
    assert_eq!(again.meetings, list.meetings);
    assert_eq!(again.warnings.len(), list.warnings.len());
}

#[test]
fn folders_without_an_id_are_not_duplicates_of_each_other() {
    let (_dir, store) = store();
    let good = create_at(&store, "2026-10-27T09:00:00+01:00");
    let blank: Vec<MeetingSummary> = ["id:", "id: \"\"", "id: ~", "id: null"]
        .iter()
        .enumerate()
        .map(|(n, id_line)| {
            let meeting = create_at(&store, &format!("2026-10-27T1{n}:00:00+01:00"));
            edit_notes(&meeting, &format!("id: {}", meeting.id), id_line);
            meeting
        })
        .collect();

    let list = store.list_meetings().unwrap();

    assert_eq!(ids(&list), [&good.id]);
    // One warning each, about the id, and none about a shared id.
    assert_eq!(list.warnings.len(), blank.len(), "{:?}", list.warnings);
    for (warning, meeting) in list.warnings.iter().zip(&blank) {
        assert_eq!(warning.path, meeting.folder);
        assert!(
            matches!(
                &warning.reason,
                SkipReason::Unreadable(Error::Frontmatter {
                    source: FrontmatterError::InvalidValue { key, .. },
                    ..
                }) if key == "id"
            ),
            "{warning:?}"
        );
    }
    for id in ["", "~", "null"] {
        let err = store.meeting(&MeetingId::new(id)).unwrap_err();
        assert!(
            matches!(err, Error::MeetingNotFound { .. }),
            "{id:?}: {err:?}"
        );
    }
}

#[test]
fn a_hidden_folder_that_holds_a_meeting_is_still_a_meeting() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let hidden = store.root().join(".put away");
    fs::rename(&meeting.folder, &hidden).unwrap();
    // A hidden folder with a `notes.md` that is broken is worth a warning.
    let broken = store.root().join(".broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join("notes.md"), "no frontmatter").unwrap();

    let list = store.list_meetings().unwrap();

    assert_eq!(ids(&list), [&meeting.id]);
    assert_eq!(list.meetings[0].folder, hidden);
    assert_eq!(list.warnings.len(), 1, "{:?}", list.warnings);
    assert_eq!(list.warnings[0].path, broken);
    assert_eq!(store.meeting(&meeting.id).unwrap().folder, hidden);
}

#[test]
fn a_root_that_cannot_be_read_is_the_one_thing_that_fails_the_list() {
    let (_dir, store) = store();
    create(&store);
    fs::remove_dir_all(store.root()).unwrap();

    let err = store.list_meetings().unwrap_err();

    assert!(
        matches!(&err, Error::Io { path, .. } if path == store.root()),
        "{err:?}"
    );
}

// --- duplicate ids -----------------------------------------------------------

#[test]
fn duplicate_ids_are_reported_not_merged() {
    let (_dir, store) = store();
    let other = create_at(&store, "2026-10-26T09:00:00+01:00");
    let original = create(&store);
    store.save_notes(&original.id, "the original\n").unwrap();
    let copy = store.root().join("2026-10-27 1430 Meeting copy");
    copy_folder(&original.folder, &copy);
    fs::write(
        copy.join("notes.md"),
        fs::read_to_string(copy.join("notes.md"))
            .unwrap()
            .replace("the original", "the copy"),
    )
    .unwrap();
    let before = snapshot(store.root());

    // A store that has never looked: the id is ambiguous from the start.
    let cold = reopened(&store);
    let id = &original.id;
    let is_duplicate = |err: &Error| {
        matches!(
            err,
            Error::DuplicateId { id: found, folders }
                if found == id && *folders == [original.folder.clone(), copy.clone()]
        )
    };
    let err = cold.meeting(id).unwrap_err();
    assert!(is_duplicate(&err), "{err:?}");
    assert_eq!(
        err.to_string(),
        format!(
            "meeting id {id} is used by 2 folders: \
             \"2026-10-27 1430 Meeting\", \"2026-10-27 1430 Meeting copy\""
        )
    );

    // Every call that takes the id refuses, and none of them writes.
    let refusals: Vec<Result<(), Error>> = vec![
        cold.load_meeting(id).map(drop),
        cold.notes(id).map(drop),
        cold.save_notes(id, "which one?").map(drop),
        cold.save_notes_at(id, "which one?", 1_000).map(drop),
        cold.rename_meeting(id, "Which one").map(drop),
        cold.finish_meeting(id, 10).map(drop),
        cold.transcript(id).map(drop),
        cold.transcript_line_count(id).map(drop),
        cold.finalize_transcript(id).map(drop),
        cold.transcript_writer(id).map(drop),
    ];
    for (n, result) in refusals.iter().enumerate() {
        let err = result.as_ref().unwrap_err();
        assert!(is_duplicate(err), "call {n}: {err:?}");
    }
    assert_eq!(snapshot(store.root()), before);

    // The list shows the meeting once, from the first folder by name, and
    // says which folder it left out and why.
    let list = cold.list_meetings().unwrap();
    assert_eq!(ids(&list), [id, &other.id]);
    assert_eq!(list.meetings[0].folder, original.folder);
    assert_eq!(list.warnings.len(), 1, "{:?}", list.warnings);
    let warning = &list.warnings[0];
    assert_eq!(warning.path, copy);
    assert!(
        matches!(
            &warning.reason,
            SkipReason::DuplicateId { id: found, listed }
                if found == id && *listed == original.folder
        ),
        "{warning:?}"
    );
    assert_eq!(
        warning.to_string(),
        format!(
            "{}: has the same meeting id ({id}) as the folder \"2026-10-27 1430 Meeting\"",
            copy.display()
        )
    );

    // Listing settled nothing: the id is still refused, and the other
    // meeting is as usable as ever.
    assert!(is_duplicate(&cold.meeting(id).unwrap_err()));
    assert_eq!(cold.meeting(&other.id).unwrap(), other);
    assert_eq!(snapshot(store.root()), before);

    // Once the user removes one copy, the id works again, without a restart.
    fs::remove_dir_all(&original.folder).unwrap();
    assert_eq!(cold.meeting(id).unwrap().folder, copy);
    assert_eq!(cold.notes(id).unwrap(), "the copy\n");
    let list = cold.list_meetings().unwrap();
    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(ids(&list), [id, &other.id]);
}

#[test]
fn a_copy_made_while_the_app_runs_never_takes_over_and_is_caught_by_the_next_list() {
    let (_dir, store) = store();
    // This store created the meeting, so it knows where it is.
    let original = create(&store);
    store.save_notes(&original.id, "before the copy\n").unwrap();
    // The copy sorts before the original by name.
    let copy = store.root().join("2026-10-27 1430 A copy");
    copy_folder(&original.folder, &copy);

    // Until the root is scanned again, calls keep going to the folder the
    // store has been using. They never switch to the copy.
    store.save_notes(&original.id, "after the copy\n").unwrap();
    assert_eq!(store.meeting(&original.id).unwrap().folder, original.folder);
    assert_eq!(
        fs::read_to_string(copy.join("notes.md")).unwrap(),
        fs::read_to_string(original.notes_path())
            .unwrap()
            .replace("after the copy", "before the copy")
    );

    // The next list scans, reports the duplicate, and from then on the id
    // is refused like any other duplicate.
    let list = store.list_meetings().unwrap();
    assert_eq!(list.meetings.len(), 1);
    assert_eq!(list.meetings[0].folder, copy, "first by name is listed");
    assert_eq!(list.warnings.len(), 1);
    assert_eq!(list.warnings[0].path, original.folder);
    for _ in 0..2 {
        let err = store.meeting(&original.id).unwrap_err();
        assert!(matches!(err, Error::DuplicateId { .. }), "{err:?}");
        let err = store.save_notes(&original.id, "x").unwrap_err();
        assert!(matches!(err, Error::DuplicateId { .. }), "{err:?}");
    }
}

#[test]
fn a_duplicate_with_broken_frontmatter_still_makes_the_id_ambiguous() {
    let (_dir, store) = store();
    let original = create(&store);
    // Sorts before the original, and its date is unusable.
    let copy = store.root().join("2026-10-27 1430 A broken copy");
    copy_folder(&original.folder, &copy);
    fs::write(
        copy.join("notes.md"),
        fs::read_to_string(copy.join("notes.md"))
            .unwrap()
            .replace("date: \"2026-10-27T14:30:00+01:00\"", "date: someday"),
    )
    .unwrap();
    let store = reopened(&store);

    let list = store.list_meetings().unwrap();

    // The readable folder is the one listed, although it is second by name.
    assert_eq!(ids(&list), [&original.id]);
    assert_eq!(list.meetings[0].folder, original.folder);
    // The broken copy is reported for both things that are wrong with it.
    assert_eq!(list.warnings.len(), 2, "{:?}", list.warnings);
    assert!(list.warnings.iter().all(|warning| warning.path == copy));
    assert!(matches!(
        list.warnings[0].reason,
        SkipReason::Unreadable(Error::Frontmatter { .. })
    ));
    assert!(matches!(
        &list.warnings[1].reason,
        SkipReason::DuplicateId { listed, .. } if *listed == original.folder
    ));
    // And the id is refused, here as everywhere.
    let err = store.meeting(&original.id).unwrap_err();
    assert!(matches!(err, Error::DuplicateId { .. }), "{err:?}");
}

// --- find by id --------------------------------------------------------------

#[test]
fn a_meeting_is_found_by_id_after_its_folder_is_renamed_outside() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let bystander = create_at(&store, "2026-10-26T09:00:00+01:00");
    store.save_notes(&meeting.id, "first\n").unwrap();
    // The store has used the folder, and listed it: it knows where it was.
    assert_eq!(store.list_meetings().unwrap().meetings.len(), 2);

    let moved = store.root().join("Renamed in Finder");
    fs::rename(&meeting.folder, &moved).unwrap();

    // Found where it is now, by every kind of call.
    assert_eq!(store.meeting(&meeting.id).unwrap().folder, moved);
    assert_eq!(
        store.load_meeting(&meeting.id).unwrap().summary.folder,
        moved
    );
    assert_eq!(store.notes(&meeting.id).unwrap(), "first\n");
    store.save_notes(&meeting.id, "second\n").unwrap();
    assert_eq!(
        fs::read_to_string(moved.join("notes.md"))
            .unwrap()
            .rsplit("---\n")
            .next(),
        Some("second\n")
    );
    // Nothing was put back under the old name.
    assert!(!meeting.folder.exists());

    // Renamed again, and this time another meeting's folder takes the name
    // the store last saw: the name alone must not be believed.
    let moved_again = store.root().join("Renamed twice");
    fs::rename(&moved, &moved_again).unwrap();
    fs::rename(&bystander.folder, &moved).unwrap();

    assert_eq!(store.meeting(&meeting.id).unwrap().folder, moved_again);
    assert_eq!(store.meeting(&bystander.id).unwrap().folder, moved);
    store.save_notes(&meeting.id, "third\n").unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), "third\n");
    assert_eq!(store.notes(&bystander.id).unwrap(), "");

    // Deleted: not found, by a store that knew it.
    fs::remove_dir_all(&moved_again).unwrap();
    let err = store.meeting(&meeting.id).unwrap_err();
    assert!(matches!(err, Error::MeetingNotFound { .. }), "{err:?}");
    assert_eq!(store.meeting(&bystander.id).unwrap().folder, moved);
}

#[test]
fn a_meeting_that_appears_from_outside_is_found_without_a_list() {
    let (_dir, store) = store();
    let (_other_dir, elsewhere) = self::store();
    let here = create(&store);
    let there = create_at(&elsewhere, "2026-10-20T08:00:00+01:00");
    elsewhere
        .save_notes(&there.id, "from another root\n")
        .unwrap();
    // The store has scanned its root and found one meeting.
    assert_eq!(store.list_meetings().unwrap().meetings.len(), 1);

    // A folder is moved in, as a sync or a restore from a backup would do.
    let arrived = store.root().join(name_of(&there.folder));
    copy_folder(&there.folder, &arrived);

    assert_eq!(store.meeting(&there.id).unwrap().folder, arrived);
    assert_eq!(store.notes(&there.id).unwrap(), "from another root\n");
    assert_eq!(store.meeting(&here.id).unwrap(), here);
    assert_eq!(ids(&store.list_meetings().unwrap()), [&here.id, &there.id]);
}

#[test]
fn an_id_changed_in_the_file_is_what_the_meeting_is_found_by() {
    let (_dir, store) = store();
    let meeting = create(&store);
    assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);

    edit_notes(
        &meeting,
        &format!("id: {}", meeting.id),
        "id: \"renumbered by hand\"",
    );

    // The store knew the folder under the old id. The file no longer says
    // so, and the file is what counts.
    let err = store.meeting(&meeting.id).unwrap_err();
    assert!(matches!(err, Error::MeetingNotFound { .. }), "{err:?}");
    let found = store
        .meeting(&MeetingId::new("renumbered by hand"))
        .unwrap();
    assert_eq!(found.folder, meeting.folder);
}

#[test]
fn a_folder_renamed_by_the_store_is_found_by_the_same_store_and_by_others() {
    let (_dir, store) = store();
    let meeting = create(&store);
    store.finish_meeting(&meeting.id, 60).unwrap();
    let observer = reopened(&store);
    assert_eq!(
        observer.meeting(&meeting.id).unwrap().folder,
        meeting.folder
    );

    let renamed = store.rename_meeting(&meeting.id, "Standup").unwrap();

    assert_eq!(name_of(&renamed.folder), "2026-10-27 1430 Standup");
    assert_eq!(store.meeting(&meeting.id).unwrap(), renamed);
    // A store that last saw the old name finds the new one.
    assert_eq!(observer.meeting(&meeting.id).unwrap(), renamed);
    observer
        .save_notes(&meeting.id, "from the observer\n")
        .unwrap();
    assert_eq!(store.notes(&meeting.id).unwrap(), "from the observer\n");
}

#[cfg(unix)]
#[test]
fn a_meeting_the_store_knows_is_found_without_reading_the_whole_root() {
    use std::os::unix::fs::PermissionsExt;

    let (_dir, store) = store();
    let meeting = create(&store);
    let listed = create_at(&store, "2026-10-26T09:00:00+01:00");
    let looked_up = create_at(&store, "2026-10-25T09:00:00+01:00");
    // Three ways a store comes to know a folder: it listed the root, it
    // looked an id up (and so saw every folder on the way), or it created
    // the meeting itself.
    let by_list = reopened(&store);
    assert_eq!(by_list.list_meetings().unwrap().meetings.len(), 3);
    let by_lookup = reopened(&store);
    by_lookup.meeting(&looked_up.id).unwrap();
    let created_here = create_at(&store, "2026-10-28T09:00:00+01:00");

    // A root whose entries can be opened by name but not listed. Anything
    // that scans fails; anything that goes straight to a folder works.
    let listable = fs::metadata(store.root()).unwrap().permissions();
    fs::set_permissions(store.root(), fs::Permissions::from_mode(0o300)).unwrap();
    let scan = store.list_meetings();
    let by_id: Vec<Result<(), Error>> = vec![
        by_list.meeting(&meeting.id).map(drop),
        by_list.load_meeting(&listed.id).map(drop),
        by_list.save_notes(&meeting.id, "saved without a scan\n"),
        by_list.transcript_line_count(&looked_up.id).map(drop),
        by_lookup.meeting(&looked_up.id).map(drop),
        by_lookup.notes(&listed.id).map(drop),
        store.notes(&created_here.id).map(drop),
        store.save_notes(&created_here.id, "also saved\n"),
    ];
    let unknown = store.meeting(&MeetingId::new("no-such-id"));
    fs::set_permissions(store.root(), listable).unwrap();

    if scan.is_ok() {
        // Running as root: permissions do not apply, nothing to test.
        return;
    }
    for (n, result) in by_id.iter().enumerate() {
        assert!(result.is_ok(), "call {n}: {result:?}");
    }
    assert_eq!(store.notes(&meeting.id).unwrap(), "saved without a scan\n");
    assert_eq!(store.notes(&created_here.id).unwrap(), "also saved\n");
    // An id the store has no folder for does need the scan, and says so.
    assert!(
        matches!(&unknown, Err(Error::Io { path, .. }) if path == store.root()),
        "{unknown:?}"
    );
}

// --- load --------------------------------------------------------------------

#[test]
fn loading_a_meeting_gives_its_summary_notes_and_line_count() {
    let (_dir, store) = store();
    let meeting = create(&store);

    // New: no notes, no lines.
    let loaded = store.load_meeting(&meeting.id).unwrap();
    assert_eq!(loaded.summary, meeting);
    assert_eq!(loaded.notes, "");
    assert_eq!(loaded.transcript_line_count, 0);

    let body = "\n# Agenda\r\n- ask Ana about the API\n---\nno newline at the end";
    store.save_notes(&meeting.id, body).unwrap();
    let mut writer = store.transcript_writer(&meeting.id).unwrap();
    for (n, text) in ["one", "two", "three"].into_iter().enumerate() {
        let segment = Segment {
            speaker: Speaker::Them,
            start_ms: n as u64 * 4_000,
            end_ms: n as u64 * 4_000 + 2_000,
            text: text.to_owned(),
        };
        writer.append(&segment).unwrap();
    }
    drop(writer);
    // A line that is not a transcript line and one cut off before its
    // newline: neither is counted.
    let transcript = fs::read_to_string(meeting.transcript_path()).unwrap();
    fs::write(
        meeting.transcript_path(),
        format!("{transcript}not a line\n[00:00:20] Me: cut o"),
    )
    .unwrap();
    let finished = store.finish_meeting(&meeting.id, 20).unwrap().meeting;

    let loaded = store.load_meeting(&meeting.id).unwrap();

    assert_eq!(loaded.summary, finished);
    assert_eq!(loaded.summary, store.meeting(&meeting.id).unwrap());
    assert_eq!(loaded.notes, body);
    assert_eq!(loaded.notes, store.notes(&meeting.id).unwrap());
    assert_eq!(loaded.transcript_line_count, 3);
    assert_eq!(
        loaded.transcript_line_count,
        store.transcript_line_count(&meeting.id).unwrap()
    );
    assert_eq!(
        loaded.transcript_line_count,
        store.transcript(&meeting.id).unwrap().lines.len()
    );

    // A transcript deleted by hand is an empty one.
    fs::remove_file(meeting.transcript_path()).unwrap();
    assert_eq!(
        store
            .load_meeting(&meeting.id)
            .unwrap()
            .transcript_line_count,
        0
    );

    // Loading reads only.
    let before = snapshot(store.root());
    store.load_meeting(&meeting.id).unwrap();
    assert_eq!(snapshot(store.root()), before);
}

#[test]
fn loading_fails_the_way_reading_a_meeting_fails() {
    let (_dir, store) = store();
    let meeting = create(&store);

    let err = store
        .load_meeting(&MeetingId::new("no-such-id"))
        .unwrap_err();
    assert!(matches!(err, Error::MeetingNotFound { .. }), "{err:?}");

    edit_notes(&meeting, "status: recording", "status: paused");
    let err = store.load_meeting(&meeting.id).unwrap_err();
    assert!(
        matches!(&err, Error::Frontmatter { path, .. } if *path == meeting.notes_path()),
        "{err:?}"
    );
}

// --- edits made outside ------------------------------------------------------

#[test]
fn edits_made_outside_show_up_on_the_next_list_and_load() {
    let (_dir, store) = store();
    let meeting = create(&store);
    let other = create_at(&store, "2026-10-26T09:00:00+01:00");
    store.finish_meeting(&meeting.id, 600).unwrap();
    store.save_notes(&meeting.id, "typed in Mneme\n").unwrap();
    // The store has listed and loaded everything once.
    let first = store.list_meetings().unwrap();
    assert_eq!(first.meetings[0].title, "Meeting, Oct 27 2:30 PM");
    assert_eq!(
        store.load_meeting(&meeting.id).unwrap().notes,
        "typed in Mneme\n"
    );

    // In Obsidian: a new title in the properties, a tag, and more notes.
    let text = fs::read_to_string(meeting.notes_path()).unwrap();
    let text = text
        .replace(
            "title: Meeting, Oct 27 2:30 PM\n",
            "title: \"Budget: round two\"\ntags: [work]\n",
        )
        .replace("duration: 600\n", "duration: 615\n")
        .replace("typed in Mneme\n", "typed in Mneme\n- added in Obsidian\n");
    fs::write(meeting.notes_path(), text).unwrap();
    // In a text editor: two more transcript lines.
    fs::write(
        meeting.transcript_path(),
        "[00:00:01] Me: added by hand\n[00:00:02] Them: so was this\n",
    )
    .unwrap();
    // In Finder: the other meeting's folder renamed, a new folder dropped in.
    let moved = store.root().join("Offsite");
    fs::rename(&other.folder, &moved).unwrap();
    let (_elsewhere_dir, elsewhere) = self::store();
    let imported = create_at(&elsewhere, "2026-10-28T08:00:00+01:00");
    copy_folder(&imported.folder, &store.root().join("Imported"));

    let list = store.list_meetings().unwrap();

    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    assert_eq!(ids(&list), [&imported.id, &meeting.id, &other.id]);
    assert_eq!(list.meetings[0].folder, store.root().join("Imported"));
    assert_eq!(list.meetings[1].title, "Budget: round two");
    assert_eq!(list.meetings[1].duration_secs, Some(615));
    // A title typed into the file is not a title edited in the app, so the
    // folder is left alone.
    assert!(!list.meetings[1].title_edited);
    assert_eq!(list.meetings[1].folder, meeting.folder);
    assert_eq!(list.meetings[2].folder, moved);

    let loaded = store.load_meeting(&meeting.id).unwrap();
    assert_eq!(loaded.summary, list.meetings[1]);
    assert_eq!(loaded.notes, "typed in Mneme\n- added in Obsidian\n");
    assert_eq!(loaded.transcript_line_count, 2);

    // Deleted in Finder: gone from the next list.
    fs::remove_dir_all(&moved).unwrap();
    let list = store.list_meetings().unwrap();
    assert_eq!(ids(&list), [&imported.id, &meeting.id]);

    // Nothing was written next to the folders to remember any of this.
    let mut in_root: Vec<String> = fs::read_dir(store.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    in_root.sort();
    assert_eq!(in_root, ["2026-10-27 1430 Meeting", "Imported"]);
}

// --- many meetings, and other threads ---------------------------------------

/// Writes `count` meeting folders straight to disk, without the store and
/// without syncing, one every 37 minutes going back from a fixed time, in a
/// mix of UTC offsets. Returns the ids from newest to oldest.
fn generate_meetings(root: &Path, count: usize) -> Vec<MeetingId> {
    use std::fmt::Write as _;

    const OFFSETS: [&str; 5] = ["+01:00", "-07:00", "+09:00", "+00:00", "+05:30"];
    let newest: mneme_store::StartTime = "2026-10-27T14:30:00+00:00".parse().unwrap();

    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        let unix = newest.unix_seconds() - n as i64 * 37 * 60;
        let offset = OFFSETS[n % OFFSETS.len()];
        let start = utc_text_in_offset(unix, offset);
        let id = MeetingId::new(format!("00000000-0000-4000-8000-{n:012}"));
        // The folder name is deliberately not in date order.
        let folder = root.join(format!("{:04} generated {n}", (n * 7919) % count));
        fs::create_dir(&folder).unwrap();
        let mut notes = String::new();
        write!(
            notes,
            "---\nid: {id}\ntitle: Generated {n}\ntitle_edited: true\ndate: \"{start}\"\n\
             duration: {}\nstatus: complete\nmodel: large-v3-turbo\nlanguage: auto\n---\n",
            60 + n
        )
        .unwrap();
        for line in 0..20 {
            writeln!(notes, "- note line {line} of meeting {n}").unwrap();
        }
        fs::write(folder.join("notes.md"), notes).unwrap();
        fs::write(folder.join("transcript.md"), "[00:00:01] Me: hello\n").unwrap();
        fs::write(folder.join("notes-timeline.jsonl"), "").unwrap();
        ids.push(id);
    }
    ids
}

/// An instant as RFC 3339 text in the given offset (`+hh:mm`), worked out
/// by hand so the test data does not come from the code under test.
fn utc_text_in_offset(unix: i64, offset: &str) -> String {
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let hours: i64 = offset[1..3].parse().unwrap();
    let minutes: i64 = offset[4..6].parse().unwrap();
    let local = unix + sign * (hours * 3600 + minutes * 60);

    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{offset}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[test]
fn five_hundred_meetings_are_listed_in_order() {
    let (_dir, store) = store();
    let expected = generate_meetings(store.root(), 500);

    let started = std::time::Instant::now();
    let list = store.list_meetings().unwrap();
    let took = started.elapsed();

    assert!(list.warnings.is_empty(), "{:?}", &list.warnings[..1]);
    assert_eq!(list.meetings.len(), 500);
    assert_eq!(ids(&list), expected.iter().collect::<Vec<_>>());
    for pair in list.meetings.windows(2) {
        assert!(pair[0].started_at > pair[1].started_at);
    }
    assert_eq!(
        list.meetings[0].started_at.to_rfc3339(),
        "2026-10-27T15:30:00+01:00"
    );
    assert_eq!(list.meetings[1].title, "Generated 1");
    assert_eq!(list.meetings[499].duration_secs, Some(559));

    // Any one of them is found by id and loads.
    let loaded = store.load_meeting(&expected[321]).unwrap();
    assert_eq!(loaded.summary, list.meetings[321]);
    assert!(loaded.notes.starts_with("- note line 0 of meeting 321\n"));
    assert_eq!(loaded.transcript_line_count, 1);

    // Not an assertion: debug build, and the machine may be busy. The
    // number to quote comes from `cargo run --release --example list_bench`.
    println!("listed 500 meetings in {took:?} (debug build)");
}

#[test]
fn the_generated_dates_are_what_they_claim_to_be() {
    // The helper above against two instants worked out by hand.
    // 1_793_111_400 is 2026-10-27T14:30:00Z.
    assert_eq!(
        utc_text_in_offset(1_793_111_400, "+00:00"),
        "2026-10-27T14:30:00+00:00"
    );
    assert_eq!(
        utc_text_in_offset(1_793_111_400, "-07:00"),
        "2026-10-27T07:30:00-07:00"
    );
    assert_eq!(
        utc_text_in_offset(1_793_111_400, "+09:30"),
        "2026-10-28T00:00:00+09:30"
    );
    let parsed: mneme_store::StartTime = "2026-10-27T14:30:00+00:00".parse().unwrap();
    assert_eq!(parsed.unix_seconds(), 1_793_111_400);
}

#[test]
fn listing_while_other_threads_rename_and_create_never_sees_half_a_change() {
    let (_dir, store) = store();
    let meetings: Vec<MeetingSummary> = (0..6)
        .map(|n| {
            let meeting = create_at(&store, &format!("2026-10-2{n}T09:00:00+01:00"));
            store.finish_meeting(&meeting.id, 60).unwrap();
            meeting
        })
        .collect();
    let done = AtomicBool::new(false);

    thread::scope(|scope| {
        let renamer = scope.spawn(|| {
            for round in 0..15 {
                for meeting in &meetings {
                    store
                        .rename_meeting(&meeting.id, &format!("Round {round}"))
                        .unwrap();
                }
            }
            done.store(true, Ordering::Relaxed);
        });
        let creator = scope.spawn(|| {
            // A minute of its own for each, so the count is not bounded by
            // the 999 folders one minute can hold.
            let mut created = 0;
            while !done.load(Ordering::Relaxed) {
                let (day, hour, minute) = (1 + created / 1440, created / 60 % 24, created % 60);
                create_at(
                    &store,
                    &format!("2026-11-{day:02}T{hour:02}:{minute:02}:00+01:00"),
                );
                created += 1;
            }
            created
        });

        let mut lists = 0;
        while !done.load(Ordering::Relaxed) || lists < 3 {
            let list = store.list_meetings().unwrap();
            // A folder caught between its creation and its `notes.md`, or
            // read under a name it no longer has, would be a warning.
            assert!(list.warnings.is_empty(), "{:?}", list.warnings);
            assert!(list.meetings.len() >= meetings.len());
            for meeting in &meetings {
                assert_eq!(
                    list.meetings.iter().filter(|m| m.id == meeting.id).count(),
                    1
                );
            }
            lists += 1;
        }
        renamer.join().unwrap();
        let created = creator.join().unwrap();
        assert_eq!(
            store.list_meetings().unwrap().meetings.len(),
            meetings.len() + created
        );
    });

    for meeting in &meetings {
        assert_eq!(store.meeting(&meeting.id).unwrap().title, "Round 14");
    }
}
