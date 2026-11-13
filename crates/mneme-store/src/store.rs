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
use crate::naming::normalize_title;
use crate::safe_write::{safe_write, sync_dir};
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
    /// Fails with [`Error::FolderExists`] if a folder with that name is
    /// already there (a second meeting started in the same minute); the
    /// existing folder is not touched.
    pub fn create_meeting(&self, new: NewMeeting) -> Result<MeetingSummary> {
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
        summary.folder = self.root.join(summary.expected_folder_name());

        match fs::create_dir(&summary.folder) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::FolderExists {
                    path: summary.folder,
                });
            }
            Err(err) => return Err(Error::io(&summary.folder, err)),
        }

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
struct OpenNotes {
    path: PathBuf,
    text: String,
    doc: NotesFile,
    summary: MeetingSummary,
}

impl OpenNotes {
    /// Writes the file back if changing `doc` changed its text.
    fn save(&self) -> Result<()> {
        let rendered = self.doc.render();
        if rendered == self.text {
            return Ok(());
        }
        safe_write(&self.path, rendered.as_bytes()).map_err(|err| Error::io(&self.path, err))
    }
}

fn load(folder: &Path) -> Result<OpenNotes> {
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
/// Only an edited title moves a folder. With the default title the expected
/// name is the one the folder was created with; if it differs, someone
/// renamed the folder by hand and that is left alone.
fn rename_target(summary: &MeetingSummary) -> Option<PathBuf> {
    if !summary.title_edited {
        return None;
    }
    let target = summary
        .folder
        .with_file_name(summary.expected_folder_name());
    (target != summary.folder).then_some(target)
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
