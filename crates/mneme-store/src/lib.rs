//! Meeting folders, transcript, notes and settings on disk (PRD 04).
//!
//! Everything here is plain files; there is no database. Each meeting is a
//! folder under a root the caller chooses:
//!
//! ```text
//! <root>/
//!   2026-10-27 0930 Standup/
//!     notes.md               frontmatter + the user's notes
//!     transcript.md          [00:01:23] Them: ...
//!     notes-timeline.jsonl   {"t":"00:04:12","text":"- ask Ana about the API"}
//!   2026-10-27 1430 Meeting/    default title kept
//!   2026-10-27 1430 Meeting 2/  another meeting started in the same minute
//! ```
//!
//! The folders are the source of truth. A meeting's identity is the `id` in
//! the frontmatter of its `notes.md`, never the folder name.
//!
//! This crate does not know about Tauri or about where the app keeps its
//! data: every path is passed in. It does not read the clock or the time zone
//! either, except in [`StartTime::now_local`].
//!
//! # Layout of the crate
//!
//! - `store`: [`Store`], the entry point: create, look up, rename and finish
//!   meetings.
//! - `meeting`: [`MeetingSummary`], [`MeetingId`], [`MeetingStatus`] and how
//!   they map to frontmatter keys.
//! - `frontmatter`: the line-preserving reader and writer for `notes.md`.
//! - `naming`: folder names and [`sanitize_title`].
//! - `time`: [`StartTime`].
//! - `safe_write`: [`safe_write`], the temp-file-and-rename helper every
//!   whole-file write goes through.
//! - `append`: what the two files that grow a line at a time share: open for
//!   append, start on a fresh line after one that was cut off.
//! - `transcript`: [`TranscriptWriter`], which appends one line per segment
//!   and sorts the file at the end, and [`Transcript`], the parser.
//! - `notes`: reading and saving the notes body ([`Store::notes`],
//!   [`Store::save_notes`]) and the note timeline
//!   ([`Store::save_notes_at`]).
//!
//! Listing and recovery, and the settings file, each get their own module
//! next to these.

mod append;
mod error;
mod frontmatter;
mod meeting;
mod naming;
mod notes;
mod safe_write;
mod store;
mod time;
mod transcript;

pub use error::{Error, FrontmatterError, Result};
pub use meeting::{
    MeetingId, MeetingStatus, MeetingSummary, NOTES_FILE, TIMELINE_FILE, TRANSCRIPT_FILE,
};
pub use naming::{DEFAULT_FOLDER_TITLE, sanitize_title};
pub use notes::SavedNotes;
pub use safe_write::{is_temp_file_name, safe_write};
pub use store::{FinishedMeeting, NewMeeting, Store};
pub use time::{ParseStartTimeError, StartTime};
pub use transcript::{
    Transcript, TranscriptLine, TranscriptStats, TranscriptWriter, count_transcript_lines,
    format_transcript_line,
};
