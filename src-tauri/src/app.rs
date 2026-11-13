//! The app's state and everything the commands do, as plain Rust.
//!
//! Nothing here knows about Tauri. What differs between the real app and a
//! test (audio sources, the transcriber, permissions, events to the window,
//! Finder) comes in through [`Host`]; folders come in as arguments. The Tauri
//! commands in `commands.rs` are one-line wrappers around [`AppCore`].

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use mneme_audio::PermissionStatus;
use mneme_core::Speaker;
use mneme_pipeline::{
    AudioError, AudioSource, Language, RecorderOptions, Session, SessionConfig, SessionSummary,
    SourcePhase, Transcriber,
};
use mneme_store::{
    MeetingId, NewMeeting, Patch, SETTINGS_FILE, Settings, SettingsPatch, SettingsStore, StartTime,
    Store,
};
use mneme_transcribe::ModelStore;

use crate::dto;

/// Errors cross to the UI as one readable sentence.
pub type Result<T> = std::result::Result<T, String>;

/// Folder inside the app data folder that holds the model files.
pub const MODELS_DIR: &str = "models";
/// Folder inside Documents that holds the meetings unless the settings say
/// otherwise.
pub const MEETINGS_DIR: &str = "Mneme";

/// How often the recording state is sent while a meeting records.
pub const TICK: Duration = Duration::from_secs(1);

/// How long quitting waits for the transcriber to catch up before the audio
/// still waiting is given up. Whatever was transcribed by then is kept.
const QUIT_GRACE: Duration = Duration::from_secs(10);

const NOT_RECORDING: &str = "No meeting is being recorded.";
const ALREADY_RECORDING: &str = "A meeting is already being recorded.";
const NO_MODEL: &str = "No transcription model is installed. Download one in Settings.";
const MIC_DENIED: &str = "Microphone access is denied. Allow Mneme in System Settings > Privacy & Security > Microphone.";

fn msg(error: impl Display) -> String {
    error.to_string()
}

fn log(message: impl Display) {
    eprintln!("mneme: {message}");
}

/// The two capture permissions, as `requestPermission` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    Microphone,
    SystemAudio,
}

impl PermissionKind {
    pub fn parse(kind: &str) -> Result<Self> {
        match kind {
            "microphone" => Ok(Self::Microphone),
            "systemAudio" => Ok(Self::SystemAudio),
            other => Err(format!("Unknown permission: {other}")),
        }
    }
}

/// Everything the core needs from outside: the real app implements it with
/// Tauri, Core Audio and Whisper (`host.rs`), tests with fakes.
pub trait Host: Send + Sync + 'static {
    /// Sends a recording state to the window.
    fn recording_state(&self, state: &dto::RecordingState);
    /// Sends a download progress event to the window.
    fn download_progress(&self, progress: &dto::DownloadProgress);
    /// Loads the model file. Slow (seconds); never called on the UI thread.
    fn load_transcriber(&self, model_path: &Path) -> Result<Arc<dyn Transcriber>>;
    /// The sources of one meeting, microphone first.
    fn sources(&self) -> Vec<Box<dyn AudioSource>>;
    fn recorder_options(&self) -> RecorderOptions {
        RecorderOptions::default()
    }
    fn microphone_permission(&self) -> PermissionStatus;
    /// May show the system prompt and block until it is answered.
    fn request_microphone_permission(&self) -> PermissionStatus;
    /// `None` means unknown, which is the normal answer on macOS.
    fn system_audio_permission(&self) -> Option<PermissionStatus>;
    /// May show the system prompt. `None` means the outcome is unknown.
    fn request_system_audio_permission(&self) -> Option<PermissionStatus>;
    /// Shows a file or folder in Finder.
    fn reveal(&self, path: &Path) -> Result<()>;
    /// Opens the pane of System Settings where the permission is changed.
    fn open_privacy_settings(&self, kind: PermissionKind) -> Result<()>;
}

