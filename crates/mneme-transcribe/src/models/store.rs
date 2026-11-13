//! [`ModelStore`]: what is installed in a models folder, and the downloads
//! into it.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use super::catalog::{ModelSpec, catalog};
use super::download::{
    self, CancelToken, DownloadProgress, DownloadState, NetPolicy, PART_SUFFIX, Reporter,
};
use super::error::ModelError;

/// A catalog entry and whether it is installed. Field for field the
/// frontend's `ModelInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub note: String,
    pub english_only: bool,
    pub installed: bool,
}

/// The models in one folder: which catalog entries are installed, where they
/// are, deleting them and downloading new ones.
///
/// A model is **installed** when a file with its final name (for example
/// `ggml-base.bin`) is in the folder and has exactly the catalog's size.
/// Listing never hashes files; the checksum is checked once, when a download
/// finishes, and [`ModelStore::verify`] repeats it on request. A download is
/// written to `<file name>.part` and renamed only after its checksum matched,
/// so a `.part` file is never an installed model.
///
/// Cloning is cheap and every clone shares the same downloads. Keep one store
/// per models folder: opening a second one on the same folder, in this
/// process or another, deletes the `.part` file of a download the first one
/// is running, and that download then fails.
#[derive(Clone)]
pub struct ModelStore {
    inner: Arc<Inner>,
}

struct Inner {
    dir: PathBuf,
    catalog: Vec<ModelSpec>,
    policy: NetPolicy,
    /// The downloads running now, by model id.
    active: Mutex<HashMap<String, CancelToken>>,
}

impl Inner {
    fn active(&self) -> MutexGuard<'_, HashMap<String, CancelToken>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for ModelStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelStore")
            .field("dir", &self.inner.dir)
            .field("models", &self.inner.catalog.len())
            .finish_non_exhaustive()
    }
}

