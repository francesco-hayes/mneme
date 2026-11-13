//! The app's settings, kept in one small JSON file.
//!
//! The file is `settings.json` in the app data folder. This crate does not
//! know where that folder is: [`SettingsStore::open`] is given the path of
//! the file and the default meetings root, both worked out by the app.
//!
//! # The file
//!
//! One JSON object, written through [`safe_write`], two-space indented:
//!
//! ```json
//! {
//!   "defaultModelId": "large-v3-turbo",
//!   "globalShortcut": "CmdOrCtrl+Shift+M",
//!   "language": "en",
//!   "meetingsRoot": "/Users/ana/Notes/Meetings"
//! }
//! ```
//!
//! The file holds only what was set. A key that is missing, `null` or an
//! empty string means "not set", and the setting has its default: no default
//! model, language `en`, no shortcut, the meetings root the app passed in.
//! The default root is never written to the file, so it follows the app (a
//! home folder that moved, say) until the user picks a root of their own.
//!
//! A save changes the keys the patch names and nothing else. Keys this
//! version does not know, at any depth, are written back as they were read,
//! so an older Mneme does not drop the settings of a newer one. They are
//! kept as JSON values, not as text: spacing, key order and the spelling of
//! a number (`1.50` comes back as `1.5`) are not preserved.
//!
//! # A file that cannot be used
//!
//! | The file | Settings | The file afterwards |
//! | --- | --- | --- |
//! | is missing | defaults | still missing: opening never creates it |
//! | is not JSON, is not a JSON object, is empty, or cannot be read | defaults | moved to `settings.json.bad` |
//! | is an object with a known key of the wrong type, or a relative `meetingsRoot` | that one setting has its default, the others are kept | untouched |
//!
//! Each of the last two is reported as a [`SettingsWarning`] by
//! [`SettingsStore::open`], for the app to log.
//!
//! A file moved aside never replaces an earlier one: if `settings.json.bad`
//! is taken the name is `settings.json.bad.2`, then `.bad.3`, up to 999. If
//! the file cannot be moved (a read-only folder), it stays where it is and
//! the store refuses to save over it: [`SettingsStore::update`] tries the
//! move again and fails until it works.
//!
//! A value of the wrong type is left in the file, since a newer version may
//! be the one that wrote it. It stays until that setting is changed here.
//!
//! # Limits
//!
//! - The file is read once, when the store is opened. An edit made by hand
//!   while the app runs is replaced by the next save.
//! - One store per file, in one process: a second store on the same file
//!   would not see the first one's changes.
//! - `~` is not expanded. A meetings root must be an absolute path.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::safe_write::{is_temp_file_of, safe_write, sync_dir};

/// The name the app gives the settings file in its data folder.
pub const SETTINGS_FILE: &str = "settings.json";

/// The language setting when none is chosen. Mneme v1 is English only.
pub const DEFAULT_LANGUAGE: &str = "en";

const KEY_DEFAULT_MODEL_ID: &str = "defaultModelId";
const KEY_LANGUAGE: &str = "language";
const KEY_GLOBAL_SHORTCUT: &str = "globalShortcut";
const KEY_MEETINGS_ROOT: &str = "meetingsRoot";

/// How many names are tried for a file moved aside: `.bad`, then `.bad.2`
/// to `.bad.999`.
const MAX_BAD_FILES: u32 = 999;

/// The settings in effect.
///
/// Serializes with the field names of the frontend contract
/// (`defaultModelId`, `language`, `globalShortcut`) plus `meetingsRoot`,
/// which the contract does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Id of the model new meetings are transcribed with. `None` until one
    /// is chosen, which the app does after the first download.
    pub default_model_id: Option<String>,
    /// A language code such as `en`, or `auto`. Never empty. Whether a model can
    /// transcribe it is for the transcribe crate to say; here it is any
    /// text.
    pub language: String,
    /// The shortcut that starts and stops a recording from anywhere.
    /// `None` by default.
    pub global_shortcut: Option<String>,
    /// The folder that holds the meeting folders. Always an absolute path
    /// that is valid UTF-8: the one in the file, or the default the store
    /// was opened with.
    pub meetings_root: PathBuf,
}

