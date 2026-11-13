//! The store: a root folder with one folder per meeting.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::{Error, Result};
use crate::frontmatter::NotesFile;
use crate::meeting::{
    KEY_DURATION, KEY_ID, KEY_STATUS, KEY_TITLE, KEY_TITLE_EDITED, MeetingId, MeetingStatus,
    MeetingSummary, NOTES_FILE, format_bool, format_duration,
};
use crate::naming::{MAX_FOLDER_NUMBER, normalize_title, numbered_folder_name};
use crate::safe_write::{PendingWrite, safe_write, sync_dir};
use crate::time::StartTime;

/// All meetings under one root folder.
///
/// The folders are the source of truth. The store keeps nothing in memory
/// about them: every call reads the files as they are now, so edits made in
/// another editor or in Finder are picked up.
///
/// The store knows nothing about where the root should be; the caller passes
/// the path. Open one `Store` per root and share it (`&Store` is enough, all
/// methods take `&self`, and it is `Send + Sync`): its lock is what keeps
/// two threads from rewriting the same `notes.md` at once.
#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    /// Held across each read-change-write of a `notes.md` and each folder
    /// rename, so concurrent calls cannot lose one another's changes. Shared
    /// with the transcript writers this store hands out, which take it for
    /// each append and for the sort at the end.
    lock: Arc<Mutex<()>>,
}

/// What a new meeting starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMeeting {
    /// When recording started. Use [`StartTime::now_local`] in the app and a
    /// fixed value in tests.
    pub started_at: StartTime,
    /// Id of the model that will transcribe the meeting.
    pub model: Option<String>,
    /// Language setting for transcription, such as `auto` or `en`.
    pub language: Option<String>,
}

/// The result of [`Store::finish_meeting`].
#[derive(Debug)]
pub struct FinishedMeeting {
    /// The meeting as it now is on disk.
    pub meeting: MeetingSummary,
    /// Set when the meeting was closed but the folder rename that was waiting
    /// for it could not be done, for example because a folder with that name
    /// appeared in the meantime. The meeting keeps its old folder and its new
    /// title; a later rename can try again.
    pub rename_error: Option<Error>,
}

