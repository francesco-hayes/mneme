//! Collections: folders of meeting folders in the root, shown in the app as
//! folders (issue 036).
//!
//! ```text
//! <root>/
//!   2026-11-24 0930 Standup/        a meeting, in no collection
//!   Acme/                           a collection
//!     2026-11-20 1400 Kickoff/
//! ```
//!
//! A collection is nothing but its folder: there is no list of them on disk,
//! so one made, renamed or removed in Finder is seen by the next scan. Which
//! root subfolders count as collections is decided by the scan (see
//! `list.rs`). Collections do not nest.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::meeting::{MeetingId, MeetingStatus, MeetingSummary, NOTES_FILE};
use crate::naming::{looks_like_meeting_folder, normalize_title, sanitize_title};
use crate::safe_write::sync_dir;
use crate::store::{self, Store, is_taken};

impl Store {
    /// Every collection in the root, empty ones too, sorted by name
    /// ignoring case. The same list [`list_meetings`](Self::list_meetings)
    /// gives in [`MeetingList::collections`](crate::MeetingList::collections).
    pub fn list_collections(&self) -> Result<Vec<String>> {
        Ok(self.list_meetings()?.collections)
    }

    /// Makes a new, empty collection and returns its name as stored.
    ///
    /// The name is made safe for a folder as titles are
    /// ([`sanitize_title`]), so `Acme / Q4` becomes `Acme Q4`. Fails with
    /// [`Error::InvalidCollectionName`] for a name with no visible text or
    /// one that starts like a meeting folder (`2026-10-27 1430 …`), and with
    /// [`Error::FolderExists`] if the name is taken.
    pub fn create_collection(&self, name: &str) -> Result<String> {
        let name = collection_name(name)?;
        let _guard = self.lock();
        let folder = self.root().join(&name);
        match fs::create_dir(&folder) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                return Err(Error::FolderExists { path: folder });
            }
            Err(err) => return Err(Error::io(&folder, err)),
        }
        sync_dir(self.root());
        Ok(name)
    }

    /// Renames a collection and returns the new name as stored. Its meetings
    /// go with it and keep their ids.
    ///
    /// The new name follows the rules of
    /// [`create_collection`](Self::create_collection). Also fails with
    /// [`Error::CollectionNotFound`], and with [`Error::CollectionRecording`]
    /// while a meeting in it is recording (its files are being appended
    /// to). Nothing changes when it fails.
    pub fn rename_collection(&self, name: &str, new_name: &str) -> Result<String> {
        let new_name = collection_name(new_name)?;
        let _guard = self.lock();
        let folder = self.collection_folder_locked(name)?;
        if new_name == name {
            return Ok(new_name);
        }
        if has_recording_meeting(&folder) {
            return Err(Error::CollectionRecording {
                name: name.to_owned(),
            });
        }
        let target = self.root().join(&new_name);
        if is_taken(&folder, &target) {
            return Err(Error::FolderExists { path: target });
        }
        fs::rename(&folder, &target).map_err(|err| Error::io(&folder, err))?;
        sync_dir(self.root());
        // The meetings inside have new paths; their hints are now stale,
        // and every hint is checked before use, so the next lookup scans.
        Ok(new_name)
    }

    /// The folder of a collection, for the caller to show in Finder or move
    /// to the Trash. Fails with [`Error::CollectionNotFound`] if `name` is
    /// not a collection in the root right now.
    pub fn collection_folder(&self, name: &str) -> Result<PathBuf> {
        let _guard = self.lock();
        self.collection_folder_locked(name)
    }

    /// Whether a meeting in this collection has `status: recording` in its
    /// frontmatter.
    pub fn collection_is_recording(&self, name: &str) -> Result<bool> {
        let _guard = self.lock();
        Ok(has_recording_meeting(&self.collection_folder_locked(name)?))
    }

    /// Moves a meeting into a collection, or out to the root with `None`,
    /// and returns it as it is afterwards. The meeting folder keeps its
    /// name; the meeting keeps its id.
    ///
    /// Moving to where it already is changes nothing. Fails with
    /// [`Error::Recording`] while the meeting records, with
    /// [`Error::CollectionNotFound`], and with [`Error::FolderExists`] when
    /// the target already has a folder of that name. Nothing changes when
    /// it fails.
    pub fn move_meeting(&self, id: &MeetingId, collection: Option<&str>) -> Result<MeetingSummary> {
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        let mut summary = store::load(&folder)?.summary;
        let parent = match collection {
            Some(name) => self.collection_folder_locked(name)?,
            None => self.root().to_owned(),
        };
        if summary.folder.parent() == Some(parent.as_path()) {
            return Ok(summary);
        }
        if summary.status == MeetingStatus::Recording {
            return Err(Error::Recording { id: summary.id });
        }
        let Some(file_name) = summary.folder.file_name() else {
            return Err(Error::MeetingNotFound { id: summary.id });
        };
        let target = parent.join(file_name);
        if is_taken(&summary.folder, &target) {
            return Err(Error::FolderExists { path: target });
        }
        fs::rename(&summary.folder, &target).map_err(|err| Error::io(&summary.folder, err))?;
        if let Some(old_parent) = summary.folder.parent() {
            sync_dir(old_parent);
        }
        sync_dir(&parent);
        self.hints.moved(&summary.id, &target);
        summary.folder = target;
        Ok(summary)
    }

    /// The folder of the collection `name`. The caller holds the lock.
    ///
    /// `name` must be exactly a collection's folder name as listed: one path
    /// component, not hidden, a folder (not a link) with no `notes.md`, and
    /// not named like a meeting folder. Anything else is
    /// [`Error::CollectionNotFound`], so a name can never reach outside the
    /// root.
    pub(crate) fn collection_folder_locked(&self, name: &str) -> Result<PathBuf> {
        let not_found = || Error::CollectionNotFound {
            name: name.to_owned(),
        };
        if name.is_empty()
            || name.starts_with('.')
            || name.contains(['/', '\\', '\0'])
            || looks_like_meeting_folder(name)
        {
            return Err(not_found());
        }
        let folder = self.root().join(name);
        match fs::symlink_metadata(&folder) {
            Ok(meta) if meta.is_dir() => {}
            _ => return Err(not_found()),
        }
        if fs::symlink_metadata(folder.join(NOTES_FILE)).is_ok() {
            return Err(not_found());
        }
        Ok(folder)
    }
}

/// The name a new or renamed collection gets, or why it cannot have one.
fn collection_name(name: &str) -> Result<String> {
    if normalize_title(name).is_empty() {
        return Err(Error::InvalidCollectionName {
            reason: "A folder name cannot be empty.",
        });
    }
    let name = sanitize_title(name);
    if looks_like_meeting_folder(&name) {
        return Err(Error::InvalidCollectionName {
            reason: "A folder name cannot start with a date and time like a meeting's (2026-10-27 1430).",
        });
    }
    Ok(name)
}

/// Whether any meeting folder directly in `collection` is recording.
fn has_recording_meeting(collection: &std::path::Path) -> bool {
    let Ok(entries) = fs::read_dir(collection) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|kind| kind.is_dir())
            && store::load(&entry.path())
                .is_ok_and(|open| open.summary.status == MeetingStatus::Recording)
    })
}