/// A meeting that is being recorded or stopped.
#[derive(Clone)]
struct Live {
    meeting_id: MeetingId,
    model_id: String,
    session: Arc<Session>,
    /// The store the meeting was created in.
    store: Arc<Store>,
}

/// One recording at a time: the whole state of it.
enum Recording {
    Idle,
    /// `startMeeting` is running: the model is loading. Reported as idle,
    /// but a second start is refused.
    Starting {
        model_id: String,
    },
    Active {
        live: Live,
        /// The loaded model. It lives only as long as the recording: an
        /// engine still alive when the process exits aborts it (issue 005).
        engine: Arc<dyn Transcriber>,
    },
    /// `stop` is running: the transcriber is catching up.
    Stopping(Live),
}

impl Recording {
    fn live(&self) -> Option<&Live> {
        match self {
            Recording::Active { live, .. } | Recording::Stopping(live) => Some(live),
            Recording::Idle | Recording::Starting { .. } => None,
        }
    }

    fn model_id(&self) -> Option<&str> {
        match self {
            Recording::Starting { model_id } => Some(model_id),
            other => other.live().map(|live| live.model_id.as_str()),
        }
    }

    fn state(&self) -> dto::RecordingState {
        let status = match self {
            Recording::Idle | Recording::Starting { .. } => return dto::RecordingState::idle(),
            Recording::Active { .. } => "recording",
            Recording::Stopping(_) => "stopping",
        };
        let live = self.live().expect("active and stopping have a meeting");
        dto::RecordingState {
            status,
            meeting_id: Some(live.meeting_id.as_str().to_owned()),
            elapsed_secs: live.session.elapsed().as_secs(),
        }
    }
}

/// The app's state: the three stores and the current recording.
pub struct AppCore {
    host: Arc<dyn Host>,
    settings: SettingsStore,
    models: ModelStore,
    /// One store per meetings root, kept for the life of the process and
    /// replaced only when the root changes.
    store: Mutex<Arc<Store>>,
    recording: Mutex<Recording>,
    /// Signalled whenever `recording` changes.
    changed: Condvar,
    quitting: AtomicBool,
    /// Set once this run has seen system audio capture work, the only way
    /// to know the permission without private API.
    system_audio_seen: AtomicBool,
    tick: Duration,
}

impl AppCore {
    /// Opens the app's state at launch: settings and models in
    /// `app_data_dir`, meetings in the root the settings name (by default
    /// `documents_dir/Mneme`). Runs store recovery, so the first list
    /// already shows a meeting the last run left unfinished as interrupted.
    pub fn open(
        app_data_dir: &Path,
        documents_dir: &Path,
        host: Arc<dyn Host>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(app_data_dir)
            .map_err(|e| format!("Could not create {}: {e}", app_data_dir.display()))?;
        let models = ModelStore::open(app_data_dir.join(MODELS_DIR)).map_err(msg)?;
        Self::open_with(app_data_dir, documents_dir, models, host, TICK)
    }

    /// [`open`](Self::open) with a given model store and state interval.
    pub fn open_with(
        app_data_dir: &Path,
        documents_dir: &Path,
        models: ModelStore,
        host: Arc<dyn Host>,
        tick: Duration,
    ) -> Result<Arc<Self>> {
        let (settings, warnings) = SettingsStore::open(
            app_data_dir.join(SETTINGS_FILE),
            documents_dir.join(MEETINGS_DIR),
        )
        .map_err(msg)?;
        for warning in warnings {
            log(format_args!("settings: {warning}"));
        }
        let store = open_store(&settings.get().meetings_root)?;
        Ok(Arc::new(Self {
            host,
            settings,
            models,
            store: Mutex::new(store),
            recording: Mutex::new(Recording::Idle),
            changed: Condvar::new(),
            quitting: AtomicBool::new(false),
            system_audio_seen: AtomicBool::new(false),
            tick,
        }))
    }

