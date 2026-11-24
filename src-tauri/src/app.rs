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

use mneme_audio::{PermissionStatus, SourceStatus};
use mneme_core::Speaker;
use mneme_pipeline::{
    AudioError, AudioSource, EndReason, Language, RecorderOptions, Session, SessionConfig,
    SessionSummary, Transcriber,
};
use mneme_store::{
    MeetingId, NewMeeting, Patch, SETTINGS_FILE, Settings, SettingsPatch, SettingsStore, StartTime,
    Store,
};
use mneme_transcribe::ModelStore;

use crate::dto;
pub use crate::permissions::PermissionKind;
use crate::permissions::{self, RequestAction, StartDecision, SystemAudioSeen};

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

/// How long one transcriber call may run before Stop gives up on it, keeps
/// what was transcribed and closes the meeting (also at quit).
///
/// A chunk holds at most 28 s of audio (`ChunkerConfig::default`). A call
/// still running after 120 s is more than four times slower than real time
/// on that chunk: a model that slow could not have kept up with the meeting
/// anyway, so the call is taken to be stuck. The limit is per call, so a
/// long backlog that the transcriber is working through is never cut short;
/// the drain progress in the recording state shows it.
#[cfg(not(test))]
const CALL_LIMIT: Duration = Duration::from_secs(120);
#[cfg(test)]
const CALL_LIMIT: Duration = Duration::from_millis(300);

const NOT_RECORDING: &str = "No meeting is being recorded.";
const ALREADY_RECORDING: &str = "A meeting is already being recorded.";
const NO_MODEL: &str = "No transcription model is installed. Download one in Settings.";
const QUITTING: &str = "Mneme is quitting.";

fn msg(error: impl Display) -> String {
    error.to_string()
}

fn log(message: impl Display) {
    eprintln!("mneme: {message}");
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
    /// A start began loading its model. The window has been sent an idle
    /// state with `starting` set; the menu bar redraws.
    fn starting(&self) {}
}

/// A meeting that is being recorded or stopped.
#[derive(Clone)]
struct Live {
    meeting_id: MeetingId,
    model_id: String,
    session: Arc<Session>,
    /// The store the meeting was created in.
    store: Arc<Store>,
    /// System audio was already known to be refused when the meeting
    /// started, and the log has said so.
    mic_only: bool,
    /// What the window is told about a channel that is not being recorded.
    /// Set at the start when the meeting is mic only, and by the watcher
    /// when a source fails. The first reason stays.
    warning: Arc<Mutex<Option<String>>>,
    /// The warning the start gave, if any. The system source is still
    /// tried, so it is taken back if that source delivers audio after all.
    start_warning: Option<String>,
}

impl Live {
    fn warning(&self) -> Option<String> {
        self.warning
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn warn(&self, why: String) {
        self.warning
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(why);
    }

    /// Takes back the start's warning: system audio is being recorded.
    fn withdraw_start_warning(&self) {
        let mut warning = self.warning.lock().unwrap_or_else(PoisonError::into_inner);
        if self.start_warning.is_some() && *warning == self.start_warning {
            *warning = None;
        }
    }
}

/// One recording at a time: the whole state of it.
enum Recording {
    Idle,
    /// `startMeeting` is running: the model is loading. Reported as idle
    /// with `starting` set, and a second start is refused.
    Starting {
        model_id: String,
    },
    Active {
        live: Live,
        /// The loaded model. [`Warm`] holds it too, so it outlives the
        /// recording; `shutdown` frees both (issue 005).
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
            Recording::Idle => return dto::RecordingState::idle(),
            Recording::Starting { model_id } => {
                return dto::RecordingState {
                    model_id: Some(model_id.clone()),
                    starting: true,
                    ..dto::RecordingState::idle()
                };
            }
            Recording::Active { .. } => "recording",
            Recording::Stopping(_) => "stopping",
        };
        let live = self.live().expect("active and stopping have a meeting");
        dto::RecordingState {
            status,
            meeting_id: Some(live.meeting_id.as_str().to_owned()),
            elapsed_secs: live.session.elapsed().as_secs(),
            model_id: Some(live.model_id.clone()),
            warning: live.warning(),
            starting: false,
        }
    }
}