/// What a [`SettingsPatch`] does to one setting.
///
/// Deserializes the way a field of the frontend's `Partial<Settings>`
/// arrives: a value is [`Set`](Patch::Set), `null` is
/// [`Clear`](Patch::Clear). A key that is absent is [`Keep`](Patch::Keep),
/// which the field's `#[serde(default)]` in [`SettingsPatch`] provides, not
/// this type: on its own it reads a missing value as `Clear`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Patch<T> {
    /// Leave the setting as it is.
    #[default]
    Keep,
    /// Back to "not set": no default model, no shortcut, language `en`,
    /// the default meetings root.
    Clear,
    /// Set it to this value.
    Set(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Patch::Set(value),
            None => Patch::Clear,
        })
    }
}

/// A change to some of the settings. The default changes nothing.
///
/// ```
/// use mneme_store::{Patch, SettingsPatch};
///
/// let patch = SettingsPatch {
///     language: Patch::Set("en".to_owned()),
///     global_shortcut: Patch::Clear,
///     ..SettingsPatch::default()
/// };
/// // The same patch as the frontend sends it.
/// let sent: SettingsPatch =
///     serde_json::from_str(r#"{"language": "en", "globalShortcut": null}"#).unwrap();
/// assert_eq!(sent, patch);
/// ```
///
/// Text is trimmed, and text that is then empty counts as
/// [`Patch::Clear`]: an empty string is never stored. So a language of `""`
/// gives `en`. An unknown key in a deserialized patch is an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    #[serde(default)]
    pub default_model_id: Patch<String>,
    #[serde(default)]
    pub language: Patch<String>,
    #[serde(default)]
    pub global_shortcut: Patch<String>,
    /// Must be an absolute path that is valid UTF-8, or the update fails
    /// with [`Error::InvalidSetting`]. Not trimmed. An empty path counts as
    /// [`Patch::Clear`].
    #[serde(default)]
    pub meetings_root: Patch<PathBuf>,
}

/// Something [`SettingsStore::open`] found wrong with the file. None of
/// them stops the store from opening.
///
/// Its `Display` is one line for a log.
#[derive(Debug)]
#[non_exhaustive]
pub enum SettingsWarning {
    /// The file could not be used at all and was moved to `kept_as`. Every
    /// setting has its default.
    Corrupt {
        path: PathBuf,
        /// What is wrong with the file; completes "the file ...".
        reason: String,
        kept_as: PathBuf,
    },
    /// The file could not be used at all and could not be moved aside
    /// either. Every setting has its default, and the store does not save
    /// until the move works: see [`SettingsStore::update`].
    CorruptNotMoved {
        path: PathBuf,
        /// What is wrong with the file; completes "the file ...".
        reason: String,
        /// Why the move failed.
        error: io::Error,
    },
    /// One known key holds a value that cannot be used. That setting has its
    /// default; the others are as the file has them. The value is left in
    /// the file until the setting is changed.
    InvalidValue {
        /// The key in the file.
        key: &'static str,
        reason: String,
    },
    /// A temp file that a save interrupted by a crash left next to the
    /// settings file was deleted. The settings file itself was never
    /// touched by that save.
    RemovedTempFile { path: PathBuf },
}

impl fmt::Display for SettingsWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt {
                path,
                reason,
                kept_as,
            } => write!(
                f,
                "{}: the file {reason}; default settings are in use and the file was kept as \"{}\"",
                path.display(),
                kept_as
                    .file_name()
                    .unwrap_or(kept_as.as_os_str())
                    .to_string_lossy(),
            ),
            Self::CorruptNotMoved {
                path,
                reason,
                error,
            } => write!(
                f,
                "{}: the file {reason}; default settings are in use. The file could not be moved \
                 aside ({error}), so settings are not saved until it can be",
                path.display(),
            ),
            Self::InvalidValue { key, reason } => write!(
                f,
                "setting `{key}`: {reason}; its default is in use and the value stays in the \
                 file until the setting is changed",
            ),
            Self::RemovedTempFile { path } => write!(
                f,
                "{}: removed the temp file of an interrupted save",
                path.display(),
            ),
        }
    }
}