    fn store(&self) -> Arc<Store> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn recording(&self) -> MutexGuard<'_, Recording> {
        self.recording
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn set_recording(&self, next: Recording) {
        *self.recording() = next;
        self.changed.notify_all();
        self.emit_state();
    }

    fn emit_state(&self) {
        let state = self.recording_state();
        self.host.recording_state(&state);
    }

    /// The live meeting with this id, if this run is recording or stopping it.
    fn live(&self, id: &MeetingId) -> Option<Live> {
        self.recording()
            .live()
            .filter(|live| &live.meeting_id == id)
            .cloned()
    }

    fn meeting_dto(&self, store: &Store, id: &MeetingId) -> Result<dto::Meeting> {
        let meeting = store.load_meeting(id).map_err(msg)?;
        Ok(dto::Meeting::from_store(&meeting, self.live(id).is_some()))
    }

    // ---- Meetings ----

    pub fn list_meetings(&self) -> Result<Vec<dto::MeetingSummary>> {
        let list = self.store().list_meetings().map_err(msg)?;
        for warning in &list.warnings {
            log(format_args!("meetings: {warning}"));
        }
        let live_id = self.recording().live().map(|live| live.meeting_id.clone());
        Ok(list
            .meetings
            .iter()
            .map(|m| dto::MeetingSummary::from_store(m, live_id.as_ref() == Some(&m.id)))
            .collect())
    }

    pub fn get_meeting(&self, id: &str) -> Result<dto::Meeting> {
        self.meeting_dto(&self.store(), &MeetingId::from(id))
    }

    pub fn get_transcript(&self, id: &str) -> Result<Vec<dto::TranscriptLine>> {
        let transcript = self.store().transcript(&MeetingId::from(id)).map_err(msg)?;
        Ok(transcript.lines.iter().map(Into::into).collect())
    }

    /// Saves the notes. While this meeting records (or its stop is still
    /// running) the save carries the meeting time, on the clock of the
    /// transcript's timestamps, so the note timeline is written. At any
    /// other time, and for any other meeting, it is a plain save.
    pub fn save_notes(&self, id: &str, notes: &str) -> Result<()> {
        let id = MeetingId::from(id);
        match self.live(&id) {
            Some(live) => {
                let at_ms = u64::try_from(live.session.elapsed().as_millis()).unwrap_or(u64::MAX);
                let saved = live.store.save_notes_at(&id, notes, at_ms).map_err(msg)?;
                if let Some(error) = saved.timeline_error {
                    log(format_args!("note timeline of {id}: {error}"));
                }
                Ok(())
            }
            None => self.store().save_notes(&id, notes).map_err(msg),
        }
    }

    pub fn rename_meeting(&self, id: &str, title: &str) -> Result<dto::MeetingSummary> {
        let title = title.trim();
        if title.is_empty() {
            return Err("The title cannot be empty.".to_owned());
        }
        let id = MeetingId::from(id);
        let summary = self.store().rename_meeting(&id, title).map_err(msg)?;
        Ok(dto::MeetingSummary::from_store(
            &summary,
            self.live(&id).is_some(),
        ))
    }

    pub fn show_in_finder(&self, id: &str) -> Result<()> {
        let meeting = self.store().meeting(&MeetingId::from(id)).map_err(msg)?;
        self.host.reveal(&meeting.folder)
    }

    // ---- Recording ----

    pub fn recording_state(&self) -> dto::RecordingState {
        self.recording().state()
    }