/// The model kept loaded between meetings, so New meeting does not wait for
/// a load (seconds, plus about 7 s on the first load while Metal compiles
/// its shaders). One engine at most. It holds 0.1 to 1.6 GB of memory,
/// depending on the model, for as long as the app runs; that is accepted
/// for the faster start.
///
/// An engine still alive when the process exits aborts it (issue 005), so
/// only this cache and a recording ever hold one, and `shutdown` frees both.
#[derive(Default)]
struct Warm {
    /// The loaded engine and the id of its model.
    engine: Option<(String, Arc<dyn Transcriber>)>,
    /// The model a load under way is loading. One load runs at a time;
    /// anyone else who needs an engine waits for it.
    loading: Option<String>,
    /// The model of the load under way was deleted meanwhile: its engine
    /// is not kept.
    discard: bool,
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
    /// What this run's recordings learned about the system audio
    /// permission, the only way to know it without private API.
    system_audio_seen: Mutex<SystemAudioSeen>,
    /// Never locked before `recording`: the order is `recording`, `warm`.
    warm: Mutex<Warm>,
    /// Signalled whenever a load ends.
    warm_changed: Condvar,
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
            system_audio_seen: Mutex::default(),
            warm: Mutex::default(),
            warm_changed: Condvar::new(),
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