/// The settings file and the settings read from it.
///
/// Open one per file and keep it for the life of the process. It can be
/// shared between threads: updates run one at a time and none is lost.
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    default_meetings_root: PathBuf,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    /// The top-level object of the file: the known keys that are set and
    /// every key this version does not know. Empty when there is no file.
    document: Map<String, Value>,
    /// What `document` means. Derived from it after every change.
    settings: Settings,
    /// True while the file at `path` is one that could not be used and
    /// could not be moved aside. No save replaces it.
    blocked: bool,
}

impl SettingsStore {
    /// Reads the settings file at `path`.
    ///
    /// `default_meetings_root` is the meetings root for as long as the file
    /// does not name one. The app works it out (the user's Documents folder
    /// plus `Mneme`); it must be an absolute path that is valid UTF-8.
    ///
    /// Nothing is created: with no file, every setting has its default and
    /// the folder is left as it is. A file that cannot be used is moved
    /// aside, and one bad value falls back to its default, as the module
    /// documentation lays out. Either is reported in the returned warnings,
    /// as is a temp file of this settings file that a crash left behind and
    /// that was deleted. The warnings are empty for a file in good order.
    ///
    /// # Errors
    ///
    /// Only for arguments that cannot be right, whatever the file holds:
    /// [`Error::InvalidSetting`] when `default_meetings_root` is relative
    /// (an unexpanded `~/Documents/Mneme` is) or not UTF-8, [`Error::Io`]
    /// when `path` does not end in a file name.
    pub fn open(
        path: impl Into<PathBuf>,
        default_meetings_root: impl Into<PathBuf>,
    ) -> Result<(Self, Vec<SettingsWarning>)> {
        let path = path.into();
        let default_meetings_root = default_meetings_root.into();
        if path.file_name().is_none() {
            let reason = "the settings path does not end in a file name";
            return Err(Error::io(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, reason),
            ));
        }
        if let Err(reason) = check_root(&default_meetings_root) {
            return Err(Error::InvalidSetting {
                key: KEY_MEETINGS_ROOT,
                reason: format!("the default passed in {reason}"),
            });
        }

        let mut warnings = Vec::new();
        remove_temp_files(&path, &mut warnings);
        let (document, blocked) = match read_document(&path) {
            Ok(document) => (document, false),
            Err(reason) => match set_aside(&path) {
                Ok(kept_as) => {
                    warnings.push(SettingsWarning::Corrupt {
                        path: path.clone(),
                        reason,
                        kept_as,
                    });
                    (Map::new(), false)
                }
                Err(error) => {
                    warnings.push(SettingsWarning::CorruptNotMoved {
                        path: path.clone(),
                        reason,
                        error,
                    });
                    (Map::new(), true)
                }
            },
        };
        let settings = read_settings(&document, &default_meetings_root, &mut warnings);

        let store = Self {
            path,
            default_meetings_root,
            state: Mutex::new(State {
                document,
                settings,
                blocked,
            }),
        };
        Ok((store, warnings))
    }

    /// The settings file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings in effect: what the last successful
    /// [`update`](Self::update) returned, or what [`open`](Self::open) read.
    pub fn get(&self) -> Settings {
        self.lock().settings.clone()
    }

    /// Applies `patch`, saves the file, and returns the settings now in
    /// effect.
    ///
    /// Each field of the patch keeps, clears or sets one setting; see
    /// [`Patch`] and [`SettingsPatch`]. The save goes through
    /// [`safe_write`], creates the file's folder if it is missing, and
    /// touches only the keys the patch names. A patch that changes nothing
    /// writes nothing, so it does not create the file either.
    ///
    /// # Errors
    ///
    /// On any error nothing changed, in the file or in what
    /// [`get`](Self::get) returns.
    ///
    /// - [`Error::InvalidSetting`]: the patch sets a meetings root that is
    ///   relative or not UTF-8.
    /// - [`Error::Io`]: the file could not be written. Also when a file
    ///   that could not be used is still at the path because moving it
    ///   aside fails: the move is tried again first, and the save happens
    ///   only once it worked.
    pub fn update(&self, patch: &SettingsPatch) -> Result<Settings> {
        let mut state = self.lock();
        let mut document = state.document.clone();
        apply(&mut document, patch)?;
        if document == state.document {
            return Ok(state.settings.clone());
        }

        if state.blocked {
            match set_aside(&self.path) {
                Ok(_) => {}
                // Someone removed it in the meantime; nothing is in the way.
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::io(&self.path, err)),
            }
            state.blocked = false;
        }
        self.save(&document)?;

        // The values that were bad when the file was read are still bad or
        // were just replaced; either way they were reported by `open`.
        state.settings = read_settings(&document, &self.default_meetings_root, &mut Vec::new());
        state.document = document;
        Ok(state.settings.clone())
    }

    fn save(&self, document: &Map<String, Value>) -> Result<()> {
        let mut text = serde_json::to_string_pretty(document)
            .map_err(|err| Error::io(&self.path, io::Error::from(err)))?;
        text.push('\n');
        if let Some(folder) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(folder).map_err(|err| Error::io(folder, err))?;
        }
        safe_write(&self.path, text.as_bytes()).map_err(|err| Error::io(&self.path, err))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // The state is replaced only after a save succeeded, in two plain
        // assignments, so a panic in another holder leaves it whole.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The top-level object of the file at `path`; empty when there is no file.
