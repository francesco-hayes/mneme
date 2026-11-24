//! The Mneme desktop app: the window and the commands behind it (PRD 06).
//!
//! `app.rs` is the app's logic with no Tauri in it, `commands.rs` exposes it
//! to the window, `host.rs` is the real audio, model and Finder side,
//! `tray.rs` is the menu bar icon, `shortcut.rs` the global shortcut,
//! `permissions.rs` the decisions about the two capture permissions.

use std::fs::{File, TryLockError};
use std::io;
use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

pub mod app;
mod commands;
pub mod dto;
mod host;
pub mod permissions;
mod shortcut;
mod tray;

use commands::AppState;

/// File in the app data folder that the running copy keeps locked.
const INSTANCE_LOCK_FILE: &str = "mneme.lock";

/// Takes the single-instance lock in `dir`. `Ok(None)` means another running
/// copy holds it. The lock is released when the returned file is dropped or
/// the process ends, however it ends.
fn instance_lock(dir: &Path) -> io::Result<Option<File>> {
    std::fs::create_dir_all(dir)?;
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(INSTANCE_LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Opens the app's state at launch. Recovery of meetings a crashed run left
/// open assumes one running copy, so a second copy exits here.
fn launch(handle: &AppHandle) -> AppState {
    let dirs = handle
        .path()
        .app_data_dir()
        .and_then(|data| Ok((data, handle.path().document_dir()?)));
    let (data_dir, documents_dir) = match dirs {
        Ok(dirs) => dirs,
        Err(error) => {
            return AppState {
                core: Err(format!("Mneme could not find its folders: {error}")),
                _instance_lock: None,
            };
        }
    };
    let instance_lock = match instance_lock(&data_dir) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            eprintln!("mneme: another copy of Mneme is already running");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("mneme: single-instance lock: {error}");
            None
        }
    };
    let host = Arc::new(host::TauriHost::new(handle.clone()));
    AppState {
        core: app::AppCore::open(&data_dir, &documents_dir, host),
        _instance_lock: instance_lock,
    }
}

/// Builds and runs the Tauri app. Blocks until the app exits.
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let state = launch(app.handle());
            match &state.core {
                // Loads the model in the background, so New meeting is fast.
                // `shutdown` at exit frees it.
                Ok(core) => core.warm_up(),
                Err(error) => eprintln!("mneme: {error}"),
            }
            app.manage(state);
            // Without the menu bar icon the window still does everything.
            if let Err(error) = tray::setup(app.handle()) {
                eprintln!("mneme: menu bar icon: {error}");
            }
            // Nor is the global shortcut needed; it logs its own failures.
            shortcut::setup(app.handle());
            Ok(())
        })
        // Closing the main window hides it: the app, and a recording, carry
        // on in the menu bar. Open Mneme or the Dock icon brings it back.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == tray::MAIN_WINDOW
            {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("mneme: hiding the main window: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_meetings,
            commands::get_meeting,
            commands::get_transcript,
            commands::start_meeting,
            commands::stop_meeting,
            commands::save_notes,
            commands::rename_meeting,
            commands::show_in_finder,
            commands::get_recording_state,
            commands::list_models,
            commands::download_model,
            commands::cancel_download,
            commands::delete_model,
            commands::get_settings,
            commands::update_settings,
            commands::get_permissions,
            commands::request_permission,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Mneme");

    app.run(|handle, event| match event {
        // No window left is not a reason to quit: the menu bar icon is still
        // there. An exit asked for in code (Quit Mneme) carries a code and
        // goes through.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        // Every way out, Quit Mneme and Cmd+Q included: stop the session,
        // wait for a model load under way and free the warm Whisper engine
        // before the process ends (an engine alive at exit aborts it).
        // `shutdown` is final, so it runs only here.
        RunEvent::Exit => {
            if let Some(state) = handle.try_state::<AppState>()
                && let Ok(core) = &state.core
            {
                core.shutdown();
            }
        }
        // A click on the Dock icon while the window is hidden.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => tray::show_main_window(handle),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    fn config() -> Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json is JSON")
    }

    #[test]
    fn identity_matches_the_prds() {
        let config = config();
        assert_eq!(config["productName"], "Mneme");
        assert_eq!(config["identifier"], "app.mneme.desktop");
        assert_eq!(config["bundle"]["macOS"]["minimumSystemVersion"], "14.4");
        assert_eq!(config["app"]["windows"][0]["title"], "Mneme");
    }

    /// Local-only rule (PRD 00): the webview may load and connect to nothing
    /// outside the app itself.
    #[test]
    fn csp_allows_no_remote_source() {
        let config = config();
        let csp = config["app"]["security"]["csp"]
            .as_object()
            .expect("csp is set, as a map of directives");
        assert_eq!(csp["default-src"], "'self'");
        assert_eq!(csp["script-src"], "'self'");

        // Everything a directive may name. `ipc:` and `http://ipc.localhost`
        // are how the webview reaches the Rust side of this app.
        let allowed = ["'self'", "'none'", "data:", "ipc:", "http://ipc.localhost"];
        for (directive, sources) in csp {
            let sources = sources.as_str().expect("directive value is a string");
            for source in sources.split_whitespace() {
                assert!(
                    allowed.contains(&source),
                    "{directive} allows {source}, which is not on the local-only list"
                );
            }
        }
        assert!(config["app"]["security"].get("devCsp").is_none());
    }

    /// Recovery assumes one running copy per meetings root.
    #[test]
    fn a_second_copy_does_not_get_the_instance_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = super::instance_lock(dir.path()).unwrap();
        assert!(first.is_some());
        assert!(super::instance_lock(dir.path()).unwrap().is_none());
        drop(first);
        assert!(super::instance_lock(dir.path()).unwrap().is_some());
    }

    /// Issue 024: the hardened runtime gets the audio input entitlement and
    /// nothing else, and the bundle config names the file.
    #[test]
    fn entitlements_are_referenced_and_hold_only_audio_input() {
        let config = config();
        assert_eq!(
            config["bundle"]["macOS"]["entitlements"],
            "Entitlements.plist"
        );
        let entitlements = include_str!("../Entitlements.plist");
        let keys: Vec<&str> = entitlements
            .split("<key>")
            .skip(1)
            .map(|rest| rest.split("</key>").next().unwrap())
            .collect();
        assert_eq!(keys, ["com.apple.security.device.audio-input"]);
        assert_eq!(entitlements.matches("<true/>").count(), 1);
    }

    /// Without a usage string macOS ends the app at the first request.
    #[test]
    fn info_plist_explains_both_kinds_of_capture() {
        let info = include_str!("../Info.plist");
        for key in [
            "NSMicrophoneUsageDescription",
            "NSAudioCaptureUsageDescription",
        ] {
            let after = info
                .split(&format!("<key>{key}</key>"))
                .nth(1)
                .unwrap_or_else(|| panic!("{key} is missing"));
            let text = after.split("</string>").next().unwrap();
            assert!(
                text.contains("transcribed on this Mac and then discarded"),
                "{key}: {text}"
            );
        }
    }

    #[test]
    fn no_updater_and_no_plugins() {
        let config = config();
        assert!(config.get("plugins").is_none());
        assert!(config["bundle"].get("createUpdaterArtifacts").is_none());
    }
}
