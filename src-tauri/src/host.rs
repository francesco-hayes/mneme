//! The real [`Host`]: Tauri events, Core Audio sources, Whisper, Finder.
//!
//! Nothing here runs in tests: it needs a window, devices, a model file and
//! the user's permissions.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use mneme_audio::{
    MicChoice, MicSource, PermissionStatus, SystemAudioSource, microphone_permission_status,
    request_microphone_permission, request_system_audio_permission, system_audio_permission_status,
};
use mneme_pipeline::{AudioSource, Transcriber};
use mneme_transcribe::WhisperEngine;
use tauri::{AppHandle, Emitter};

use crate::app::{Host, PermissionKind, Result};
use crate::dto;
use crate::tray;

/// Event carrying a `RecordingState`: on every change, and about once a
/// second while a meeting records.
pub const RECORDING_STATE_EVENT: &str = "recording-state";
/// Event carrying a `DownloadProgress`.
pub const DOWNLOAD_PROGRESS_EVENT: &str = "download-progress";

pub struct TauriHost {
    app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// Runs `/usr/bin/open` with these arguments. `open` hands the request to
/// Finder or System Settings and returns; it opens no network connection.
fn open(arguments: &[&std::ffi::OsStr]) -> Result<()> {
    let status = Command::new("/usr/bin/open")
        .args(arguments)
        .status()
        .map_err(|e| format!("Could not run open: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open failed ({status})"))
    }
}

impl Host for TauriHost {
    fn recording_state(&self, state: &dto::RecordingState) {
        if let Err(error) = self.app.emit(RECORDING_STATE_EVENT, state.clone()) {
            eprintln!("mneme: sending the recording state: {error}");
        }
        // The menu bar icon follows the same state as the window.
        tray::recording_state_changed(&self.app, state);
    }

    fn starting(&self) {
        tray::redraw(&self.app);
    }

    fn download_progress(&self, progress: &dto::DownloadProgress) {
        if let Err(error) = self.app.emit(DOWNLOAD_PROGRESS_EVENT, progress.clone()) {
            eprintln!("mneme: sending download progress: {error}");
        }
        // A download that ended may have installed the first model.
        if progress.state != "downloading" {
            tray::refresh(&self.app);
        }
    }

    fn load_transcriber(&self, model_path: &Path) -> Result<Arc<dyn Transcriber>> {
        WhisperEngine::silence_log();
        let engine = WhisperEngine::load(model_path)
            .map_err(|e| format!("The transcription model could not be loaded: {e}"))?;
        Ok(Arc::new(engine))
    }

    fn sources(&self, microphone: &MicChoice) -> Vec<Box<dyn AudioSource>> {
        vec![
            Box::new(MicSource::with_choice(microphone.clone())),
            Box::new(SystemAudioSource::new()),
        ]
    }

    fn microphones(&self) -> Vec<dto::Microphone> {
        mneme_audio::input_devices()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    fn microphone_permission(&self) -> PermissionStatus {
        microphone_permission_status()
    }

    fn request_microphone_permission(&self) -> PermissionStatus {
        request_microphone_permission()
    }

    fn system_audio_permission(&self) -> Option<PermissionStatus> {
        system_audio_permission_status()
    }

    fn request_system_audio_permission(&self) -> Option<PermissionStatus> {
        request_system_audio_permission()
    }

    fn reveal(&self, path: &Path) -> Result<()> {
        open(&["-R".as_ref(), path.as_os_str()])
    }

    fn trash(&self, path: &Path) -> Result<()> {
        move_to_trash(path)
    }

    fn open_privacy_settings(&self, kind: PermissionKind) -> Result<()> {
        open(&[kind.settings_url().as_ref()])
    }
}

/// Moves `path` to the user's Trash with `NSFileManager`, as Finder's Move
/// to Trash does: Put Back works on it afterwards. Local only; nothing is
/// sent anywhere.
#[cfg(target_os = "macos")]
fn move_to_trash(path: &Path) -> Result<()> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    let path_str = path
        .to_str()
        .ok_or_else(|| format!("The folder name of {} is not valid text.", path.display()))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path_str));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| {
            format!(
                "Could not move the meeting to the Trash: {}",
                error.localizedDescription()
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn move_to_trash(_path: &Path) -> Result<()> {
    Err("Moving to the Trash is only supported on macOS.".to_owned())
}