/// The error is what is wrong with the file, completing "the file ...".
fn read_document(path: &Path) -> Result<Map<String, Value>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(err) => return Err(format!("cannot be read ({err})")),
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(document)) => Ok(document),
        Ok(other) => Err(format!(
            "holds {} where a JSON object is expected",
            kind_of(&other)
        )),
        Err(err) => Err(format!("is not valid JSON ({err})")),
    }
}

/// The settings a document stands for. A known key whose value cannot be
/// used gets its default and a warning.
fn read_settings(
    document: &Map<String, Value>,
    default_meetings_root: &Path,
    warnings: &mut Vec<SettingsWarning>,
) -> Settings {
    let meetings_root = match text_of(document, KEY_MEETINGS_ROOT, false, warnings) {
        None => default_meetings_root.to_owned(),
        Some(text) => {
            let root = PathBuf::from(text);
            match check_root(&root) {
                Ok(_) => root,
                Err(reason) => {
                    warnings.push(SettingsWarning::InvalidValue {
                        key: KEY_MEETINGS_ROOT,
                        reason,
                    });
                    default_meetings_root.to_owned()
                }
            }
        }
    };
    Settings {
        default_model_id: text_of(document, KEY_DEFAULT_MODEL_ID, true, warnings),
        language: text_of(document, KEY_LANGUAGE, true, warnings)
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
        global_shortcut: text_of(document, KEY_GLOBAL_SHORTCUT, true, warnings),
        meetings_root,
    }
}

/// The text under `key`, or `None` when the setting is not set: the key is
/// missing, `null`, or empty text (after trimming, with `trim`). A value
/// that is not text is not set either, and is a warning.
fn text_of(
    document: &Map<String, Value>,
    key: &'static str,
    trim: bool,
    warnings: &mut Vec<SettingsWarning>,
) -> Option<String> {
    match document.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let text = if trim { text.trim() } else { text.as_str() };
            (!text.is_empty()).then(|| text.to_owned())
        }
        Some(other) => {
            warnings.push(SettingsWarning::InvalidValue {
                key,
                reason: format!("is {}, not text", kind_of(other)),
            });
            None
        }
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Changes the keys of `document` that `patch` names.
fn apply(document: &mut Map<String, Value>, patch: &SettingsPatch) -> Result<()> {
    // Validate before the first change, though the caller works on a copy.
    let meetings_root = match &patch.meetings_root {
        Patch::Keep => Patch::Keep,
        Patch::Clear => Patch::Clear,
        Patch::Set(root) if root.as_os_str().is_empty() => Patch::Clear,
        Patch::Set(root) => match check_root(root) {
            Ok(text) => Patch::Set(text),
            Err(reason) => {
                return Err(Error::InvalidSetting {
                    key: KEY_MEETINGS_ROOT,
                    reason,
                });
            }
        },
    };

    let trimmed = |patch: &Patch<String>| match patch {
        Patch::Keep => Patch::Keep,
        Patch::Clear => Patch::Clear,
        Patch::Set(text) if text.trim().is_empty() => Patch::Clear,
        Patch::Set(text) => Patch::Set(text.trim().to_owned()),
    };
    let mut put = |key: &str, change: Patch<String>| match change {
        Patch::Keep => {}
        Patch::Clear => {
            document.remove(key);
        }
        Patch::Set(text) => {
            document.insert(key.to_owned(), Value::String(text));
        }
    };
    put(KEY_DEFAULT_MODEL_ID, trimmed(&patch.default_model_id));
    put(KEY_LANGUAGE, trimmed(&patch.language));
    put(KEY_GLOBAL_SHORTCUT, trimmed(&patch.global_shortcut));
    put(KEY_MEETINGS_ROOT, meetings_root);
    Ok(())
}

