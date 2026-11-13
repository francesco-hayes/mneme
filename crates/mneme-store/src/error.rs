//! Errors returned by the store.

use std::io;
use std::path::{Path, PathBuf};

use crate::meeting::{MeetingId, MeetingStatus};

/// Result type used across the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why a store operation failed.
///
/// Every variant concerns one file or one meeting; a broken meeting never
/// makes operations on other meetings fail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Reading, writing, creating or renaming `path` failed.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The `notes.md` at `path` has no frontmatter, or frontmatter this crate
    /// cannot read.
    #[error("{}: {source}", path.display())]
    Frontmatter {
        path: PathBuf,
        #[source]
        source: FrontmatterError,
    },

    /// No folder under the root has a readable `notes.md` with this id.
    #[error("no meeting with id {id}")]
    MeetingNotFound { id: MeetingId },

    /// A folder with the wanted name is already there. Nothing was changed.
    ///
    /// Returned when a title the user chose needs a folder name that is
    /// taken. A new meeting does not fail this way on a taken name, it gets a
    /// numbered one, unless every number up to 999 is taken as well.
    #[error("a folder named \"{}\" already exists", folder_name(path))]
    FolderExists { path: PathBuf },

    /// The new title is empty once whitespace is trimmed. Nothing was changed.
    #[error("the title is empty")]
    EmptyTitle,

    /// The operation needs a meeting that is still recording.
    #[error("meeting {id} is not recording (status: {status})")]
    NotRecording {
        id: MeetingId,
        status: MeetingStatus,
    },
}

impl Error {
    pub(crate) fn io(path: &Path, source: io::Error) -> Self {
        Error::Io {
            path: path.to_owned(),
            source,
        }
    }

    pub(crate) fn frontmatter(path: &Path, source: FrontmatterError) -> Self {
        Error::Frontmatter {
            path: path.to_owned(),
            source,
        }
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// What is wrong with the frontmatter of one `notes.md`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FrontmatterError {
    /// The file does not start with a `---` line.
    #[error("the file does not start with a `---` frontmatter block")]
    Missing,

    /// The opening `---` line is never followed by a closing one.
    #[error("the frontmatter block is not closed by a `---` line")]
    Unterminated,

    /// A line inside the block is not `key: value`, a comment, or part of the
    /// value above it. `line` counts from 1 at the opening `---`.
    #[error("frontmatter line {line} is not a `key: value` entry")]
    InvalidLine { line: usize },

    /// The same key is written twice.
    #[error("frontmatter key `{key}` appears more than once")]
    DuplicateKey { key: String },

    /// A key every meeting needs is not there.
    #[error("frontmatter key `{key}` is missing")]
    MissingKey { key: &'static str },

    /// A key Mneme reads holds a value it cannot use. `reason` completes the
    /// sentence "key `x` ...".
    #[error("frontmatter key `{key}` {reason}")]
    InvalidValue { key: String, reason: String },
}
