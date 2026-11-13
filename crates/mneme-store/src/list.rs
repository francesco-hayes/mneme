//! Listing the meetings under the root, and finding one by its id.
//!
//! Both come from one scan: read the root's direct subfolders and the
//! frontmatter of each `notes.md`. Nothing about the meetings is kept on
//! disk besides the folders themselves, so a title changed in another editor
//! or a folder renamed in Finder shows up on the next call.
//!
//! # What is in the root
//!
//! | Entry | In the list | Warning |
//! | --- | --- | --- |
//! | Folder whose `notes.md` has readable frontmatter | yes | none |
//! | Folder with no `notes.md` | no | [`SkipReason::NoNotesFile`] |
//! | Folder whose `notes.md` cannot be read or has broken frontmatter | no | [`SkipReason::Unreadable`] |
//! | Second folder with an id another folder has | no | [`SkipReason::DuplicateId`] |
//! | A file or a link | no | [`SkipReason::NotAFolder`] |
//! | A hidden file or link (`.DS_Store`) | no | none |
//! | A hidden folder with no `notes.md` (`.obsidian`) | no | none |
//!
//! A scan fails only when the root itself cannot be read. Whatever is wrong
//! with one entry is a warning about that entry.
//!
//! # The folder hints
//!
//! Finding a meeting by id means reading every `notes.md` until the id turns
//! up, and every notes save starts with that. So the store remembers, in
//! memory only, where each id was at the last scan ([`FolderHints`]). A hint
//! is never trusted: before it is used, the `notes.md` in the hinted folder
//! is read again and must still carry the id. If it does not (the folder was
//! renamed, deleted or replaced) the root is scanned as before. The hints are
//! thrown away and rebuilt by every scan, so they hold nothing a scan would
//! not find.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::error::{Error, Result};
use crate::frontmatter::NotesFile;
use crate::meeting::{KEY_ID, Meeting, MeetingId, MeetingSummary, NOTES_FILE};
use crate::store::{self, OpenNotes, Store};
use crate::transcript::count_transcript_lines;

/// What [`Store::list_meetings`] found under the root.
#[derive(Debug)]
pub struct MeetingList {
    /// Every readable meeting, newest first.
    ///
    /// The order is by the instant in `date`, so a meeting recorded in
    /// another time zone is placed by when it happened, not by the wall
    /// clock in its folder name. Meetings with exactly the same instant are
    /// ordered by folder name, last name first: of two meetings started in
    /// the same second, `… Meeting 2` comes before `… Meeting`.
    pub meetings: Vec<MeetingSummary>,
    /// Everything in the root that is not in `meetings`, with the reason.
    /// Sorted by path. Empty for a tidy root.
    pub warnings: Vec<ListWarning>,
}

/// One entry of the root that a scan left out, and why.
///
/// Its `Display` is one line naming the path and the reason, for a log.
#[derive(Debug)]
pub struct ListWarning {
    /// The entry in the root this is about: a folder or a file. The root
    /// itself when an entry could not even be named.
    pub path: PathBuf,
    pub reason: SkipReason,
}

/// Why an entry of the root is not in the list.
#[derive(Debug)]
#[non_exhaustive]
pub enum SkipReason {
    /// A file or a link directly in the root. Meetings are real folders.
    NotAFolder,
    /// A folder without a `notes.md`. A crash between creating a meeting's
    /// folder and writing its `notes.md` leaves one of these; nothing in
    /// this crate deletes it.
    NoNotesFile,
    /// The folder's `notes.md` could not be read, or its frontmatter could
    /// not be understood. The error names the file and what is wrong.
    Unreadable(Error),
    /// The folder's `notes.md` carries an id that another folder has too.
    /// One meeting cannot be in two folders, so only one of them is listed,
    /// and every call that takes this id fails with [`Error::DuplicateId`]
    /// until the user removes a folder or changes an id.
    DuplicateId {
        id: MeetingId,
        /// The folder that stands for this id: the first by name whose
        /// frontmatter is readable, which is the one in the list. If none is
        /// readable, the first by name.
        listed: PathBuf,
    },
}

