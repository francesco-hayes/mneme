//! One Tauri command per method of the frontend contract
//! (`src/lib/api/types.ts`). Each runs its [`AppCore`] call on the blocking
//! thread pool, so neither the window nor the async runtime waits on file
//! access, a model load, a stop or a permission prompt.

use std::fs::File;
use std::sync::Arc;

use mneme_store::{Settings, SettingsPatch};
use tauri::{AppHandle, State};

use crate::app::{AppCore, PermissionKind};
use crate::dto;

type CommandResult<T> = Result<T, String>;

/// What Tauri manages for the commands.
pub struct AppState {
    /// The core, or why it could not be opened at launch. Every command then
    /// fails with that message instead of the app refusing to start.
    pub core: CommandResult<Arc<AppCore>>,
    /// Held for the life of the process: the single-instance lock.
    pub _instance_lock: Option<File>,
}

async fn run<T, F>(state: State<'_, AppState>, call: F) -> CommandResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Arc<AppCore>) -> CommandResult<T> + Send + 'static,
{
    let core = state.core.clone()?;
    tauri::async_runtime::spawn_blocking(move || call(&core))
        .await
        .map_err(|e| format!("The request did not finish: {e}"))?
}

#[tauri::command]
pub async fn list_meetings(state: State<'_, AppState>) -> CommandResult<Vec<dto::MeetingSummary>> {
    run(state, |core| core.list_meetings()).await
}

#[tauri::command]
pub async fn get_meeting(state: State<'_, AppState>, id: String) -> CommandResult<dto::Meeting> {
    run(state, move |core| core.get_meeting(&id)).await
}

#[tauri::command]
pub async fn get_transcript(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<dto::TranscriptLine>> {
    run(state, move |core| core.get_transcript(&id)).await
}

#[tauri::command]
pub async fn start_meeting(state: State<'_, AppState>) -> CommandResult<dto::Meeting> {
    run(state, |core| core.start_meeting()).await
}

#[tauri::command]
pub async fn stop_meeting(state: State<'_, AppState>) -> CommandResult<dto::Meeting> {
    run(state, |core| core.stop_meeting()).await
}

#[tauri::command]
pub async fn save_notes(
    state: State<'_, AppState>,
    id: String,
    notes: String,
) -> CommandResult<dto::MeetingSummary> {
    run(state, move |core| core.save_notes(&id, &notes)).await
}

#[tauri::command]
pub async fn rename_meeting(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> CommandResult<dto::MeetingSummary> {
    run(state, move |core| core.rename_meeting(&id, &title)).await
}

#[tauri::command]
pub async fn show_in_finder(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    run(state, move |core| core.show_in_finder(&id)).await
}

#[tauri::command]
pub async fn delete_meeting(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    run(state, move |core| core.delete_meeting(&id)).await
}

#[tauri::command]
pub async fn get_recording_state(state: State<'_, AppState>) -> CommandResult<dto::RecordingState> {
    run(state, |core| Ok(core.recording_state())).await
}

#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> CommandResult<Vec<dto::ModelInfo>> {
    run(state, |core| Ok(core.list_models())).await
}

#[tauri::command]
pub async fn download_model(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    run(state, move |core| core.download_model(&id)).await
}

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    run(state, move |core| {
        core.cancel_download(&id);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_model(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    run(state, move |core| core.delete_model(&id)).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    run(state, |core| Ok(core.get_settings())).await
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> CommandResult<Settings> {
    // Through the shortcut module: a new global shortcut is registered
    // with the system before it is saved.
    run(state, move |core| {
        crate::shortcut::update_settings(&app, core, &patch)
    })
    .await
}

#[tauri::command]
pub async fn list_microphones(state: State<'_, AppState>) -> CommandResult<Vec<dto::Microphone>> {
    run(state, |core| Ok(core.list_microphones())).await
}

#[tauri::command]
pub async fn get_permissions(state: State<'_, AppState>) -> CommandResult<dto::Permissions> {
    run(state, |core| Ok(core.permissions())).await
}

#[tauri::command]
pub async fn request_permission(
    state: State<'_, AppState>,
    kind: String,
) -> CommandResult<&'static str> {
    let kind = PermissionKind::parse(&kind)?;
    run(state, move |core| Ok(core.request_permission(kind))).await
}