    /// Starts recording a new meeting and returns it. Blocks while the model
    /// loads (seconds), so call it off the UI thread.
    ///
    /// Refused while another meeting is recording, starting or stopping,
    /// when no model is installed, and when the microphone is denied.
    /// System audio that cannot be captured does not stop the meeting: the
    /// session records the microphone alone.
    pub fn start_meeting(self: &Arc<Self>) -> Result<dto::Meeting> {
        let (model_id, model_path) = {
            let mut recording = self.recording();
            if self.quitting.load(Ordering::SeqCst) {
                return Err("Mneme is quitting.".to_owned());
            }
            if !matches!(*recording, Recording::Idle) {
                return Err(ALREADY_RECORDING.to_owned());
            }
            let model = self.pick_model()?;
            *recording = Recording::Starting {
                model_id: model.0.clone(),
            };
            model
        };

        match self.start_session(&model_id, &model_path) {
            Ok((live, engine)) => {
                let (id, store) = (live.meeting_id.clone(), live.store.clone());
                self.set_recording(Recording::Active { live, engine });
                let core = Arc::clone(self);
                let watched = id.clone();
                if let Err(error) = thread::Builder::new()
                    .name("mneme-recording-state".to_owned())
                    .spawn(move || core.watch(&watched))
                {
                    log(format_args!("recording state thread: {error}"));
                }
                self.meeting_dto(&store, &id)
            }
            Err(error) => {
                self.set_recording(Recording::Idle);
                Err(error)
            }
        }
    }

    /// The default model if it is installed, otherwise the first installed
    /// one in catalog order.
    fn pick_model(&self) -> Result<(String, PathBuf)> {
        let default = self.settings.get().default_model_id;
        let id = default
            .filter(|id| self.models.is_installed(id))
            .or_else(|| {
                let installed = self.models.list().into_iter().find(|m| m.installed);
                installed.map(|m| m.id)
            })
            .ok_or_else(|| NO_MODEL.to_owned())?;
        let path = self.models.installed_path(&id).map_err(msg)?;
        Ok((id, path))
    }

    fn start_session(
        &self,
        model_id: &str,
        model_path: &Path,
    ) -> Result<(Live, Arc<dyn Transcriber>)> {
        let microphone = match self.host.microphone_permission() {
            PermissionStatus::NotDetermined => self.host.request_microphone_permission(),
            status => status,
        };
        if !microphone.is_granted() {
            return Err(MIC_DENIED.to_owned());
        }

        // The model first: if it cannot be loaded, no meeting folder is left
        // behind.
        let engine = self.host.load_transcriber(model_path)?;

        let store = self.store();
        let meeting = store
            .create_meeting(NewMeeting {
                started_at: StartTime::now_local(),
                model: Some(model_id.to_owned()),
                // v1 is English only, whatever the settings file says.
                language: Some(Language::english().as_str().to_owned()),
            })
            .map_err(msg)?;

        let started = store
            .transcript_writer(&meeting.id)
            .map_err(msg)
            .and_then(|writer| {
                let mut config = SessionConfig::new(Language::english());
                config.audio_dir = Some(meeting.folder.clone());
                config.recorder = self.host.recorder_options();
                Session::start(self.host.sources(), Arc::clone(&engine), writer, config)
                    .map_err(msg)
            });
        match started {
            Ok(session) => Ok((
                Live {
                    meeting_id: meeting.id,
                    model_id: model_id.to_owned(),
                    session: Arc::new(session),
                    store,
                },
                engine,
            )),
            Err(error) => {
                // Not left as `recording`: nothing is recording it.
                if let Err(close) = store.finish_meeting(&meeting.id, 0) {
                    log(format_args!(
                        "closing {} after a failed start: {close}",
                        meeting.id
                    ));
                }
                Err(format!("The recording could not start: {error}"))
            }
        }
    }