impl fmt::Display for ListWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::NotAFolder => f.write_str("not a folder, so not a meeting"),
            SkipReason::NoNotesFile => f.write_str("the folder has no notes.md"),
            SkipReason::Unreadable(Error::Io { source, .. }) => {
                write!(f, "notes.md cannot be read: {source}")
            }
            SkipReason::Unreadable(Error::Frontmatter { source, .. }) => {
                write!(f, "notes.md: {source}")
            }
            SkipReason::Unreadable(other) => write!(f, "{other}"),
            SkipReason::DuplicateId { id, listed } => write!(
                f,
                "has the same meeting id ({id}) as the folder \"{}\"",
                listed
                    .file_name()
                    .unwrap_or(listed.as_os_str())
                    .to_string_lossy()
            ),
        }
    }
}

impl Store {
    /// Lists the meetings under the root, newest first.
    ///
    /// Every call reads the folders as they are now: the root's direct
    /// subfolders and the frontmatter of each `notes.md`. Nothing is cached
    /// on disk and no result is reused, so a meeting added, removed, renamed
    /// or edited outside Mneme is seen by the next call.
    ///
    /// One bad entry never fails the list. Anything in the root that is not
    /// a readable meeting is left out and reported in
    /// [`MeetingList::warnings`] with its path and the reason; see
    /// [`SkipReason`] for the cases. Hidden files, such as `.DS_Store`, and
    /// hidden folders without a `notes.md`, such as `.obsidian`, are passed
    /// over without a warning.
    ///
    /// An id found in more than one folder is listed once, with a warning
    /// for each other folder. Opening that meeting fails with
    /// [`Error::DuplicateId`], which names the folders.
    ///
    /// The order is described at [`MeetingList::meetings`].
    ///
    /// Fails only when the root itself cannot be read.
    pub fn list_meetings(&self) -> Result<MeetingList> {
        let _guard = self.lock();

        let mut meetings = Vec::new();
        let mut scan = scan(self.root(), |open| meetings.push(open.summary))?;

        let extra = scan.report_duplicates(meetings.iter().map(|meeting| meeting.folder.as_path()));
        meetings.retain(|meeting| !extra.contains(&meeting.folder));
        meetings.sort_by(newest_first);

        self.hints.replace(&scan);
        Ok(MeetingList {
            meetings,
            warnings: scan.into_warnings(),
        })
    }

    /// Reads one meeting in full: its summary, the notes body and the number
    /// of transcript lines.
    ///
    /// The three are read under one lock and from one lookup of the folder,
    /// so they belong to the same moment. The notes are what
    /// [`notes`](Self::notes) returns and the count is what
    /// [`transcript_line_count`](Self::transcript_line_count) returns; a
    /// missing `transcript.md` counts as no lines.
    ///
    /// Fails as [`meeting`](Self::meeting) does, and with an I/O error if
    /// the transcript is there but cannot be read.
    pub fn load_meeting(&self, id: &MeetingId) -> Result<Meeting> {
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        let open = store::load(&folder)?;
        let transcript_line_count = count_transcript_lines(&open.summary.transcript_path())?;
        Ok(Meeting {
            summary: open.summary,
            notes: open.doc.body,
            transcript_line_count,
        })
    }

    /// Finds the folder whose `notes.md` carries this id. The caller holds
    /// the store lock.
    ///
    /// The folder the id was in at the last scan is tried first, and used
    /// only if its `notes.md` still carries the id. Otherwise the root is
    /// scanned, which also finds a folder that was renamed outside Mneme.
    ///
    /// An id in no folder is [`Error::MeetingNotFound`]; an id in several
    /// folders is [`Error::DuplicateId`]. A folder whose frontmatter has the
    /// id but is otherwise unusable is still found, so that the caller's
    /// read of it reports what is wrong with the file.
    pub(crate) fn find_folder(&self, id: &MeetingId) -> Result<PathBuf> {
        if let Some(folder) = self.hints.get(id)
            && folder_has_id(&folder, id)
        {
            return Ok(folder);
        }
        let scan = scan(self.root(), |_| {})?;
        self.hints.replace(&scan);
        scan.resolve(id)
    }
}