impl Store {
    /// Opens the store on `root`, creating the folder if it is missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|err| Error::io(&root, err))?;
        Ok(Self {
            root,
            lock: Arc::new(Mutex::new(())),
        })
    }

    /// The folder that holds the meeting folders.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Creates the folder and files of a meeting that is starting now.
    ///
    /// The folder is `YYYY-MM-DD HHMM Meeting`, from the start time's local
    /// wall clock. It gets a `notes.md` with frontmatter and an empty body,
    /// an empty `transcript.md` and an empty `notes-timeline.jsonl`. The
    /// meeting starts with a fresh id, the default display title, and status
    /// `recording`.
    ///
    /// If that folder name is taken, as it is for a second meeting started
    /// in the same minute, the new folder is `YYYY-MM-DD HHMM Meeting 2`,
    /// or the lowest number from 2 up whose name is free. Whatever has the
    /// taken names is not touched. The number is part of the folder name
    /// only: the title and every other frontmatter value are the same as
    /// for the plain name, and no later call removes the number unless the
    /// title is edited.
    ///
    /// Fails with [`Error::FolderExists`] only if the plain name and every
    /// numbered one up to 999 are taken; nothing is created then.
    pub fn create_meeting(&self, new: NewMeeting) -> Result<MeetingSummary> {
        // Creates on this store run one at a time. Claiming a folder name is
        // atomic without the lock; what the lock adds is that no rename on
        // this store can run while the new folder is still empty. A folder
        // rename replaces an empty folder that has its target name, and the
        // cleanup below would then delete the meeting that was moved there.
        let _guard = self.lock();

        let mut summary = MeetingSummary {
            id: MeetingId::random(),
            title: new.started_at.default_title(),
            title_edited: false,
            started_at: new.started_at,
            duration_secs: None,
            status: MeetingStatus::Recording,
            model: new.model.filter(|model| !model.is_empty()),
            language: new.language.filter(|language| !language.is_empty()),
            folder: PathBuf::new(),
        };
        summary.folder = create_free_folder(&self.root, &summary.expected_folder_name())?;

        if let Err(err) = write_new_meeting_files(&summary) {
            // The folder was created just above and holds nothing but what
            // this call put there.
            let _ = fs::remove_dir_all(&summary.folder);
            return Err(err);
        }
        sync_dir(&self.root);
        Ok(summary)
    }

    /// Reads one meeting by id.
    ///
    /// The folder is found by scanning the root for a `notes.md` with this
    /// id, so it works whatever the folder is called now.
    pub fn meeting(&self, id: &MeetingId) -> Result<MeetingSummary> {
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        Ok(load(&folder)?.summary)
    }

    /// Gives a meeting a new title.
    ///
    /// The title is tidied to one line and saved to frontmatter with
    /// `title_edited: true`. Then the folder is renamed to match, unless the
    /// meeting is recording: a folder is never renamed while files in it are
    /// being appended to, so that rename waits for
    /// [`finish_meeting`](Self::finish_meeting). The returned summary has the
    /// folder as it is after the call.
    ///
    /// Nothing changes, on disk or in frontmatter, when the call fails:
    /// - [`Error::EmptyTitle`] if the title has no visible text;
    /// - [`Error::FolderExists`] if another folder already has the new name
    ///   (checked while recording too, so the problem shows up when the user
    ///   types the title, not at stop).
    pub fn rename_meeting(&self, id: &MeetingId, title: &str) -> Result<MeetingSummary> {
        let title = normalize_title(title);
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }

        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        let mut open = load(&folder)?;

        open.summary.title = title;
        open.summary.title_edited = true;
        if let Some(target) = rename_target(&open.summary)
            && is_taken(&open.summary.folder, &target)
        {
            return Err(Error::FolderExists { path: target });
        }

        open.doc.set_string(KEY_TITLE, &open.summary.title);
        open.doc.set(KEY_TITLE_EDITED, format_bool(true));
        open.save()?;

        if open.summary.status != MeetingStatus::Recording
            && let Err(err) = self.apply_pending_rename(&mut open.summary)
        {
            // Put the old frontmatter back so a failed rename changes
            // nothing.
            let _ = safe_write(&open.path, open.text.as_bytes());
            return Err(err);
        }
        Ok(open.summary)
    }

    /// Closes a meeting when recording stops: sets `duration` and
    /// `status: complete`, then applies a folder rename that was requested
    /// during recording.
    ///
    /// The frontmatter is saved first. If the folder rename then fails, the
    /// meeting is still complete; the failure is reported in
    /// [`FinishedMeeting::rename_error`] instead of as an error.
    ///
    /// Fails with [`Error::NotRecording`] if the meeting is not recording,
    /// so a duration that was already saved is never overwritten.
    pub fn finish_meeting(&self, id: &MeetingId, duration_secs: u64) -> Result<FinishedMeeting> {
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        let open = load(&folder)?;
        if open.summary.status != MeetingStatus::Recording {
            return Err(Error::NotRecording {
                id: open.summary.id,
                status: open.summary.status,
            });
        }
        self.close(open, duration_secs, MeetingStatus::Complete)
    }

    /// Ends a recording with the given final status. The caller holds the
    /// lock.
    fn close(
        &self,
        mut open: OpenNotes,
        duration_secs: u64,
        status: MeetingStatus,
    ) -> Result<FinishedMeeting> {
        open.summary.duration_secs = Some(duration_secs);
        open.summary.status = status;
        open.doc
            .set(KEY_DURATION, &format_duration(open.summary.duration_secs));
        open.doc.set(KEY_STATUS, status.as_str());
        open.save()?;

        let rename_error = self.apply_pending_rename(&mut open.summary).err();
        Ok(FinishedMeeting {
            meeting: open.summary,
            rename_error,
        })
    }

    /// Renames the folder to what the frontmatter says it should be, if the
    /// title was edited and the name differs. Updates `summary.folder`.
    fn apply_pending_rename(&self, summary: &mut MeetingSummary) -> Result<()> {
        let Some(target) = rename_target(summary) else {
            return Ok(());
        };
        if is_taken(&summary.folder, &target) {
            return Err(Error::FolderExists { path: target });
        }
        fs::rename(&summary.folder, &target).map_err(|err| Error::io(&summary.folder, err))?;
        sync_dir(&self.root);
        summary.folder = target;
        Ok(())
    }

    /// Finds the folder whose `notes.md` carries this id. Folders that have
    /// no readable frontmatter are passed over. If the id was copied into
    /// several folders, the first by name wins.
    pub(crate) fn find_folder(&self, id: &MeetingId) -> Result<PathBuf> {
        find_folder(&self.root, id)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        lock(&self.lock)
    }

    /// The store lock itself, for a value that outlives the borrow of the
    /// store, such as a transcript writer on the pipeline's worker thread.
    pub(crate) fn lock_handle(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.lock)
    }
}

/// Takes a store lock.
pub(crate) fn lock(lock: &Mutex<()>) -> MutexGuard<'_, ()> {
    // The mutex guards no data, so a panic in another holder leaves nothing
    // half-updated behind it.
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