impl ModelStore {
    /// Opens the models folder `dir` with the built-in [`catalog`].
    ///
    /// Creates the folder if it is missing and deletes every `*.part` file in
    /// it: those are downloads that a crash or a quit interrupted. Downloads
    /// do not resume, so an interrupted one starts again from the beginning.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ModelError> {
        Self::open_with_catalog(dir, catalog().to_vec())
    }

    /// Like [`ModelStore::open`] with another catalog, for tests and tools.
    ///
    /// The catalog's ids and file names must be unique, file names must be
    /// plain names that do not end in `.part`, and checksums must be 64 hex
    /// characters. Downloads still only accept HTTPS URLs.
    pub fn open_with_catalog(
        dir: impl AsRef<Path>,
        catalog: Vec<ModelSpec>,
    ) -> Result<Self, ModelError> {
        Self::open_with_policy(dir.as_ref(), catalog, NetPolicy::PRODUCTION)
    }

    pub(crate) fn open_with_policy(
        dir: &Path,
        catalog: Vec<ModelSpec>,
        policy: NetPolicy,
    ) -> Result<Self, ModelError> {
        validate_catalog(&catalog)?;
        fs::create_dir_all(dir).map_err(|e| ModelError::io(dir, &e))?;
        remove_part_files(dir)?;
        Ok(ModelStore {
            inner: Arc::new(Inner {
                dir: dir.to_path_buf(),
                catalog,
                policy,
                active: Mutex::new(HashMap::new()),
            }),
        })
    }

    /// The models folder.
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    /// This store's catalog.
    pub fn catalog(&self) -> &[ModelSpec] {
        &self.inner.catalog
    }

    /// The catalog entry with this id.
    pub fn spec(&self, id: &str) -> Result<&ModelSpec, ModelError> {
        self.inner
            .catalog
            .iter()
            .find(|spec| spec.id == id)
            .ok_or_else(|| ModelError::UnknownModel { id: id.to_owned() })
    }

    /// Every catalog entry, in catalog order, with its installed flag.
    pub fn list(&self) -> Vec<ModelInfo> {
        self.inner
            .catalog
            .iter()
            .map(|spec| ModelInfo {
                id: spec.id.clone(),
                name: spec.name.clone(),
                size_bytes: spec.size_bytes,
                note: spec.note.clone(),
                english_only: spec.english_only,
                installed: self.spec_is_installed(spec),
            })
            .collect()
    }

    /// True if the model is installed (see the type's documentation). False
    /// for an id the catalog does not have.
    pub fn is_installed(&self, id: &str) -> bool {
        self.spec(id).is_ok_and(|spec| self.spec_is_installed(spec))
    }

    /// The file of an installed model, to hand to `WhisperEngine::load`.
    pub fn installed_path(&self, id: &str) -> Result<PathBuf, ModelError> {
        let spec = self.spec(id)?;
        if self.spec_is_installed(spec) {
            Ok(self.final_path(spec))
        } else {
            Err(not_installed(spec))
        }
    }

    /// Deletes a model's file.
    ///
    /// Returns [`ModelError::NotInstalled`] when the folder has no file with
    /// the model's name. A file with the right name and the wrong size does
    /// not count as installed, but it is deleted all the same. A download of
    /// the same model that is still running is not affected.
    pub fn delete(&self, id: &str) -> Result<(), ModelError> {
        let spec = self.spec(id)?;
        let path = self.final_path(spec);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(not_installed(spec)),
            Err(error) => Err(ModelError::io(&path, &error)),
        }
    }

    /// Reads an installed model in full and compares its SHA-256 with the
    /// catalog's. Slow for large models: it reads the whole file.
    pub fn verify(&self, id: &str) -> Result<(), ModelError> {
        let spec = self.spec(id)?;
        let path = self.installed_path(id)?;
        let actual = download::sha256_file(&path).map_err(|e| ModelError::io(&path, &e))?;
        if actual.eq_ignore_ascii_case(&spec.sha256) {
            Ok(())
        } else {
            Err(ModelError::ChecksumMismatch {
                expected: spec.sha256.clone(),
                actual,
            })
        }
    }

    /// Starts downloading a model on a new thread and returns at once.
    ///
    /// `on_progress` is called on that thread: first with `downloading` and
    /// zero bytes, then `downloading` about ten times a second while bytes
    /// arrive and once more with the full size, then `verifying`, then
    /// exactly one of `done`, `cancelled` or `error`. By the time the last
    /// event is delivered the `.part` file is gone, the model is installed
    /// or not, and the same id can be downloaded again.
    ///
    /// Fails without starting anything, and without events, when the id is
    /// unknown ([`ModelError::UnknownModel`]), the model is installed
    /// ([`ModelError::AlreadyInstalled`]) or a download of it is running
    /// ([`ModelError::AlreadyDownloading`]). Different models can download at
    /// the same time.
    pub fn start_download<F>(&self, id: &str, on_progress: F) -> Result<DownloadHandle, ModelError>
    where
        F: FnMut(DownloadProgress) + Send + 'static,
    {
        let spec = self.spec(id)?.clone();
        let cancel = CancelToken::new(&self.inner.policy);
        {
            let mut active = self.inner.active();
            if active.contains_key(&spec.id) {
                return Err(ModelError::AlreadyDownloading {
                    id: spec.id,
                    name: spec.name,
                });
            }
            if self.spec_is_installed(&spec) {
                return Err(ModelError::AlreadyInstalled {
                    id: spec.id,
                    name: spec.name,
                });
            }
            active.insert(spec.id.clone(), cancel.clone());
        }

        let registration = Registration {
            inner: Arc::clone(&self.inner),
            id: spec.id.clone(),
        };
        let model_id = spec.id.clone();
        let worker_cancel = cancel.clone();
        let mut on_progress = on_progress;
        let thread = thread::Builder::new()
            .name(format!("mneme-download-{model_id}"))
            .spawn(move || {
                let inner = Arc::clone(&registration.inner);
                let mut reporter = Reporter::new(&spec, &mut on_progress);
                reporter.send(DownloadState::Downloading, None);
                let result = download::download(
                    &spec,
                    &inner.dir,
                    &inner.policy,
                    &worker_cancel,
                    &mut reporter,
                );
                // The download is over before its last event says so.
                drop(registration);
                match &result {
                    Ok(_) => reporter.send(DownloadState::Done, None),
                    Err(ModelError::Cancelled) => reporter.send(DownloadState::Cancelled, None),
                    Err(error) => reporter.send(DownloadState::Error, Some(error.to_string())),
                }
                result
            })
            // A failed spawn drops the closure, and with it the registration.
            .map_err(|e| ModelError::io(&self.inner.dir, &e))?;

        Ok(DownloadHandle {
            model_id,
            cancel,
            thread,
        })
    }

    /// Cancels the running download of this model. Returns false, and does
    /// nothing, when none is running.
    ///
    /// Returns at once. The download ends with a `cancelled` event when its
    /// `.part` file is gone: straight away while bytes are arriving or the
    /// connection is stalled. If the connection was still being set up, the
    /// event comes when name resolution and the TCP connect finish or time
    /// out (15 s and 20 s). A cancel that arrives after the file was renamed
    /// into place changes nothing; that download is `done`.
    pub fn cancel_download(&self, id: &str) -> bool {
        let cancel = self.inner.active().get(id).cloned();
        match cancel {
            Some(cancel) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// True while a download of this model is running.
    pub fn is_downloading(&self, id: &str) -> bool {
        self.inner.active().contains_key(id)
    }

    /// The ids of the models being downloaded now, in no particular order.
    pub fn active_downloads(&self) -> Vec<String> {
        self.inner.active().keys().cloned().collect()
    }

    fn final_path(&self, spec: &ModelSpec) -> PathBuf {
        self.inner.dir.join(&spec.file_name)
    }

    fn spec_is_installed(&self, spec: &ModelSpec) -> bool {
        file_is_installed(&self.final_path(spec), spec)
    }
}

/// The installed rule: a regular file of exactly the catalog's size.
fn file_is_installed(path: &Path, spec: &ModelSpec) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == spec.size_bytes)
}