    /// Runs on its own thread for one meeting: sends the state every tick,
    /// and finishes the meeting if the session ends by itself (every source
    /// ended, or the session failed).
    fn watch(&self, meeting_id: &MeetingId) {
        let mut reported_failures = false;
        let mut next_tick = Instant::now() + self.tick;
        loop {
            let session = {
                let mut recording = self.recording();
                // Wait out the tick. Any change of state ends the watch; a
                // wake-up for no reason goes back to waiting.
                loop {
                    match &*recording {
                        Recording::Active { live, .. } if &live.meeting_id == meeting_id => {}
                        _ => return,
                    }
                    let now = Instant::now();
                    if now >= next_tick {
                        break;
                    }
                    recording = self
                        .changed
                        .wait_timeout(recording, next_tick - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                next_tick = Instant::now() + self.tick;
                match &*recording {
                    Recording::Active { live, .. } => Arc::clone(&live.session),
                    _ => return,
                }
            };
            if session.is_done() {
                if let Err(error) = self.stop(Some(meeting_id), false) {
                    log(format_args!("finishing {meeting_id}: {error}"));
                }
                return;
            }
            let status = session.status();
            for source in &status.sources {
                if source.speaker == Speaker::Them && source.phase == SourcePhase::Running {
                    self.system_audio_seen.store(true, Ordering::SeqCst);
                }
                if source.phase == SourcePhase::FailedToStart && !reported_failures {
                    let reason = source.errors.first().map(ToString::to_string);
                    log(format_args!(
                        "{} audio is not being recorded: {}",
                        source.speaker,
                        reason.as_deref().unwrap_or("unknown error")
                    ));
                }
            }
            reported_failures |= status
                .sources
                .iter()
                .any(|s| s.phase == SourcePhase::FailedToStart);
            self.emit_state();
        }
    }

    /// Stops the recording and returns the finished meeting. Blocks until
    /// the transcript is complete, so call it off the UI thread. The idle
    /// state is sent only after the store has closed the meeting.
    pub fn stop_meeting(&self) -> Result<dto::Meeting> {
        self.stop(None, false)
    }

    /// `only`: stop only if this meeting is the one recording. `hurry`: give
    /// the transcriber [`QUIT_GRACE`] and then give up the audio still
    /// waiting.
    fn stop(&self, only: Option<&MeetingId>, hurry: bool) -> Result<dto::Meeting> {
        let (live, engine) = {
            let mut recording = self.recording();
            match &*recording {
                Recording::Active { live, .. } if only.is_none_or(|id| id == &live.meeting_id) => {}
                Recording::Stopping(_) => return Err("The meeting is already stopping.".to_owned()),
                _ => return Err(NOT_RECORDING.to_owned()),
            }
            let Recording::Active { live, engine } =
                std::mem::replace(&mut *recording, Recording::Idle)
            else {
                unreachable!("checked above under the same lock");
            };
            *recording = Recording::Stopping(live.clone());
            (live, engine)
        };
        self.changed.notify_all();
        self.emit_state();

        let summary = if hurry {
            live.session.request_stop();
            live.session.wait_timeout(QUIT_GRACE).unwrap_or_else(|| {
                live.session.abandon_backlog();
                live.session.stop()
            })
        } else {
            live.session.stop()
        };
        // The session's reference to the engine is gone once `stop` has
        // returned, so this frees the model.
        drop(engine);
        let result = self.close_meeting(&live, &summary);
        // Idle only now: a `getMeeting` right after the idle state must see
        // the final status, duration and line count.
        self.set_recording(Recording::Idle);
        result
    }

    fn close_meeting(&self, live: &Live, summary: &SessionSummary) -> Result<dto::Meeting> {
        let id = &live.meeting_id;
        report(id, summary);
        if !summary.finalized
            && let Err(error) = live.store.finalize_transcript(id)
        {
            log(format_args!("sorting the transcript of {id}: {error}"));
        }
        let finished = live
            .store
            .finish_meeting(id, summary.duration.as_secs())
            .map_err(msg)?;
        if let Some(error) = finished.rename_error {
            log(format_args!("renaming the folder of {id}: {error}"));
        }
        let meeting = live.store.load_meeting(id).map_err(msg)?;
        Ok(dto::Meeting::from_store(&meeting, false))
    }

    /// Called when the app is about to exit: refuses new recordings, waits
    /// for a start or stop that is under way, and stops a running recording.
    /// When it returns no session and no engine is alive. Safe to call more
    /// than once.
    pub fn shutdown(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        loop {
            let recording = self.recording();
            match &*recording {
                Recording::Idle => return,
                Recording::Active { .. } => {
                    drop(recording);
                    // An error here means another thread got to it first;
                    // the loop then waits for that stop to finish.
                    if let Err(error) = self.stop(None, true)
                        && error != NOT_RECORDING
                    {
                        log(format_args!("stopping at quit: {error}"));
                    }
                }
                Recording::Starting { .. } | Recording::Stopping(_) => {
                    drop(
                        self.changed
                            .wait_timeout(recording, Duration::from_millis(200))
                            .unwrap_or_else(PoisonError::into_inner),
                    );
                }
            }
        }
    }

    // ---- Models ----

    pub fn list_models(&self) -> Vec<dto::ModelInfo> {
        self.models.list().into_iter().map(Into::into).collect()
    }

    /// Starts a download and returns at once; progress and the outcome
    /// arrive as download progress events.
    pub fn download_model(&self, id: &str) -> Result<()> {
        let host = Arc::clone(&self.host);
        self.models
            .start_download(id, move |progress| host.download_progress(&progress.into()))
            .map(drop)
            .map_err(msg)
    }

    pub fn cancel_download(&self, id: &str) {
        self.models.cancel_download(id);
    }

    /// Deletes a model file. Refused for the model the current recording
    /// uses. Deleting the default model clears the default.
    pub fn delete_model(&self, id: &str) -> Result<()> {
        let name = self.models.spec(id).map_err(msg)?.name.clone();
        {
            // Held across the delete so a start cannot pick the model in
            // between.
            let recording = self.recording();
            if recording.model_id() == Some(id) {
                return Err(format!(
                    "{name} is in use by the current recording and cannot be deleted."
                ));
            }
            self.models.delete(id).map_err(msg)?;
        }
        if self.settings.get().default_model_id.as_deref() == Some(id) {
            let patch = SettingsPatch {
                default_model_id: Patch::Clear,
                ..SettingsPatch::default()
            };
            self.settings.update(&patch).map_err(msg)?;
        }
        Ok(())
    }

    // ---- Settings ----

    pub fn get_settings(&self) -> Settings {
        self.settings.get()
    }

    /// Applies a patch and returns the settings, which are on disk by then.
    /// Refuses a default model that is not installed. A new meetings root
    /// (not in the contract, but in the settings file) is refused during a
    /// recording and otherwise opens, and recovers, a store on it.
    pub fn update_settings(&self, patch: &SettingsPatch) -> Result<Settings> {
        if let Patch::Set(id) = &patch.default_model_id
            && !self.models.is_installed(id.trim())
        {
            return Err(format!(
                "Model {} is not installed, so it cannot be the default.",
                id.trim()
            ));
        }
        if matches!(patch.meetings_root, Patch::Keep) {
            return self.settings.update(patch).map_err(msg);
        }

        // Held to the end so no meeting starts in the old root meanwhile.
        let recording = self.recording();
        if !matches!(*recording, Recording::Idle) {
            return Err(
                "The meetings folder cannot be changed while a meeting is being recorded."
                    .to_owned(),
            );
        }
        let before = self.settings.get().meetings_root;
        let after = self.settings.update(patch).map_err(msg)?;
        if after.meetings_root != before {
            match open_store(&after.meetings_root) {
                Ok(store) => *self.store.lock().unwrap_or_else(PoisonError::into_inner) = store,
                Err(error) => {
                    let revert = SettingsPatch {
                        meetings_root: Patch::Set(before),
                        ..SettingsPatch::default()
                    };
                    if let Err(revert_error) = self.settings.update(&revert) {
                        log(format_args!(
                            "restoring the meetings folder: {revert_error}"
                        ));
                    }
                    return Err(error);
                }
            }
        }
        Ok(after)
    }

    // ---- Permissions ----

    pub fn permissions(&self) -> dto::Permissions {
        dto::Permissions {
            microphone: permission(self.host.microphone_permission()),
            system_audio: self.system_audio_permission(),
        }
    }

    /// macOS has no public way to read this permission, so it is
    /// `undetermined` unless this run saw capture work or the host knows
    /// better. It is never reported as denied on a guess: a prompt may
    /// still be open.
    fn system_audio_permission(&self) -> &'static str {
        if self.system_audio_seen.load(Ordering::SeqCst) {
            return dto::GRANTED;
        }
        self.host
            .system_audio_permission()
            .map_or(dto::UNDETERMINED, permission)
    }

    /// Asks for a permission and returns what [`permissions`](Self::permissions)
    /// reports afterwards. May block on a system prompt: call it off the UI
    /// thread. For a permission that is already denied, macOS shows no
    /// prompt again, so this opens its pane in System Settings instead and
    /// returns `denied`.
    pub fn request_permission(&self, kind: PermissionKind) -> &'static str {
        let before = match kind {
            PermissionKind::Microphone => permission(self.host.microphone_permission()),
            PermissionKind::SystemAudio => self.system_audio_permission(),
        };
        match before {
            dto::DENIED => {
                if let Err(error) = self.host.open_privacy_settings(kind) {
                    log(format_args!("opening System Settings: {error}"));
                }
            }
            dto::UNDETERMINED => match kind {
                PermissionKind::Microphone => {
                    self.host.request_microphone_permission();
                }
                PermissionKind::SystemAudio => {
                    // Only a positive answer is trusted: a refusal read here
                    // can be a prompt that is still open.
                    if self.host.request_system_audio_permission()
                        == Some(PermissionStatus::Granted)
                    {
                        self.system_audio_seen.store(true, Ordering::SeqCst);
                    }
                }
            },
            _ => {}
        }
        match kind {
            PermissionKind::Microphone => permission(self.host.microphone_permission()),
            PermissionKind::SystemAudio => self.system_audio_permission(),
        }
    }
}

