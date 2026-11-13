//! Repairing meetings that a crash left open.
//!
//! A meeting has `status: recording` from the moment it is created until
//! [`Store::finish_meeting`] closes it. If the app is killed in between, the
//! folder stays as it was at that instant: the status still says
//! `recording`, there is no duration, the transcript holds every line that
//! was appended but in arrival order, and its last line may be cut short.
//! [`Store::recover`] closes such meetings the next time the app starts.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::list::{ListWarning, scan};
use crate::meeting::{MeetingStatus, MeetingSummary, NOTES_FILE, TRANSCRIPT_FILE};
use crate::safe_write::is_temp_file_of;
use crate::store::{OpenNotes, Store};
use crate::transcript::{TranscriptStats, finalize_file};

/// What [`Store::recover`] found and did.
#[derive(Debug, Default)]
pub struct RecoveryReport {
    /// Meetings that were `recording` and are now `interrupted`, in folder
    /// name order.
    pub recovered: Vec<RecoveredMeeting>,
    /// Meetings that are still `recording` because repairing them failed.
    /// Each is as it was found, apart from a transcript that may already be
    /// sorted. The next call tries again.
    pub failed: Vec<RecoveryFailure>,
    /// Meetings that are `recording` because this store created them and
    /// has not finished them yet. They are not leftovers and were not
    /// touched. Always empty when `recover` is called where it belongs, at
    /// launch.
    pub in_progress: Vec<MeetingSummary>,
    /// Entries of the root the scan could not read as meetings, exactly as
    /// [`Store::list_meetings`] reports them. A meeting whose frontmatter
    /// is unreadable cannot be recovered and shows up here, not in `failed`.
    pub warnings: Vec<ListWarning>,
}

impl RecoveryReport {
    /// True when there was nothing to repair: no meeting was changed and no
    /// repair failed. What a launch after a clean quit gets, and what a
    /// second call gets after a first one that repaired everything.
    pub fn nothing_to_do(&self) -> bool {
        self.recovered.is_empty() && self.failed.is_empty()
    }
}

/// One meeting that [`Store::recover`] closed.
#[derive(Debug)]
pub struct RecoveredMeeting {
    /// The meeting as it now is on disk: `interrupted`, with a duration, in
    /// the folder it has after any rename.
    pub meeting: MeetingSummary,
    /// What the transcript holds now that it is sorted. The duration is its
    /// last timestamp in whole seconds, or zero when it has no lines.
    pub transcript: TranscriptStats,
    /// Set when the meeting was closed but the folder rename waiting for it
    /// (a title typed before the crash) could not be done. As in
    /// [`FinishedMeeting::rename_error`](crate::FinishedMeeting::rename_error):
    /// the meeting keeps its old folder and its title.
    pub rename_error: Option<Error>,
    /// Temp files of interrupted saves that were deleted from the folder.
    pub removed_temp_files: Vec<PathBuf>,
}

/// One meeting that [`Store::recover`] could not close.
#[derive(Debug)]
pub struct RecoveryFailure {
    /// The meeting as it was found, still `recording`.
    pub meeting: MeetingSummary,
    /// What went wrong; it names the file.
    pub error: Error,
}