    fn warm(&self) -> MutexGuard<'_, Warm> {
        self.warm.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// A start is loading its model: the state still says idle, but a
    /// second start would be refused.
    pub fn is_starting(&self) -> bool {
        matches!(*self.recording(), Recording::Starting { .. })
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
                return Err(QUITTING.to_owned());
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
        self.changed.notify_all();
        // Still idle, so the window can disable its own start meanwhile.
        self.emit_state();
        self.host.starting();

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
        let warning = match permissions::start_decision(microphone, self.system_audio_permission())
        {
            StartDecision::Record => None,
            // Logged, and said in the window for as long as the meeting runs.
            StartDecision::MicOnly(why) => {
                log(&why);
                Some(why)
            }
            StartDecision::Refuse(why) => return Err(why),
        };

        // The model first: if it cannot be loaded, no meeting folder is left
        // behind. Usually it is already warm.
        let engine = self
            .warm_engine(model_id, model_path, true)?
            .expect("a reference was asked for");

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
                    mic_only: warning.is_some(),
                    warning: Arc::new(Mutex::new(warning.clone())),
                    start_warning: warning,
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

    /// Loads the model a start would pick on a thread of its own and keeps
    /// it, so the next New meeting does not wait for it. Run after launch
    /// and whenever the pick may have changed. Waits for a recording to end
    /// first, so a load does not compete with a meeting's transcription.
    /// Does nothing when no model is installed.
    pub fn warm_up(self: &Arc<Self>) {
        let core = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("mneme-warm-up".to_owned())
            .spawn(move || {
                {
                    let mut recording = core.recording();
                    while !matches!(*recording, Recording::Idle) {
                        recording = core
                            .changed
                            .wait(recording)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                }
                let Ok((id, path)) = core.pick_model() else {
                    return;
                };
                if let Err(error) = core.warm_engine(&id, &path, false)
                    && error != QUITTING
                {
                    log(format_args!("loading {id} ahead of a meeting: {error}"));
                }
            });
        if let Err(error) = spawned {
            log(format_args!("warm-up thread: {error}"));
        }
    }

    /// The engine for this model: the warm one if it is this model, else
    /// loaded now and kept in its place. Waits for a load under way rather
    /// than loading alongside it. Never called with `recording` locked.
    ///
    /// `want`: return a reference. The warm-up does not ask for one, so its
    /// thread never holds the engine once the load is over: `shutdown` may
    /// return the moment the load ends.
    fn warm_engine(
        &self,
        model_id: &str,
        model_path: &Path,
        want: bool,
    ) -> Result<Option<Arc<dyn Transcriber>>> {
        let mut warm = self.warm();
        loop {
            if self.quitting.load(Ordering::SeqCst) {
                return Err(QUITTING.to_owned());
            }
            if let Some((id, engine)) = &warm.engine
                && id == model_id
            {
                return Ok(want.then(|| Arc::clone(engine)));
            }
            if warm.loading.is_none() {
                break;
            }
            warm = self
                .warm_changed
                .wait(warm)
                .unwrap_or_else(PoisonError::into_inner);
        }
        // Checked under the lock `delete_model` holds while it deletes.
        if !self.models.is_installed(model_id) {
            return Err(format!("Model {model_id} is not installed."));
        }
        let previous = warm.engine.take();
        warm.loading = Some(model_id.to_owned());
        drop(warm);
        // One engine at a time: the other model goes before this one loads.
        drop(previous);

        let loaded = self.host.load_transcriber(model_path);

        let mut warm = self.warm();
        let discard = std::mem::take(&mut warm.discard);
        // Settled before the load is marked over, so an engine nobody keeps
        // is already gone when `shutdown` sees the load end.
        let result = match loaded {
            Ok(engine) if self.quitting.load(Ordering::SeqCst) => {
                drop(engine);
                Err(QUITTING.to_owned())
            }
            Ok(engine) => {
                if !discard {
                    warm.engine = Some((model_id.to_owned(), Arc::clone(&engine)));
                }
                Ok(want.then_some(engine))
            }
            Err(error) => Err(error),
        };
        warm.loading = None;
        drop(warm);
        self.warm_changed.notify_all();
        result
    }

    /// Runs on its own thread for one meeting: sends the state every tick,
    /// and finishes the meeting if the session ends by itself (every source
    /// ended, or the session failed).
    fn watch(&self, meeting_id: &MeetingId) {
        // The channels already reported as lost: each is said once.
        let mut lost: Vec<Speaker> = Vec::new();
        let mut next_tick = Instant::now() + self.tick;
        loop {
            let live = {
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
                    Recording::Active { live, .. } => {
                        if live.mic_only && !lost.contains(&Speaker::Them) {
                            lost.push(Speaker::Them);
                        }
                        live.clone()
                    }
                    _ => return,
                }
            };
            let session = Arc::clone(&live.session);
            let status = session.status();
            self.learn_system_audio(&status.sources);
            let them_heard = status.sources.iter().any(|source| {
                source.speaker == Speaker::Them && source.frames > 0 && !source.phase.is_failure()
            });
            if them_heard {
                live.withdraw_start_warning();
            }
            for source in &status.sources {
                if source.phase.is_failure() && !lost.contains(&source.speaker) {
                    lost.push(source.speaker);
                    let why =
                        permissions::channel_lost(source.speaker, source.phase, &source.errors);
                    log(format_args!("{meeting_id}: {why}"));
                    live.warn(why);
                }
            }
            // Capture is over without a stop: nothing is being recorded any
            // more, so the state must not go on saying `recording`. `stop`
            // sends `stopping` at once and `idle` when the transcript is
            // complete.
            let over = matches!(
                status.end,
                Some(EndReason::SourcesEnded | EndReason::CaptureFailed)
            );
            if over || session.is_done() {
                if status.end == Some(EndReason::SourcesEnded) {
                    log(format_args!(
                        "{meeting_id}: no audio source is left, so the recording was ended"
                    ));
                }
                if let Err(error) = self.stop(Some(meeting_id), false) {
                    log(format_args!("finishing {meeting_id}: {error}"));
                }
                return;
            }
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
    /// waiting. Either way a transcriber call that runs past [`CALL_LIMIT`]
    /// is given up on, so this always returns.
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

        if hurry {
            live.session.request_stop();
            if live.session.wait_timeout(QUIT_GRACE).is_none() {
                live.session.abandon_backlog();
            }
        }
        let summary = live.session.stop_or_give_up(CALL_LIMIT);
        if report_stuck(&live.meeting_id, &summary) {
            // The engine is stuck in a call: the next meeting must not get
            // it from the warm cache. It loads a fresh one.
            self.forget_warm(&engine);
        }
        // The session's reference to the engine is gone once it has
        // returned; the warm cache usually keeps the model for the next
        // meeting. If the session gave up on a stuck transcriber call, the
        // thread stuck in it still holds the engine: it is freed if that
        // call ever returns, and if the app quits first, whisper.cpp aborts
        // the process at exit (issue 005). The meeting is closed and its
        // transcript sorted before that.
        drop(engine);
        let result = self.close_meeting(&live, &summary);
        // Idle only now: a `getMeeting` right after the idle state must see
        // the final status, duration and line count.
        self.set_recording(Recording::Idle);
        result
    }

    fn close_meeting(&self, live: &Live, summary: &SessionSummary) -> Result<dto::Meeting> {
        let id = &live.meeting_id;
        // A meeting shorter than a tick is only seen here.
        self.learn_system_audio(&summary.sources);
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

    /// Drops this engine from the warm cache, if it is the one there.
    fn forget_warm(&self, engine: &Arc<dyn Transcriber>) {
        let mut warm = self.warm();
        let same = |(_, cached): &mut (String, Arc<dyn Transcriber>)| {
            std::ptr::addr_eq(Arc::as_ptr(cached), Arc::as_ptr(engine))
        };
        let forgotten = warm.engine.take_if(same);
        drop(warm);
        drop(forgotten);
    }

    /// Called when the app is about to exit: refuses new recordings and
    /// loads, waits for a start or stop that is under way, stops a running
    /// recording, waits for a model load under way, and frees the warm
    /// engine. When it returns no session and no engine is alive. Safe to
    /// call more than once.
    pub fn shutdown(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        loop {
            let recording = self.recording();
            match &*recording {
                Recording::Idle => break,
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
        // A load that ends now drops its engine, since `quitting` is set.
        let mut warm = self.warm();
        while warm.loading.is_some() {
            warm = self
                .warm_changed
                .wait(warm)
                .unwrap_or_else(PoisonError::into_inner);
        }
        let engine = warm.engine.take();
        drop(warm);
        drop(engine);
    }

    // ---- Models ----

    pub fn list_models(&self) -> Vec<dto::ModelInfo> {
        self.models.list().into_iter().map(Into::into).collect()
    }

    /// Starts a download and returns at once; progress and the outcome
    /// arrive as download progress events. An installed model may be the
    /// one a start now picks, so the pick is warmed up.
    pub fn download_model(self: &Arc<Self>, id: &str) -> Result<()> {
        let host = Arc::clone(&self.host);
        // Weak: a download does not keep the core alive.
        let core = Arc::downgrade(self);
        self.models
            .start_download(id, move |progress| {
                let progress: dto::DownloadProgress = progress.into();
                host.download_progress(&progress);
                if progress.state == "done"
                    && let Some(core) = core.upgrade()
                {
                    core.warm_up();
                }
            })
            .map(drop)
            .map_err(msg)
    }

    pub fn cancel_download(&self, id: &str) {
        self.models.cancel_download(id);
    }

    /// Deletes a model file. Refused for the model the current recording
    /// uses. A warm engine of it is freed first. Deleting the default model
    /// clears the default. Then the pick is warmed up again.
    pub fn delete_model(self: &Arc<Self>, id: &str) -> Result<()> {
        let name = self.models.spec(id).map_err(msg)?.name.clone();
        let (deleted, freed) = {
            // Held across the delete so a start cannot pick the model in
            // between.
            let recording = self.recording();
            if recording.model_id() == Some(id) {
                return Err(format!(
                    "{name} is in use by the current recording and cannot be deleted."
                ));
            }
            // Also held across it, so no load of the model begins meanwhile.
            let mut warm = self.warm();
            let freed = warm.engine.take_if(|(warm_id, _)| warm_id == id).is_some();
            if warm.loading.as_deref() == Some(id) {
                warm.discard = true;
            }
            (self.models.delete(id).map_err(msg), freed)
        };
        let result = deleted.and_then(|()| {
            if self.settings.get().default_model_id.as_deref() != Some(id) {
                return Ok(());
            }
            let patch = SettingsPatch {
                default_model_id: Patch::Clear,
                ..SettingsPatch::default()
            };
            self.settings.update(&patch).map(drop).map_err(msg)
        });
        // The pick may be another model now.
        if freed || result.is_ok() {
            self.warm_up();
        }
        result
    }

    // ---- Settings ----

    pub fn get_settings(&self) -> Settings {
        self.settings.get()
    }

    /// Applies a patch and returns the settings, which are on disk by then.
    /// Refuses a default model that is not installed. A new default model is
    /// warmed up in place of the old one. A new meetings root (not in the
    /// contract, but in the settings file) is refused during a recording and
    /// otherwise opens, and recovers, a store on it.
    pub fn update_settings(self: &Arc<Self>, patch: &SettingsPatch) -> Result<Settings> {
        let before = self.settings.get().default_model_id;
        let result = self.apply_settings(patch);
        if let Ok(after) = &result
            && after.default_model_id != before
        {
            self.warm_up();
        }
        result
    }

    fn apply_settings(&self, patch: &SettingsPatch) -> Result<Settings> {
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

    /// The microphone is read from macOS every time, so a change made in
    /// System Settings shows at the next read.
    pub fn permissions(&self) -> dto::Permissions {
        dto::Permissions {
            microphone: permissions::status_name(self.host.microphone_permission()),
            system_audio: self.system_audio_permission(),
        }
    }

    /// macOS has no public way to read this permission, so it is
    /// `undetermined` until a recording of this run settled it: `denied`
    /// once the system source was refused access, `granted` once it
    /// delivered audio.
    fn system_audio_permission(&self) -> &'static str {
        let seen = *self
            .system_audio_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        permissions::system_audio_status(self.host.system_audio_permission(), seen)
    }

    /// Remembers what the system source of a recording says about the
    /// permission. The latest recording wins: access given or taken away in
    /// System Settings shows at the next meeting.
    fn learn_system_audio(&self, sources: &[SourceStatus]) {
        let evidence = sources
            .iter()
            .filter(|source| source.speaker == Speaker::Them)
            .find_map(|s| permissions::system_audio_evidence(s.phase, &s.errors, s.frames));
        if let Some(seen) = evidence {
            *self
                .system_audio_seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = seen;
        }
    }

    fn permission(&self, kind: PermissionKind) -> &'static str {
        match kind {
            PermissionKind::Microphone => {
                permissions::status_name(self.host.microphone_permission())
            }
            PermissionKind::SystemAudio => self.system_audio_permission(),
        }
    }

    /// Asks for a permission and returns what [`permissions`](Self::permissions)
    /// reports afterwards. May block on a system prompt: call it off the UI
    /// thread. For a permission that is already denied, macOS shows no
    /// prompt again, so this opens its pane in System Settings instead and
    /// returns `denied`. It never fails.
    pub fn request_permission(&self, kind: PermissionKind) -> &'static str {
        match permissions::request_action(self.permission(kind)) {
            RequestAction::Nothing => {}
            RequestAction::OpenSettings => {
                if let Err(error) = self.host.open_privacy_settings(kind) {
                    log(format_args!("opening {}: {error}", kind.pane()));
                }
            }
            RequestAction::Prompt => match kind {
                PermissionKind::Microphone => {
                    self.host.request_microphone_permission();
                }
                PermissionKind::SystemAudio => {
                    // Opens a tap for a moment, which makes macOS ask. Only
                    // a yes is kept: a refusal read here can be a prompt
                    // that is still open, so it stays `undetermined` until
                    // a recording is refused.
                    if self.host.request_system_audio_permission()
                        == Some(PermissionStatus::Granted)
                    {
                        *self
                            .system_audio_seen
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner) = SystemAudioSeen::Delivering;
                    }
                }
            },
        }
        self.permission(kind)
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

/// Logs a transcriber call that Stop gave up on, and what was lost with it.
/// Returns whether there was one.
fn report_stuck(id: &MeetingId, summary: &SessionSummary) -> bool {
    let Some(stuck) = summary
        .dropped
        .iter()
        .find(|chunk| chunk.reason == mneme_pipeline::DropReason::TranscriberStuck)
    else {
        return false;
    };
    let lost_ms: u64 = summary
        .dropped
        .iter()
        .filter(|chunk| {
            matches!(
                chunk.reason,
                mneme_pipeline::DropReason::TranscriberStuck
                    | mneme_pipeline::DropReason::DrainAbandoned
            )
        })
        .map(|chunk| chunk.duration_ms)
        .sum();
    log(format_args!(
        "{id}: a transcriber call did not return within {} s; gave up on it \
         ({} at {} ms) and on the audio still waiting: {} ms of speech not \
         transcribed. {} lines were kept.",
        CALL_LIMIT.as_secs(),
        stuck.speaker,
        stuck.start_ms,
        lost_ms,
        summary.lines
    ));
    true
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