/// Newest first by the instant the meeting started; for equal instants, by
/// folder name with the last name first.
fn newest_first(a: &MeetingSummary, b: &MeetingSummary) -> std::cmp::Ordering {
    b.started_at
        .cmp(&a.started_at)
        .then_with(|| b.folder.cmp(&a.folder))
}

/// [`Store::find_folder`] for a caller that has the root but not the store,
/// and so no hints: always a scan.
pub(crate) fn find_folder(root: &Path, id: &MeetingId) -> Result<PathBuf> {
    scan(root, |_| {})?.resolve(id)
}

/// Where each meeting id was at the last scan. In memory only.
///
/// These are hints, not facts: [`Store::find_folder`] reads the hinted
/// folder's `notes.md` again before using one. An id is in here only if the
/// last scan found it in exactly one folder, or if this store created the
/// meeting itself, so a hint can never hide a duplicate that a scan knew of.
#[derive(Debug, Default)]
pub(crate) struct FolderHints(Mutex<HashMap<MeetingId, PathBuf>>);

impl FolderHints {
    fn get(&self, id: &MeetingId) -> Option<PathBuf> {
        self.map().get(id).cloned()
    }

    /// Forgets everything and takes the unambiguous ids of a scan instead.
    pub(crate) fn replace(&self, scan: &Scan) {
        let mut map = self.map();
        map.clear();
        for (id, folders) in &scan.ids {
            if let [folder] = folders.as_slice() {
                map.insert(id.clone(), folder.clone());
            }
        }
    }

    /// Records the folder of a meeting this store has just created. Its id
    /// is new, so no other folder can have it.
    pub(crate) fn created(&self, id: &MeetingId, folder: &Path) {
        self.map().insert(id.clone(), folder.to_owned());
    }

    /// Follows a folder rename this store has just made. Does nothing for an
    /// id without a hint, so an id that a scan found in several folders
    /// stays without one.
    pub(crate) fn moved(&self, id: &MeetingId, folder: &Path) {
        if let Some(known) = self.map().get_mut(id) {
            *known = folder.to_owned();
        }
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<MeetingId, PathBuf>> {
        // A panic while the map was held cannot leave it wrong in a way that
        // matters: every hint is checked against the disk before use.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What one pass over the root found, besides the meetings themselves, which
/// are handed to the caller of [`scan`] one at a time.
#[derive(Debug)]
pub(crate) struct Scan {
    /// Every id seen, with each folder whose frontmatter carries it, sorted
    /// by path. This includes folders whose other keys are unusable.
    ids: HashMap<MeetingId, Vec<PathBuf>>,
    warnings: Vec<ListWarning>,
}

impl Scan {
    /// The one folder with this id.
    fn resolve(&self, id: &MeetingId) -> Result<PathBuf> {
        match self.ids.get(id).map(Vec::as_slice) {
            None | Some([]) => Err(Error::MeetingNotFound { id: id.clone() }),
            Some([folder]) => Ok(folder.clone()),
            Some(folders) => Err(Error::DuplicateId {
                id: id.clone(),
                folders: folders.to_vec(),
            }),
        }
    }

    /// Adds a warning for every folder that shares its id with a folder that
    /// comes before it, and returns those folders: the ones a list leaves
    /// out. `readable` is the folders that were handed to the caller of
    /// [`scan`] as meetings.
    pub(crate) fn report_duplicates<'a>(
        &mut self,
        readable: impl Iterator<Item = &'a Path>,
    ) -> HashSet<PathBuf> {
        let readable: HashSet<&Path> = readable.collect();
        let mut extra = HashSet::new();
        for (id, folders) in &self.ids {
            if folders.len() < 2 {
                continue;
            }
            let listed = folders
                .iter()
                .find(|folder| readable.contains(folder.as_path()))
                .unwrap_or(&folders[0]);
            for folder in folders.iter().filter(|folder| *folder != listed) {
                self.warnings.push(ListWarning {
                    path: folder.clone(),
                    reason: SkipReason::DuplicateId {
                        id: id.clone(),
                        listed: listed.clone(),
                    },
                });
                extra.insert(folder.clone());
            }
        }
        extra
    }

    /// The warnings, sorted by path. A folder with two warnings has the one
    /// about its `notes.md` first.
    pub(crate) fn into_warnings(mut self) -> Vec<ListWarning> {
        // Stable, so the order the two kinds were added in is kept.
        self.warnings.sort_by(|a, b| a.path.cmp(&b.path));
        self.warnings
    }
}

/// Reads every direct subfolder of `root` once. Each folder whose `notes.md`
/// has readable frontmatter is passed to `meeting`, in no particular order;
/// everything else becomes a warning in the result.
///
/// Fails only if the root itself cannot be listed. The caller holds the
/// store lock if it needs the folders to stay put while it works on them.
pub(crate) fn scan(root: &Path, mut meeting: impl FnMut(OpenNotes)) -> Result<Scan> {
    let entries = fs::read_dir(root).map_err(|err| Error::io(root, err))?;

    let mut ids: HashMap<MeetingId, Vec<PathBuf>> = HashMap::new();
    let mut warnings = Vec::new();
    let mut warn = |path: PathBuf, reason: SkipReason| warnings.push(ListWarning { path, reason });

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                warn(
                    root.to_owned(),
                    SkipReason::Unreadable(Error::io(root, err)),
                );
                continue;
            }
        };
        let path = entry.path();
        let hidden = entry.file_name().as_encoded_bytes().first() == Some(&b'.');