/// Checks that `root` can be a meetings root and returns it as the text the
/// file stores. The error completes "the meetings root ...".
///
/// A relative path is refused rather than resolved: it would land wherever
/// the process happens to be running. That includes a path starting with
/// `~`, which only a shell expands.
fn check_root(root: &Path) -> Result<String, String> {
    let Some(text) = root.to_str() else {
        return Err(format!(
            "is not valid UTF-8: \"{}\"",
            root.to_string_lossy()
        ));
    };
    if !root.is_absolute() {
        return Err(format!(
            "is not an absolute path: \"{text}\" (`~` is not expanded)"
        ));
    }
    Ok(text.to_owned())
}

/// Moves whatever is at `path` to the first free name among
/// `<name>.bad`, `<name>.bad.2`, ... and returns that name.
///
/// A name that is taken is never reused, so no earlier bad file is lost.
/// Looking for a free name and renaming are two steps; that is safe with
/// one running copy of the app, which the meeting store assumes as well.
fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let name = path.file_name().unwrap_or(path.as_os_str());
    for n in 1..=MAX_BAD_FILES {
        let mut bad_name = OsString::from(name);
        bad_name.push(".bad");
        if n > 1 {
            bad_name.push(format!(".{n}"));
        }
        let kept_as = path.with_file_name(bad_name);
        match fs::symlink_metadata(&kept_as) {
            Ok(_) => continue,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        fs::rename(path, &kept_as)?;
        if let Some(folder) = kept_as.parent() {
            sync_dir(folder);
        }
        return Ok(kept_as);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("every name up to .bad.{MAX_BAD_FILES} is taken"),
    ))
}

/// Deletes the temp files that interrupted saves of the file at `path` left
/// in its folder. Best effort: one that cannot be deleted is left alone, as
/// it is hidden and never read.
fn remove_temp_files(path: &Path, warnings: &mut Vec<SettingsWarning>) {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let name = name.to_string_lossy();
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut removed: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| is_temp_file_of(&entry.file_name().to_string_lossy(), &name))
        .map(|entry| entry.path())
        .filter(|temp| fs::remove_file(temp).is_ok())
        .collect();
    removed.sort();
    warnings.extend(
        removed
            .into_iter()
            .map(|path| SettingsWarning::RemovedTempFile { path }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // What the fields of a patch turn into is tested through the public API
    // in `tests/settings.rs`. Here: the one thing that cannot be reached
    // from there without filling a folder with 999 files.
    #[test]
    fn set_aside_takes_the_first_free_name_and_never_a_taken_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let names = |dir: &Path| {
            let mut names: Vec<String> = fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };

        for (round, expected) in [
            "settings.json.bad",
            "settings.json.bad.2",
            "settings.json.bad.3",
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&path, format!("round {round}")).unwrap();
            let kept_as = set_aside(&path).unwrap();
            assert_eq!(kept_as, dir.path().join(expected));
            assert_eq!(
                fs::read_to_string(&kept_as).unwrap(),
                format!("round {round}")
            );
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.bad")).unwrap(),
            "round 0"
        );

        // A gap is filled; a link or a folder with a taken name counts as
        // taken.
        fs::remove_file(dir.path().join("settings.json.bad.2")).unwrap();
        fs::create_dir(dir.path().join("settings.json.bad.2")).unwrap();
        fs::write(&path, "round 3").unwrap();
        assert_eq!(
            set_aside(&path).unwrap(),
            dir.path().join("settings.json.bad.4")
        );
        assert_eq!(
            names(dir.path()),
            [
                "settings.json.bad",
                "settings.json.bad.2",
                "settings.json.bad.3",
                "settings.json.bad.4"
            ]
        );

        // Nothing to move is an error, not a new empty file.
        let err = set_aside(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
