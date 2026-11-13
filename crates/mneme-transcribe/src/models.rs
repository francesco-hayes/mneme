//! The model catalog, the models folder and downloads (PRD 02).
//!
//! - [`catalog`] is the fixed list of Whisper models, with the size and
//!   SHA-256 of each file as the model source lists them.
//! - [`ModelStore`] is a models folder: which entries are installed, their
//!   paths, deleting them, and downloading new ones with progress events and
//!   cancel.
//!
//! ```no_run
//! use mneme_transcribe::{DownloadState, ModelStore, WhisperEngine};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = ModelStore::open("/path/to/app-data/models")?;
//! for model in store.list() {
//!     println!("{} ({} bytes) installed: {}", model.name, model.size_bytes, model.installed);
//! }
//!
//! // Returns at once; the events arrive on the download's own thread.
//! let download = store.start_download("base", |progress| {
//!     println!("{} {}/{}", progress.state, progress.received_bytes, progress.total_bytes);
//!     if progress.state == DownloadState::Error {
//!         println!("{}", progress.error.unwrap_or_default());
//!     }
//! })?;
//! // From any thread: `store.cancel_download("base")`.
//! let path = download.wait()?;
//!
//! let engine = WhisperEngine::load(store.installed_path("base")?)?;
//! # let _ = (path, engine);
//! # Ok(())
//! # }
//! ```

mod catalog;
mod download;
mod error;
mod store;

#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tests;

pub use catalog::{MODEL_HOST, MODEL_REPOSITORY, MODEL_REVISION, ModelSpec, catalog, find_model};
pub use download::{DownloadProgress, DownloadState};
pub use error::ModelError;
pub use store::{DownloadHandle, ModelInfo, ModelStore};