/// [`Store::find_folder`] for a caller that has the root but not the store.
pub(crate) fn find_folder(root: &Path, id: &MeetingId) -> Result<PathBuf> {
    let entries = fs::read_dir(root).map_err(|err| Error::io(root, err))?;
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|folder| folder_has_id(folder, id))
        .min()
        .ok_or_else(|| Error::MeetingNotFound { id: id.clone() })
}

/// A `notes.md` read from disk: its path, its exact text, the parsed file and
/// the meeting it describes.
pub(crate) struct OpenNotes {
    pub(crate) path: PathBuf,
    /// The file as it was read. Not updated by a save.
    pub(crate) text: String,
    pub(crate) doc: NotesFile,
    pub(crate) summary: MeetingSummary,
}

impl OpenNotes {
    /// Writes the file back if changing `doc` changed its text.
    pub(crate) fn save(&self) -> Result<()> {
        match self.stage()? {
            Some(pending) => pending.commit().map_err(|err| Error::io(&self.path, err)),
            None => Ok(()),
        }
    }

    /// The first half of [`save`](Self::save): writes the new text to a temp
    /// file beside `notes.md` without replacing it yet. `None` when changing
    /// `doc` did not change the text, so there is nothing to write.
    pub(crate) fn stage(&self) -> Result<Option<PendingWrite>> {
        let rendered = self.doc.render();
        if rendered == self.text {
            return Ok(None);
        }
        PendingWrite::stage(&self.path, rendered.as_bytes())
            .map(Some)
            .map_err(|err| Error::io(&self.path, err))
    }
}

/// Reads the `notes.md` in `folder`. The caller holds the store lock if it
/// is going to write the file back.
pub(crate) fn load(folder: &Path) -> Result<OpenNotes> {
    let path = folder.join(NOTES_FILE);
    let text = fs::read_to_string(&path).map_err(|err| Error::io(&path, err))?;
    let doc = NotesFile::parse(&text).map_err(|err| Error::frontmatter(&path, err))?;
    let summary =
        MeetingSummary::from_notes(&doc, folder).map_err(|err| Error::frontmatter(&path, err))?;
    Ok(OpenNotes {
        path,
        text,
        doc,
        summary,
    })
}

fn folder_has_id(folder: &Path, id: &MeetingId) -> bool {
    let Ok(text) = fs::read_to_string(folder.join(NOTES_FILE)) else {
        return false;
    };
    let Ok(doc) = NotesFile::parse(&text) else {
        return false;
    };
    matches!(doc.get(KEY_ID), Ok(Some(found)) if found.text == id.as_str())
}

/// Creates an empty folder under `root` for a new meeting and returns its
/// path: `name` if that is free, otherwise `name 2`, `name 3` and so on, the
/// lowest number whose name is free.
///
/// There is no "is it free" check to race with. Each name is claimed by
/// creating the folder, which the file system refuses if anything (a folder,
/// a file, a link) already has the name, and a refusal moves on to the next
/// number. So two creates can never end up in one folder, whether they come
/// from this store, another `Store` on the same root or another program, and
/// a folder that appears from outside at the wrong moment only costs one more
/// attempt.
///
/// A gap is filled: with `name` and `name 3` there, the result is `name 2`.
/// Gives up with [`Error::FolderExists`] for the plain name once every name
/// up to [`MAX_FOLDER_NUMBER`] has been refused.
fn create_free_folder(root: &Path, name: &str) -> Result<PathBuf> {
    for number in 1..=MAX_FOLDER_NUMBER {
        let folder = root.join(numbered_folder_name(name, number));
        match fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(Error::io(&folder, err)),
        }
    }
    Err(Error::FolderExists {
        path: root.join(name),
    })
}

/// Writes the three files of a new meeting into its (empty) folder.
/// `notes.md` goes last: a folder is a meeting once it has one, and by then
/// the other two exist.
fn write_new_meeting_files(summary: &MeetingSummary) -> Result<()> {
    for path in [summary.transcript_path(), summary.timeline_path()] {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| Error::io(&path, err))?;
    }
    let notes = summary.notes_path();
    safe_write(&notes, summary.to_new_notes().render().as_bytes())
        .map_err(|err| Error::io(&notes, err))
}

/// Where the folder should move to, or `None` if it should stay.
///
/// Only an edited title moves a folder; the rule is
/// [`MeetingSummary::pending_folder_name`]. With the default title the folder
/// keeps whatever name it has: the one it was created with, numbered or not,
/// or one it was given by hand.
fn rename_target(summary: &MeetingSummary) -> Option<PathBuf> {
    summary
        .pending_folder_name()
        .map(|name| summary.folder.with_file_name(name))
}