        // `file_type` does not follow links: a link to a folder is not a
        // meeting folder. It could point outside the root or at a folder
        // that is already listed.
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(_) => {
                if !hidden {
                    warn(path, SkipReason::NotAFolder);
                }
                continue;
            }
            Err(err) => {
                let reason = SkipReason::Unreadable(Error::io(&path, err));
                warn(path, reason);
                continue;
            }
        }

        match read_folder(&path) {
            Ok(Some(open)) => {
                ids.entry(open.summary.id.clone()).or_default().push(path);
                meeting(open);
            }
            Ok(None) => {
                if !hidden {
                    warn(path, SkipReason::NoNotesFile);
                }
            }
            Err(Broken { id, error }) => {
                if let Some(id) = id {
                    ids.entry(id).or_default().push(path.clone());
                }
                warn(path, SkipReason::Unreadable(error));
            }
        }
    }

    // `read_dir` returns entries in whatever order the file system keeps
    // them. Sorted, "the first folder with this id" means the same thing on
    // every call.
    for folders in ids.values_mut() {
        folders.sort();
    }
    Ok(Scan { ids, warnings })
}

/// A `notes.md` that is there but cannot be read or understood. `id` is set
/// when the frontmatter got as far as giving one.
struct Broken {
    id: Option<MeetingId>,
    error: Error,
}

/// Reads the `notes.md` of one folder: a meeting when its frontmatter is
/// readable, `None` when the folder has no `notes.md` at all.
fn read_folder(folder: &Path) -> Result<Option<OpenNotes>, Broken> {
    let path = folder.join(NOTES_FILE);
    let broken = |id, error| Broken { id, error };

    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(broken(None, Error::io(&path, err))),
    };
    let doc =
        NotesFile::parse(&text).map_err(|err| broken(None, Error::frontmatter(&path, err)))?;
    match MeetingSummary::from_notes(&doc, folder) {
        Ok(summary) => Ok(Some(OpenNotes {
            path,
            text,
            doc,
            summary,
        })),
        Err(err) => Err(broken(id_of(&doc), Error::frontmatter(&path, err))),
    }
}

/// The `id` key of parsed frontmatter, if it holds a value.
fn id_of(doc: &NotesFile) -> Option<MeetingId> {
    match doc.get(KEY_ID) {
        Ok(Some(id)) if !id.is_null() && !id.text.is_empty() => Some(MeetingId::new(id.text)),
        _ => None,
    }
}

/// Whether the `notes.md` in `folder` carries this id right now.
fn folder_has_id(folder: &Path, id: &MeetingId) -> bool {
    let Ok(text) = fs::read_to_string(folder.join(NOTES_FILE)) else {
        return false;
    };
    let Ok(doc) = NotesFile::parse(&text) else {
        return false;
    };
    id_of(&doc).as_ref() == Some(id)
}
