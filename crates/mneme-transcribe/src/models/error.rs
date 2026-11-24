//! The error type of the model store and the downloader.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Why a model store operation or a download failed.
///
/// The messages are written to be shown to the user as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// The catalog has no model with this id.
    UnknownModel { id: String },
    /// The model is installed, so there is nothing to download. Delete it
    /// first to download it again.
    AlreadyInstalled { id: String, name: String },
    /// A download of this model is already running.
    AlreadyDownloading { id: String, name: String },
    /// The model is not installed, so there is no path to give and nothing to
    /// delete.
    NotInstalled { id: String, name: String },
    /// A catalog passed to the store is not usable.
    InvalidCatalog { reason: String },
    /// A URL, or a redirect on the way to the file, is not HTTPS. Nothing was
    /// sent to it.
    InsecureUrl { url: String },
    /// The server sent a redirect that could not be followed.
    BadRedirect { url: String, reason: String },
    /// More redirects in a row than the downloader follows.
    TooManyRedirects { limit: u32 },
    /// The server answered with something other than the file.
    HttpStatus { status: u16, url: String },
    /// The connection failed, was lost or stalled.
    Network { message: String },
    /// The server's file is not the size the catalog lists. `actual` is the
    /// length the server declared, or the number of bytes it sent.
    SizeMismatch { expected: u64, actual: u64 },
    /// The downloaded bytes do not have the SHA-256 the catalog lists.
    ChecksumMismatch { expected: String, actual: String },
    /// The disk that holds the models folder has less free space than the
    /// model needs plus a margin, so the download was not started.
    NotEnoughSpace {
        id: String,
        name: String,
        size_bytes: u64,
        margin_bytes: u64,
        free_bytes: u64,
    },
    /// The download was cancelled.
    Cancelled,
    /// Reading or writing the models folder failed.
    Io { path: PathBuf, message: String },
}

impl ModelError {
    pub(crate) fn io(path: &Path, error: &io::Error) -> Self {
        ModelError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::UnknownModel { id } => write!(f, "Unknown model: {id}"),
            ModelError::AlreadyInstalled { name, .. } => write!(f, "{name} is already installed."),
            ModelError::AlreadyDownloading { name, .. } => {
                write!(f, "{name} is already downloading.")
            }
            ModelError::NotInstalled { name, .. } => write!(f, "{name} is not installed."),
            ModelError::InvalidCatalog { reason } => write!(f, "invalid model catalog: {reason}"),
            ModelError::InsecureUrl { url } => write!(
                f,
                "refused to download over a connection that is not HTTPS: {url}"
            ),
            ModelError::BadRedirect { url, reason } => {
                write!(
                    f,
                    "the server at {url} sent a redirect that cannot be followed: {reason}"
                )
            }
            ModelError::TooManyRedirects { limit } => {
                write!(f, "the download was redirected more than {limit} times")
            }
            ModelError::HttpStatus { status, url } => {
                write!(f, "the server answered with HTTP status {status} for {url}")
            }
            ModelError::Network { message } => write!(f, "the download failed: {message}"),
            ModelError::SizeMismatch { expected, actual } => write!(
                f,
                "the file on the server has the wrong size: expected {expected} bytes, got {actual}"
            ),
            ModelError::ChecksumMismatch { expected, actual } => write!(
                f,
                "the downloaded file failed its checksum: expected SHA-256 {expected}, got {actual}"
            ),
            ModelError::NotEnoughSpace {
                name,
                size_bytes,
                margin_bytes,
                free_bytes,
                ..
            } => write!(
                f,
                "Not enough disk space to download {name}: it needs {} plus {} to spare, \
                 and only {} is free.",
                human_size(*size_bytes),
                human_size(*margin_bytes),
                human_size(*free_bytes)
            ),
            ModelError::Cancelled => write!(f, "the download was cancelled"),
            ModelError::Io { path, message } => {
                write!(f, "could not use {}: {message}", path.display())
            }
        }
    }
}

impl Error for ModelError {}

/// A size as people read it, in the decimal units Finder uses: `466 MB`,
/// `1.6 GB`.
fn human_size(bytes: u64) -> String {
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;
    if bytes >= GB {
        // One decimal, rounded down, so a size is never overstated.
        format!("{}.{} GB", bytes / GB, bytes % GB / (GB / 10))
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_finder() {
        assert_eq!(human_size(0), "0 KB");
        assert_eq!(human_size(1), "1 KB");
        assert_eq!(human_size(200_000_000), "200 MB");
        assert_eq!(human_size(487_601_967), "487 MB");
        assert_eq!(human_size(1_624_555_275), "1.6 GB");
        assert_eq!(human_size(3_095_033_483), "3.0 GB");
    }

    #[test]
    fn not_enough_space_names_the_model_its_size_and_the_free_space() {
        let error = ModelError::NotEnoughSpace {
            id: "big".to_owned(),
            name: "Big Model".to_owned(),
            size_bytes: 1_624_555_275,
            margin_bytes: 200_000_000,
            free_bytes: 900_000_000,
        };
        assert_eq!(
            error.to_string(),
            "Not enough disk space to download Big Model: it needs 1.6 GB plus 200 MB \
             to spare, and only 900 MB is free."
        );
    }
}