/// The file of a model of the built-in [`catalog`] installed in the models
/// folder `dir`, found without opening a [`ModelStore`].
///
/// This only looks: it creates no folder and deletes nothing, so it is safe
/// on a folder another process is downloading into, where
/// [`ModelStore::open`] would remove that download's `.part` file. For
/// command-line tools that take a model by id. The installed rule and the
/// errors are those of [`ModelStore::installed_path`].
pub fn installed_model_path(dir: impl AsRef<Path>, id: &str) -> Result<PathBuf, ModelError> {
    let spec = catalog()
        .iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| ModelError::UnknownModel { id: id.to_owned() })?;
    let path = dir.as_ref().join(&spec.file_name);
    if file_is_installed(&path, spec) {
        Ok(path)
    } else {
        Err(not_installed(spec))
    }
}

/// A running download. Dropping the handle leaves the download running.
#[derive(Debug)]
pub struct DownloadHandle {
    model_id: String,
    cancel: CancelToken,
    thread: JoinHandle<Result<PathBuf, ModelError>>,
}

impl DownloadHandle {
    /// The id of the model being downloaded.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Cancels the download; the same as [`ModelStore::cancel_download`].
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// True once the download has ended, whatever the outcome.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Blocks until the download ends. Returns the installed file's path, or
    /// why nothing was installed ([`ModelError::Cancelled`] after a cancel).
    ///
    /// # Panics
    ///
    /// If the progress callback panicked, with that panic. The download
    /// stopped there and its `.part` file was removed.
    pub fn wait(self) -> Result<PathBuf, ModelError> {
        match self.thread.join() {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

/// Keeps a download in the store's list of running downloads until dropped.
struct Registration {
    inner: Arc<Inner>,
    id: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.inner.active().remove(&self.id);
    }
}

fn not_installed(spec: &ModelSpec) -> ModelError {
    ModelError::NotInstalled {
        id: spec.id.clone(),
        name: spec.name.clone(),
    }
}

fn validate_catalog(catalog: &[ModelSpec]) -> Result<(), ModelError> {
    let invalid = |reason: String| Err(ModelError::InvalidCatalog { reason });
    let mut ids = HashSet::new();
    let mut file_names = HashSet::new();
    for spec in catalog {
        if spec.id.is_empty() {
            return invalid("a model has an empty id".to_owned());
        }
        if !ids.insert(spec.id.as_str()) {
            return invalid(format!("the id `{}` is used twice", spec.id));
        }
        let name = spec.file_name.as_str();
        let is_plain_name =
            !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0']);
        if !is_plain_name {
            return invalid(format!("`{}` has the file name {name:?}", spec.id));
        }
        if name.ends_with(PART_SUFFIX) {
            return invalid(format!(
                "`{}` has a file name ending in {PART_SUFFIX}",
                spec.id
            ));
        }
        if !file_names.insert(name) {
            return invalid(format!("the file name `{name}` is used twice"));
        }
        let is_sha256 =
            spec.sha256.len() == 64 && spec.sha256.bytes().all(|b| b.is_ascii_hexdigit());
        if !is_sha256 {
            return invalid(format!(
                "the checksum of `{}` is not 64 hex characters",
                spec.id
            ));
        }
    }
    Ok(())
}

/// Deletes every `*.part` file directly inside `dir`. A file that cannot be
/// deleted is left: it is never mistaken for a model, and the next download
/// of that model overwrites it.
fn remove_part_files(dir: &Path) -> Result<(), ModelError> {
    let entries = fs::read_dir(dir).map_err(|e| ModelError::io(dir, &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| ModelError::io(dir, &e))?;
        let is_part = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.ends_with(PART_SUFFIX));
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_part && !is_dir {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}