/// Whether something other than `folder` itself already has the name
/// `target`. On a file system that ignores case, a rename that only changes
/// capitals finds the folder itself under the new name; that is not a clash.
fn is_taken(folder: &Path, target: &Path) -> bool {
    fs::symlink_metadata(target).is_ok() && !is_same_dir(folder, target)
}

#[cfg(unix)]
fn is_same_dir(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_same_dir(_a: &Path, _b: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("Mneme")).unwrap();
        (dir, store)
    }

    fn create(store: &Store) -> MeetingSummary {
        store
            .create_meeting(NewMeeting {
                started_at: "2026-10-27T14:30:00+01:00".parse().unwrap(),
                model: None,
                language: None,
            })
            .unwrap()
    }

    fn name_of(folder: &Path) -> &str {
        folder.file_name().unwrap().to_str().unwrap()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// What recovery (issue 013) does to a meeting a crash left open: the
    /// same `close` that finish uses, with status `interrupted`.
    fn close_as_interrupted(store: &Store, id: &MeetingId) -> FinishedMeeting {
        let _guard = store.lock();
        let open = load(&store.find_folder(id).unwrap()).unwrap();
        store.close(open, 7, MeetingStatus::Interrupted).unwrap()
    }

    #[test]
    fn closing_an_interrupted_meeting_leaves_a_numbered_folder_where_it_is() {
        let (_dir, store) = store();
        let first = create(&store);
        let second = create(&store);
        assert_eq!(name_of(&second.folder), "2026-10-27 1430 Meeting 2");
        // Even with the plain name free again.
        fs::remove_dir_all(&first.folder).unwrap();

        let closed = close_as_interrupted(&store, &second.id);

        assert!(closed.rename_error.is_none());
        assert_eq!(closed.meeting.folder, second.folder);
        assert_eq!(closed.meeting.status, MeetingStatus::Interrupted);
        assert_eq!(closed.meeting.duration_secs, Some(7));
        assert!(!closed.meeting.title_edited);
        assert_eq!(names_in(store.root()), ["2026-10-27 1430 Meeting 2"]);
    }

    #[test]
    fn closing_an_interrupted_meeting_applies_a_title_typed_before_the_crash() {
        let (_dir, store) = store();
        let _first = create(&store);
        let second = create(&store);
        store.rename_meeting(&second.id, "Standup").unwrap();

        let closed = close_as_interrupted(&store, &second.id);

        assert!(closed.rename_error.is_none());
        assert_eq!(name_of(&closed.meeting.folder), "2026-10-27 1430 Standup");
        assert_eq!(
            names_in(store.root()),
            ["2026-10-27 1430 Meeting", "2026-10-27 1430 Standup"]
        );
    }

    #[test]
    fn a_free_folder_is_the_plain_name_then_the_lowest_free_number() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let made: Vec<PathBuf> = (0..3)
            .map(|_| create_free_folder(root, "name").unwrap())
            .collect();
        assert_eq!(
            made,
            [root.join("name"), root.join("name 2"), root.join("name 3")]
        );

        fs::remove_dir(root.join("name 2")).unwrap();
        assert_eq!(
            create_free_folder(root, "name").unwrap(),
            root.join("name 2")
        );
        assert_eq!(
            create_free_folder(root, "name").unwrap(),
            root.join("name 4")
        );
        // Another name starts its own count.
        assert_eq!(
            create_free_folder(root, "other").unwrap(),
            root.join("other")
        );
        // Each folder is new and empty.
        for folder in names_in(root) {
            assert!(names_in(&root.join(folder)).is_empty());
        }
    }

    #[test]
    fn a_long_title_part_keeps_its_length_and_gets_the_number_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let start: StartTime = "2026-10-27T14:30:00+01:00".parse().unwrap();

        for title in ["long ".repeat(100), "🚀".repeat(500)] {
            let title_part = crate::naming::sanitize_title(&title);
            let name = crate::naming::folder_name(&start, &title_part);
            assert_eq!(create_free_folder(root, &name).unwrap(), root.join(&name));

            let second = create_free_folder(root, &name).unwrap();

            assert_eq!(name_of(&second), format!("{name} 2"));
            assert!(name_of(&second).len() <= 255);
            assert!(second.is_dir());
        }
    }

    #[test]
    fn a_failure_other_than_a_taken_name_is_reported_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let missing_root = dir.path().join("gone");

        let err = create_free_folder(&missing_root, "name").unwrap_err();

        // The first attempt failed for a reason a number cannot fix, so no
        // other name was tried.
        assert!(
            matches!(&err, Error::Io { path, .. } if *path == missing_root.join("name")),
            "{err:?}"
        );
    }
}