impl Store {
    /// Closes every meeting a crash left with `status: recording`.
    ///
    /// # When to call it
    ///
    /// Once, when the app starts, before the first list is shown and before
    /// any meeting is started. On disk a meeting that is being recorded
    /// looks exactly like one whose recording died, so it is the caller's
    /// place in the launch sequence that tells them apart.
    ///
    /// Calling it later is safe as long as there is one `Store` for the
    /// root, which is how a store is meant to be used: a store remembers
    /// the meetings it created and never treats them as leftovers. They are
    /// listed in [`RecoveryReport::in_progress`] and not touched. What the
    /// store cannot see is a recording made through another `Store` value
    /// on the same root or by another running copy of the app; with either
    /// of those, a call in the middle of that recording would close it.
    ///
    /// # What it does
    ///
    /// For each folder whose `notes.md` says `recording`, in folder name
    /// order:
    ///
    /// 1. Sorts `transcript.md` by timestamp, as
    ///    [`TranscriptWriter::finalize`](crate::TranscriptWriter::finalize)
    ///    does. Every line is kept. A last line that was cut off is kept
    ///    too: as a transcript line if what is left of it still reads as
    ///    one, otherwise after the others.
    /// 2. Sets `duration` to the last transcript timestamp in whole seconds,
    ///    or to zero if the transcript has no lines, and `status` to
    ///    `interrupted`.
    /// 3. Renames the folder if a title typed before the crash was still
    ///    waiting for that, by the same rule as
    ///    [`finish_meeting`](Self::finish_meeting). A folder with the
    ///    default title is left where it is.
    /// 4. Deletes the temp files that an interrupted save of `notes.md` or
    ///    `transcript.md` left in the folder
    ///    (`.notes.md.<pid>-<n>.tmp`). Such a file is a save that never
    ///    took effect and may be incomplete; nothing reads it.
    ///
    /// The notes body, the note timeline and every other frontmatter key
    /// are left byte for byte as they were.
    ///
    /// Nothing else is touched: not meetings that are `complete` or
    /// `interrupted`, not their leftover temp files, and not folders
    /// without a readable `notes.md`. In particular the empty folder that a
    /// crash during [`create_meeting`](Self::create_meeting) can leave is
    /// not deleted; it is reported in [`RecoveryReport::warnings`], as it
    /// is by every list.
    ///
    /// # Failures
    ///
    /// One meeting failing does not stop the others. The steps run in the
    /// order above and the status changes in step 2, in one atomic
    /// replacement of `notes.md`, so a meeting that fails before that is
    /// still `recording` with all its data and is tried again by the next
    /// call; it is reported in [`RecoveryReport::failed`]. A failure after
    /// that (the rename) leaves a recovered meeting and is reported with
    /// it. A crash in the middle of recovery is no different: the next
    /// launch finds the meeting still `recording`, or already closed.
    ///
    /// Calling it again changes nothing: there is no `recording` meeting
    /// left to find, and the result has
    /// [`nothing_to_do`](RecoveryReport::nothing_to_do).
    ///
    /// Fails only when the root itself cannot be read.
    pub fn recover(&self) -> Result<RecoveryReport> {
        // Held for the whole pass, like any other read-change-write. This is
        // why the steps below use the crate's lock-free pieces and never
        // call back into the store's public methods: the lock is not
        // re-entrant.
        let _guard = self.lock();

        let mut open_meetings: Vec<OpenNotes> = Vec::new();
        let mut readable: Vec<PathBuf> = Vec::new();
        let mut scan = scan(self.root(), |open| {
            readable.push(open.summary.folder.clone());
            if open.summary.status == MeetingStatus::Recording {
                open_meetings.push(open);
            }
        })?;
        scan.report_duplicates(readable.iter().map(PathBuf::as_path));
        // Before the repairs, so that the renames they make are followed.
        self.hints.replace(&scan);
        open_meetings.sort_by(|a, b| a.summary.folder.cmp(&b.summary.folder));

        let mut report = RecoveryReport {
            warnings: scan.into_warnings(),
            ..RecoveryReport::default()
        };
        for open in open_meetings {
            if self.created_here(&open.summary.id) {
                report.in_progress.push(open.summary);
                continue;
            }
            let found = open.summary.clone();
            match self.recover_one(open) {
                Ok(recovered) => report.recovered.push(recovered),
                Err(error) => report.failed.push(RecoveryFailure {
                    meeting: found,
                    error,
                }),
            }
        }
        Ok(report)
    }

    /// Closes one meeting that was found `recording`. The caller holds the
    /// store lock.
    ///
    /// The meeting is worked on through its folder, not its id, so a folder
    /// that shares its id with another is repaired like any other.
    fn recover_one(&self, open: OpenNotes) -> Result<RecoveredMeeting> {
        // The sort comes first. If it fails, the status has not changed and
        // the next launch starts over; if the status changed first, nothing
        // would ever come back to sort the file.
        let transcript = finalize_file(&open.summary.transcript_path())?;
        let duration_secs = transcript.last_timestamp_ms.unwrap_or(0) / 1000;

        let closed = self.close(open, duration_secs, MeetingStatus::Interrupted)?;

        let removed_temp_files = remove_temp_files(&closed.meeting.folder);
        Ok(RecoveredMeeting {
            meeting: closed.meeting,
            transcript,
            rename_error: closed.rename_error,
            removed_temp_files,
        })
    }
}

/// Deletes the temp files that interrupted safe writes of `notes.md` and
/// `transcript.md` left in `folder`, and returns the ones it deleted, sorted.
///
/// Only plain files whose name is exactly the pattern the safe write helper
/// uses for those two files are deleted; a hidden `.tmp` file of another
/// program is not ours to remove. Best effort: a file that cannot be deleted
/// stays, and is harmless.
fn remove_temp_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut removed: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                is_temp_file_of(name, NOTES_FILE) || is_temp_file_of(name, TRANSCRIPT_FILE)
            })
        })
        .map(|entry| entry.path())
        .filter(|path| fs::remove_file(path).is_ok())
        .collect();
    removed.sort();
    removed
}