fn permission(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Granted => dto::GRANTED,
        PermissionStatus::Denied | PermissionStatus::Restricted => dto::DENIED,
        PermissionStatus::NotDetermined => dto::UNDETERMINED,
    }
}

/// Opens the store on a meetings root and closes whatever a crashed run
/// left as `recording` there, before anything is listed.
fn open_store(root: &Path) -> Result<Arc<Store>> {
    let store = Store::open(root).map_err(msg)?;
    match store.recover() {
        Ok(report) => {
            for recovered in &report.recovered {
                log(format_args!(
                    "recovered interrupted meeting {} ({} transcript lines)",
                    recovered.meeting.id, recovered.transcript.lines
                ));
            }
            for failure in &report.failed {
                log(format_args!(
                    "could not recover meeting {}: {}",
                    failure.meeting.id, failure.error
                ));
            }
            for warning in &report.warnings {
                log(format_args!("recovery: {warning}"));
            }
        }
        Err(error) => log(format_args!("recovery of {}: {error}", root.display())),
    }
    Ok(Arc::new(store))
}

/// Logs what went wrong in a session. The contract has no place for it.
fn report(id: &MeetingId, summary: &SessionSummary) {
    for source in summary.failed_sources() {
        let errors: Vec<String> = source.errors.iter().map(AudioError::to_string).collect();
        log(format_args!(
            "{id}: {} audio failed ({:?}): {}",
            source.speaker,
            source.phase,
            errors.join("; ")
        ));
    }
    for failure in &summary.transcription_errors {
        log(format_args!(
            "{id}: not transcribed, {} at {} ms: {}",
            failure.speaker, failure.start_ms, failure.error
        ));
    }
    for failure in &summary.sink_errors {
        log(format_args!("{id}: line not written: {}", failure.error));
    }
    if !summary.dropped.is_empty() {
        log(format_args!(
            "{id}: {} chunks of audio were dropped",
            summary.dropped.len()
        ));
    }
    if let Some(error) = &summary.finalize_error {
        log(format_args!("{id}: transcript not finalized: {error}"));
    }
}

#[cfg(test)]
mod tests;
