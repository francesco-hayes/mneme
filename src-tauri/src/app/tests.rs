//! The core with fake sources and a fake transcriber: no Tauri runtime, no
//! devices, no model, no permission prompts.

use std::fs;
use std::sync::Weak;
use std::sync::mpsc::{Receiver, Sender, channel};

use mneme_audio::{FakeSource, FrameSender, Pacing};
use mneme_core::SAMPLE_RATE;
use mneme_store::{MeetingStatus, NOTES_FILE, TIMELINE_FILE};
use mneme_transcribe::{FakeTranscriber, ModelSpec, TextSpan, TranscribeError};
use tempfile::TempDir;

use super::*;

const WAIT: Duration = Duration::from_secs(20);
/// A tick no test reaches: the watcher then only ends with the recording.
const NEVER: Duration = Duration::from_secs(3600);

/// A transcriber that notes the language of every call in a log that
/// outlives it, so a test can also check that the engine itself is gone.
struct Spy {
    inner: FakeTranscriber,
    languages: Arc<Mutex<Vec<Language>>>,
    hold: Option<Hold>,
}

/// Makes every call from the `from_call`th on (counted from 1) wait until
/// `released` is set: a transcriber that never returns, until it does.
#[derive(Clone)]
struct Hold {
    from_call: usize,
    released: Arc<(Mutex<bool>, Condvar)>,
}

impl Hold {
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}

impl Transcriber for Spy {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> std::result::Result<Vec<TextSpan>, TranscribeError> {
        let call = {
            let mut languages = self.languages.lock().unwrap();
            languages.push(language.clone());
            languages.len()
        };
        if let Some(hold) = self.hold.as_ref().filter(|hold| call >= hold.from_call) {
            let mut released = hold.released.0.lock().unwrap();
            while !*released {
                released = hold.released.1.wait(released).unwrap();
            }
        }
        self.inner.transcribe(samples, language)
    }
}

struct TestHost {
    meetings_root: PathBuf,
    states: Mutex<Vec<dto::RecordingState>>,
    /// The statuses on disk at the moment each idle state was sent.
    on_disk_at_idle: Mutex<Vec<Vec<MeetingStatus>>>,
    progress: Mutex<Vec<dto::DownloadProgress>>,
    loaded: Mutex<Vec<PathBuf>>,
    engine: Mutex<Weak<Spy>>,
    languages: Arc<Mutex<Vec<Language>>>,
    /// When set, `load_transcriber` waits for a message before returning.
    load_gate: Mutex<Option<Receiver<()>>>,
    /// When set, the engine's calls block from a given call on.
    hold: Option<Hold>,
    pacing: Pacing,
    /// Length of each source's tape.
    tape_ms: u64,
    microphone: Mutex<PermissionStatus>,
    microphone_answer: PermissionStatus,
    microphone_requests: Mutex<u32>,
    system_audio: Option<PermissionStatus>,
    system_audio_answer: Option<PermissionStatus>,
    /// The system source's access check is refused, as it is without the
    /// System Audio Recording permission.
    system_audio_refused: AtomicBool,
    revealed: Mutex<Vec<PathBuf>>,
    settings_opened: Mutex<Vec<PermissionKind>>,
}

impl TestHost {
    fn new(meetings_root: PathBuf) -> Self {
        Self {
            meetings_root,
            states: Mutex::default(),
            on_disk_at_idle: Mutex::default(),
            progress: Mutex::default(),
            loaded: Mutex::default(),
            engine: Mutex::new(Weak::new()),
            languages: Arc::default(),
            load_gate: Mutex::new(None),
            hold: None,
            pacing: Pacing::RealTime,
            tape_ms: 60_000,
            microphone: Mutex::new(PermissionStatus::Granted),
            microphone_answer: PermissionStatus::Granted,
            microphone_requests: Mutex::new(0),
            system_audio: None,
            system_audio_answer: None,
            system_audio_refused: AtomicBool::new(false),
            revealed: Mutex::default(),
            settings_opened: Mutex::default(),
        }
    }

    fn statuses(&self) -> Vec<&'static str> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .map(|s| if s.starting { "starting" } else { s.status })
            .collect()
    }

    fn engine_is_alive(&self) -> bool {
        self.engine.lock().unwrap().upgrade().is_some()
    }
}

/// Low noise with one 600 ms tone ("speech") starting at `burst_at_ms`. With
/// the default chunker that is one chunk, cut about 700 ms after the tone.
fn tape(burst_at_ms: u64, total_ms: u64) -> Vec<f32> {
    let per_ms = u64::from(SAMPLE_RATE) / 1000;
    let step = std::f32::consts::TAU * 220.0 / SAMPLE_RATE as f32;
    let mut seed = 0x9E37_79B9_u32;
    (0..total_ms * per_ms)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
            let ms = i / per_ms;
            if (burst_at_ms..burst_at_ms + 600).contains(&ms) {
                0.3 * (i as f32 * step).sin() + 0.03 * noise
            } else {
                0.0008 * noise
            }
        })
        .collect()
}

/// A system source that fails the way `SystemAudioSource` does when macOS
/// refuses its access check.
struct RefusedSystemAudio;

impl AudioSource for RefusedSystemAudio {
    fn speaker(&self) -> Speaker {
        Speaker::Them
    }

    fn start(&mut self, _: Instant, _: FrameSender) -> std::result::Result<(), AudioError> {
        Err(AudioError::PermissionDenied(
            "system audio recording is not allowed".to_owned(),
        ))
    }

    fn stop(&mut self) {}

    fn is_running(&self) -> bool {
        false
    }

    fn take_errors(&mut self) -> Vec<AudioError> {
        Vec::new()
    }
}

impl Host for TestHost {
    fn recording_state(&self, state: &dto::RecordingState) {
        // A start that is loading is idle too, but no meeting has closed.
        if state.status == "idle" && !state.starting {
            let store = Store::open(&self.meetings_root).unwrap();
            let list = store.list_meetings().unwrap();
            let statuses = list.meetings.iter().map(|m| m.status).collect();
            self.on_disk_at_idle.lock().unwrap().push(statuses);
        }
        self.states.lock().unwrap().push(state.clone());
    }

    fn download_progress(&self, progress: &dto::DownloadProgress) {
        self.progress.lock().unwrap().push(progress.clone());
    }

    fn load_transcriber(&self, model_path: &Path) -> Result<Arc<dyn Transcriber>> {
        self.loaded.lock().unwrap().push(model_path.to_path_buf());
        // Taken in its own statement so the lock is not held while waiting.
        let gate = self.load_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.recv().map_err(msg)?;
        }
        let engine = Arc::new(Spy {
            inner: FakeTranscriber::new(),
            languages: Arc::clone(&self.languages),
            hold: self.hold.clone(),
        });
        *self.engine.lock().unwrap() = Arc::downgrade(&engine);
        Ok(engine)
    }

    fn sources(&self) -> Vec<Box<dyn AudioSource>> {
        let source = |speaker, burst_at_ms| {
            let samples = tape(burst_at_ms, self.tape_ms);
            Box::new(FakeSource::from_samples(speaker, samples).with_pacing(self.pacing))
                as Box<dyn AudioSource>
        };
        if self.system_audio_refused.load(Ordering::SeqCst) {
            return vec![source(Speaker::Me, 300), Box::new(RefusedSystemAudio)];
        }
        vec![source(Speaker::Me, 300), source(Speaker::Them, 500)]
    }

    fn recorder_options(&self) -> RecorderOptions {
        RecorderOptions {
            head_start: Duration::ZERO,
        }
    }

    fn microphone_permission(&self) -> PermissionStatus {
        *self.microphone.lock().unwrap()
    }

    fn request_microphone_permission(&self) -> PermissionStatus {
        *self.microphone_requests.lock().unwrap() += 1;
        *self.microphone.lock().unwrap() = self.microphone_answer;
        self.microphone_answer
    }

    fn system_audio_permission(&self) -> Option<PermissionStatus> {
        self.system_audio
    }

    fn request_system_audio_permission(&self) -> Option<PermissionStatus> {
        self.system_audio_answer
    }

    fn reveal(&self, path: &Path) -> Result<()> {
        self.revealed.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }

    fn open_privacy_settings(&self, kind: PermissionKind) -> Result<()> {
        self.settings_opened.lock().unwrap().push(kind);
        Ok(())
    }
}

/// A temp app data folder and Documents folder, and a core opened on them.
struct Fixture {
    dir: TempDir,
    host: Arc<TestHost>,
    core: Arc<AppCore>,
}

fn spec(id: &str) -> ModelSpec {
    ModelSpec {
        id: id.to_owned(),
        name: format!("Model {id}"),
        file_name: format!("ggml-{id}.bin"),
        url: format!("https://models.invalid/ggml-{id}.bin"),
        size_bytes: 4,
        sha256: "0".repeat(64),
        english_only: true,
        retired: false,
        note: String::new(),
    }
}

impl Fixture {
    fn data_dir(dir: &TempDir) -> PathBuf {
        dir.path().join("data")
    }

    fn documents_dir(dir: &TempDir) -> PathBuf {
        dir.path().join("Documents")
    }

    fn meetings_root(dir: &TempDir) -> PathBuf {
        Self::documents_dir(dir).join(MEETINGS_DIR)
    }

    /// A two-model catalog (`small`, `large`) with the named ones installed.
    fn new(installed: &[&str], tick: Duration, tune: impl FnOnce(&mut TestHost)) -> Self {
        Self::open_in(TempDir::new().unwrap(), installed, tick, tune)
    }

    fn open_in(
        dir: TempDir,
        installed: &[&str],
        tick: Duration,
        tune: impl FnOnce(&mut TestHost),
    ) -> Self {
        let models_dir = Self::data_dir(&dir).join(MODELS_DIR);
        fs::create_dir_all(&models_dir).unwrap();
        for id in installed {
            fs::write(models_dir.join(format!("ggml-{id}.bin")), b"1234").unwrap();
        }
        let models =
            ModelStore::open_with_catalog(&models_dir, vec![spec("small"), spec("large")]).unwrap();
        let mut host = TestHost::new(Self::meetings_root(&dir));
        tune(&mut host);
        let host = Arc::new(host);
        let core = AppCore::open_with(
            &Self::data_dir(&dir),
            &Self::documents_dir(&dir),
            models,
            Arc::clone(&host) as Arc<dyn Host>,
            tick,
        )
        .unwrap();
        Self { dir, host, core }
    }

    fn wait_for(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// The model of the warm engine, if one is kept.
    fn warm_id(&self) -> Option<String> {
        self.core.warm().engine.as_ref().map(|(id, _)| id.clone())
    }

    fn wait_for_warm(&self, id: &str) {
        self.wait_for("the warm model", || self.warm_id().as_deref() == Some(id));
    }

    /// The file names of every model load so far.
    fn loaded(&self) -> Vec<String> {
        let loaded = self.host.loaded.lock().unwrap();
        let names = loaded.iter().map(|path| path.file_name().unwrap());
        names
            .map(|name| name.to_str().unwrap().to_owned())
            .collect()
    }

    fn wait_for_lines(&self, id: &str, lines: usize) {
        self.wait_for("transcript lines", || {
            self.core.get_meeting(id).unwrap().transcript_line_count >= lines
        });
    }
}

#[test]
fn start_then_stop_gives_a_complete_meeting_with_transcript_duration_and_note_timeline() {
    let f = Fixture::new(&["small"], Duration::from_millis(100), |_| {});

    let started = f.core.start_meeting().unwrap();
    let id = started.summary.id.clone();
    assert_eq!(started.summary.status, "recording");
    assert_eq!(started.summary.duration_secs, None);
    assert!(Path::new(&started.folder_path).join(NOTES_FILE).is_file());
    assert_eq!(f.core.recording_state().status, "recording");
    assert_eq!(f.core.recording_state().meeting_id.as_deref(), Some(&*id));
    assert_eq!(f.core.list_meetings().unwrap()[0].status, "recording");

    // Both speakers' speech reaches the transcript while recording.
    f.wait_for_lines(&id, 2);
    // Notes saved now get a time on the meeting clock.
    f.core.save_notes(&id, "decided to ship\n").unwrap();
    f.wait_for("a second of meeting time", || {
        f.core.recording_state().elapsed_secs >= 1
    });

    let stopped = f.core.stop_meeting().unwrap();
    assert_eq!(stopped.summary.id, id);
    assert_eq!(stopped.summary.status, "complete");
    assert!(stopped.summary.duration_secs.unwrap() >= 1);
    assert_eq!(stopped.transcript_line_count, 2);
    assert_eq!(stopped.notes.trim(), "decided to ship");

    // What `getMeeting` and the files say right after stop.
    let read = f.core.get_meeting(&id).unwrap();
    assert_eq!(read, stopped);
    let transcript = f.core.get_transcript(&id).unwrap();
    let speakers: Vec<_> = transcript.iter().map(|line| line.speaker).collect();
    assert_eq!(speakers, ["Me", "Them"]);
    assert_eq!(transcript[0].t, "00:00:00");
    assert!(
        transcript
            .iter()
            .all(|line| line.text.starts_with("chunk "))
    );
    let folder = PathBuf::from(&stopped.folder_path);
    let timeline = fs::read_to_string(folder.join(TIMELINE_FILE)).unwrap();
    assert_eq!(timeline.lines().count(), 1, "{timeline}");
    assert!(timeline.contains("decided to ship"), "{timeline}");

    // The state the window saw: starting while the model loaded, recording
    // at least twice (the change and a tick), then stopping, then idle, and
    // nothing after it.
    let statuses = f.host.statuses();
    assert_eq!(statuses[0], "starting");
    assert_eq!(statuses.iter().filter(|s| **s == "starting").count(), 1);
    let recording = statuses.iter().filter(|s| **s == "recording").count();
    assert!(recording >= 2, "{statuses:?}");
    assert_eq!(statuses[statuses.len() - 2..], ["stopping", "idle"]);
    assert_eq!(statuses.iter().filter(|s| **s == "idle").count(), 1);
    // Idle was sent only once the meeting was closed on disk.
    assert_eq!(
        *f.host.on_disk_at_idle.lock().unwrap(),
        [vec![MeetingStatus::Complete]]
    );
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());

    // English only, and the engine is kept for the next meeting (030).
    let languages = f.host.languages.lock().unwrap().clone();
    assert_eq!(languages, [Language::english(), Language::english()]);
    assert!(f.host.engine_is_alive());
    assert_eq!(f.warm_id().as_deref(), Some("small"));

    // A late save after stop is a plain save: saved, no timeline entry.
    f.core
        .save_notes(&id, "decided to ship\nlate line\n")
        .unwrap();
    assert!(f.core.get_meeting(&id).unwrap().notes.contains("late line"));
    let timeline = fs::read_to_string(folder.join(TIMELINE_FILE)).unwrap();
    assert_eq!(timeline.lines().count(), 1, "{timeline}");

    // And a second stop has nothing to stop.
    assert_eq!(f.core.stop_meeting().unwrap_err(), NOT_RECORDING);
}

#[test]
fn a_second_start_is_refused_while_recording() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    let first = f.core.start_meeting().unwrap();

    assert_eq!(f.core.start_meeting().unwrap_err(), ALREADY_RECORDING);
    // Nothing was created or loaded for the refused start.
    assert_eq!(f.core.list_meetings().unwrap().len(), 1);
    assert_eq!(f.host.loaded.lock().unwrap().len(), 1);
    assert_eq!(
        f.core.recording_state().meeting_id,
        Some(first.summary.id.clone())
    );
    // The model in use cannot be deleted.
    let error = f.core.delete_model("small").unwrap_err();
    assert!(error.contains("in use by the current recording"), "{error}");
    assert!(f.core.list_models()[0].installed);

    f.core.stop_meeting().unwrap();
    // Once stopped, a new meeting can start.
    let second = f.core.start_meeting().unwrap();
    assert_ne!(second.summary.id, first.summary.id);
    f.core.stop_meeting().unwrap();
}

#[test]
fn a_second_start_is_refused_while_the_model_is_still_loading() {
    let (open_gate, gate): (Sender<()>, Receiver<()>) = channel();
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.load_gate = Mutex::new(Some(gate));
    });
    let core = Arc::clone(&f.core);
    let first = thread::spawn(move || core.start_meeting());
    f.wait_for("the model load to begin", || {
        !f.host.loaded.lock().unwrap().is_empty()
    });

    // Loading is not yet recording, but the slot is taken, and the window
    // has been told so.
    let loading = dto::RecordingState {
        model_id: Some("small".to_owned()),
        starting: true,
        ..dto::RecordingState::idle()
    };
    assert_eq!(f.core.recording_state(), loading);
    assert_eq!(f.host.states.lock().unwrap().last(), Some(&loading));
    assert!(f.core.is_starting());
    assert_eq!(f.core.start_meeting().unwrap_err(), ALREADY_RECORDING);
    assert!(f.core.delete_model("small").is_err());
    assert_eq!(f.core.stop_meeting().unwrap_err(), NOT_RECORDING);

    open_gate.send(()).unwrap();
    let meeting = first.join().unwrap().unwrap();
    assert!(!f.core.is_starting());
    let state = f.core.recording_state();
    assert_eq!(state.meeting_id, Some(meeting.summary.id));
    assert_eq!(state.model_id.as_deref(), Some("small"));
    assert_eq!(state.warning, None);
    f.core.stop_meeting().unwrap();
    assert!(!f.core.is_starting());
}

#[test]
fn the_state_says_why_system_audio_is_missing_until_it_is_heard() {
    let f = Fixture::new(&["small"], Duration::from_millis(50), |host| {
        host.system_audio_refused = AtomicBool::new(true);
    });

    // Not known to be refused at the start: the watcher finds out.
    f.core.start_meeting().unwrap();
    f.wait_for("the warning", || f.core.recording_state().warning.is_some());
    let warning = f.core.recording_state().warning.unwrap();
    assert!(
        warning.starts_with("System audio is not being recorded"),
        "{warning}"
    );
    f.core.stop_meeting().unwrap();
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());

    // Known to be refused now: the next meeting says so from the start.
    f.core.start_meeting().unwrap();
    let warning = f.core.recording_state().warning.unwrap();
    assert!(
        warning.starts_with("System audio access is denied"),
        "{warning}"
    );
    f.core.stop_meeting().unwrap();

    // Access given meanwhile: the warning is taken back once audio arrives.
    f.host.system_audio_refused.store(false, Ordering::SeqCst);
    f.core.start_meeting().unwrap();
    assert!(f.core.recording_state().warning.is_some());
    f.wait_for("the warning to go", || {
        f.core.recording_state().warning.is_none()
    });
    f.core.stop_meeting().unwrap();
}

#[test]
fn start_without_an_installed_model_is_refused_with_a_clear_message() {
    let f = Fixture::new(&[], NEVER, |_| {});

    let error = f.core.start_meeting().unwrap_err();
    assert_eq!(
        error,
        "No transcription model is installed. Download one in Settings."
    );
    assert!(f.core.list_meetings().unwrap().is_empty());
    assert!(f.host.loaded.lock().unwrap().is_empty());
    assert!(f.host.states.lock().unwrap().is_empty());
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());
    // The refusal did not leave the app stuck in "starting".
    assert_eq!(f.core.start_meeting().unwrap_err(), error);
}

#[test]
fn start_uses_the_default_model_or_else_any_installed_one() {
    let f = Fixture::new(&["small", "large"], NEVER, |_| {});
    let default = SettingsPatch {
        default_model_id: Patch::Set("large".to_owned()),
        ..SettingsPatch::default()
    };
    f.core.update_settings(&default).unwrap();
    f.core.start_meeting().unwrap();
    f.core.stop_meeting().unwrap();

    // The default is deleted, so the default is cleared and the other
    // installed model is used.
    f.core.delete_model("large").unwrap();
    assert_eq!(f.core.get_settings().default_model_id, None);
    f.core.start_meeting().unwrap();
    f.core.stop_meeting().unwrap();

    let loaded = f.host.loaded.lock().unwrap().clone();
    let names: Vec<_> = loaded
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["ggml-large.bin", "ggml-small.bin"]);
}

#[test]
fn start_is_refused_when_the_microphone_is_denied() {
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.microphone = Mutex::new(PermissionStatus::Denied);
    });
    let error = f.core.start_meeting().unwrap_err();
    assert!(error.starts_with("Microphone access is denied."), "{error}");
    assert!(f.core.list_meetings().unwrap().is_empty());
    assert!(f.host.loaded.lock().unwrap().is_empty());
    // The window saw the start begin and end, so its own start comes back.
    assert_eq!(f.host.statuses(), ["starting", "idle"]);
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());

    // Not asked yet: start asks, and a refusal there also ends the start.
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.microphone = Mutex::new(PermissionStatus::NotDetermined);
        host.microphone_answer = PermissionStatus::Denied;
    });
    assert!(f.core.start_meeting().is_err());
    assert_eq!(*f.host.microphone_requests.lock().unwrap(), 1);
    assert!(f.core.list_meetings().unwrap().is_empty());
}

#[test]
fn recovery_runs_on_launch_before_the_first_list() {
    let dir = TempDir::new().unwrap();
    // A run that crashed mid-meeting: the meeting is `recording` on disk.
    let crashed = {
        let store = Store::open(Fixture::meetings_root(&dir)).unwrap();
        let meeting = store
            .create_meeting(NewMeeting {
                started_at: StartTime::now_local(),
                model: None,
                language: None,
            })
            .unwrap();
        assert_eq!(meeting.status, MeetingStatus::Recording);
        meeting.id
    };

    let f = Fixture::open_in(dir, &["small"], NEVER, |_| {});

    // On disk, not only in what the list reports.
    let on_disk = Store::open(Fixture::meetings_root(&f.dir)).unwrap();
    assert_eq!(
        on_disk.meeting(&crashed).unwrap().status,
        MeetingStatus::Interrupted
    );
    let list = f.core.list_meetings().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].status, "interrupted");
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());
}

#[test]
fn a_meeting_left_recording_on_disk_is_never_shown_as_live() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    // Appears after launch, so recovery has not seen it; nothing records it.
    let other = Store::open(Fixture::meetings_root(&f.dir)).unwrap();
    let stray = other
        .create_meeting(NewMeeting {
            started_at: StartTime::now_local(),
            model: None,
            language: None,
        })
        .unwrap();

    assert_eq!(f.core.list_meetings().unwrap()[0].status, "interrupted");
    let read = f.core.get_meeting(stray.id.as_str()).unwrap();
    assert_eq!(read.summary.status, "interrupted");
}

#[test]
fn a_session_whose_sources_end_is_finished_without_a_stop() {
    let f = Fixture::new(&["small"], Duration::from_millis(30), |host| {
        host.pacing = Pacing::Fast;
        host.tape_ms = 2_000;
    });
    let id = f.core.start_meeting().unwrap().summary.id;

    f.wait_for("the meeting to finish by itself", || {
        f.host.statuses().last() == Some(&"idle")
    });
    let meeting = f.core.get_meeting(&id).unwrap();
    assert_eq!(meeting.summary.status, "complete");
    assert_eq!(meeting.transcript_line_count, 2);
    assert_eq!(
        *f.host.on_disk_at_idle.lock().unwrap(),
        [vec![MeetingStatus::Complete]]
    );
    // Kept warm for the next meeting.
    assert_eq!(f.warm_id().as_deref(), Some("small"));
    assert_eq!(f.core.stop_meeting().unwrap_err(), NOT_RECORDING);
}

#[test]
fn shutdown_while_recording_stops_the_meeting_and_frees_the_engine() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    let id = f.core.start_meeting().unwrap().summary.id;
    f.wait_for_lines(&id, 2);
    assert!(f.host.engine_is_alive());

    f.core.shutdown();

    assert!(!f.host.engine_is_alive());
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());
    let meeting = f.core.get_meeting(&id).unwrap();
    assert_eq!(meeting.summary.status, "complete");
    assert_eq!(meeting.transcript_line_count, 2);
    // Nothing starts once the app is on its way out, and a second call
    // (both exit events arrive) has nothing left to do.
    assert_eq!(f.core.start_meeting().unwrap_err(), "Mneme is quitting.");
    f.core.shutdown();
}

#[test]
fn notes_title_and_finder_work_on_a_meeting_that_is_not_recording() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    let id = f.core.start_meeting().unwrap().summary.id;
    let finished = f.core.stop_meeting().unwrap();

    f.core.save_notes(&id, "# Agenda\n").unwrap();
    assert_eq!(f.core.get_meeting(&id).unwrap().notes.trim(), "# Agenda");
    assert!(f.core.save_notes("no-such-meeting", "x").is_err());

    assert_eq!(
        f.core.rename_meeting(&id, "   ").unwrap_err(),
        "The title cannot be empty."
    );
    let renamed = f.core.rename_meeting(&id, "  Budget review ").unwrap();
    assert_eq!(renamed.title, "Budget review");
    assert_eq!(f.core.list_meetings().unwrap()[0].title, "Budget review");

    f.core.show_in_finder(&id).unwrap();
    let folder = PathBuf::from(f.core.get_meeting(&id).unwrap().folder_path);
    assert_ne!(folder, PathBuf::from(&finished.folder_path));
    assert_eq!(*f.host.revealed.lock().unwrap(), [folder]);
    assert!(f.core.show_in_finder("no-such-meeting").is_err());
}

#[test]
fn settings_are_checked_and_on_disk_when_the_update_returns() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    assert_eq!(f.core.get_settings().default_model_id, None);
    let set = |id: &str| SettingsPatch {
        default_model_id: Patch::Set(id.to_owned()),
        ..SettingsPatch::default()
    };

    let error = f.core.update_settings(&set("large")).unwrap_err();
    assert_eq!(
        error,
        "Model large is not installed, so it cannot be the default."
    );
    assert_eq!(f.core.get_settings().default_model_id, None);

    let updated = f.core.update_settings(&set("small")).unwrap();
    assert_eq!(updated.default_model_id.as_deref(), Some("small"));
    // Durable: a store opened on the same file reads it back.
    let (reopened, _) = SettingsStore::open(
        Fixture::data_dir(&f.dir).join(SETTINGS_FILE),
        Fixture::documents_dir(&f.dir).join(MEETINGS_DIR),
    )
    .unwrap();
    assert_eq!(reopened.get().default_model_id.as_deref(), Some("small"));

    // The patch arrives from the window as `Partial<Settings>` JSON.
    let patch: SettingsPatch = serde_json::from_str(r#"{"defaultModelId":null}"#).unwrap();
    assert_eq!(
        f.core.update_settings(&patch).unwrap().default_model_id,
        None
    );
    let json = serde_json::to_value(f.core.get_settings()).unwrap();
    assert_eq!(json["language"], "en");
    assert!(json["globalShortcut"].is_null());
}

#[test]
fn a_new_meetings_root_gets_its_own_store() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    f.core.start_meeting().unwrap();
    let elsewhere = f.dir.path().join("Elsewhere");
    let patch = SettingsPatch {
        meetings_root: Patch::Set(elsewhere.clone()),
        ..SettingsPatch::default()
    };
    // Not while a meeting records into the old one.
    assert!(f.core.update_settings(&patch).is_err());
    f.core.stop_meeting().unwrap();
    assert_eq!(f.core.list_meetings().unwrap().len(), 1);

    f.core.update_settings(&patch).unwrap();
    assert!(f.core.list_meetings().unwrap().is_empty());
    let meeting = f.core.start_meeting().unwrap();
    f.core.stop_meeting().unwrap();
    assert!(Path::new(&meeting.folder_path).starts_with(&elsewhere));
}

#[test]
fn models_are_listed_and_download_and_delete_are_checked() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    let models = f.core.list_models();
    let installed: Vec<_> = models.iter().map(|m| (&*m.id, m.installed)).collect();
    assert_eq!(installed, [("small", true), ("large", false)]);
    assert_eq!(models[0].name, "Model small");

    // Refused before anything starts, and without an event.
    assert!(f.core.download_model("no-such-model").is_err());
    assert!(f.core.download_model("small").is_err());
    assert!(f.core.delete_model("no-such-model").is_err());
    assert!(f.core.delete_model("large").is_err());
    f.core.cancel_download("large");
    assert!(f.host.progress.lock().unwrap().is_empty());

    f.core.delete_model("small").unwrap();
    assert!(f.core.list_models().iter().all(|m| !m.installed));
}

#[test]
fn permissions_report_what_is_known_and_a_request_agrees_with_the_next_read() {
    // Microphone not asked yet: the request asks, and the read agrees.
    let f = Fixture::new(&[], NEVER, |host| {
        host.microphone = Mutex::new(PermissionStatus::NotDetermined);
        host.microphone_answer = PermissionStatus::Granted;
    });
    let before = f.core.permissions();
    assert_eq!(before.microphone, "undetermined");
    // System audio cannot be read on macOS: unknown is `undetermined`.
    assert_eq!(before.system_audio, "undetermined");
    assert_eq!(
        f.core.request_permission(PermissionKind::Microphone),
        "granted"
    );
    assert_eq!(f.core.permissions().microphone, "granted");
    assert!(f.host.settings_opened.lock().unwrap().is_empty());

    // Already denied: no prompt can appear, so System Settings opens.
    let f = Fixture::new(&[], NEVER, |host| {
        host.microphone = Mutex::new(PermissionStatus::Denied);
        // A refusal read while a prompt may be open is not believed.
        host.system_audio_answer = Some(PermissionStatus::Denied);
    });
    assert_eq!(
        f.core.request_permission(PermissionKind::Microphone),
        "denied"
    );
    assert_eq!(*f.host.microphone_requests.lock().unwrap(), 0);
    assert_eq!(
        *f.host.settings_opened.lock().unwrap(),
        [PermissionKind::Microphone]
    );
    assert_eq!(
        f.core.request_permission(PermissionKind::SystemAudio),
        "undetermined"
    );
    assert_eq!(f.core.permissions().system_audio, "undetermined");

    // A positive answer for system audio is remembered for this run.
    let f = Fixture::new(&[], NEVER, |host| {
        host.system_audio_answer = Some(PermissionStatus::Granted);
    });
    assert_eq!(
        f.core.request_permission(PermissionKind::SystemAudio),
        "granted"
    );
    assert_eq!(f.core.permissions().system_audio, "granted");

    // A microphone grant made in System Settings shows at the next read.
    let f = Fixture::new(&[], NEVER, |host| {
        host.microphone = Mutex::new(PermissionStatus::Denied);
    });
    assert_eq!(f.core.permissions().microphone, "denied");
    *f.host.microphone.lock().unwrap() = PermissionStatus::Granted;
    assert_eq!(f.core.permissions().microphone, "granted");
}

#[test]
fn a_refused_system_source_records_the_microphone_only_and_reads_denied_from_then_on() {
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.system_audio_refused = AtomicBool::new(true);
    });
    assert_eq!(f.core.permissions().system_audio, "undetermined");

    // The meeting starts all the same, with the microphone alone.
    let id = f.core.start_meeting().unwrap().summary.id;
    f.wait_for_lines(&id, 1);
    let stopped = f.core.stop_meeting().unwrap();
    assert_eq!(stopped.summary.status, "complete");
    let transcript = f.core.get_transcript(&id).unwrap();
    assert!(transcript.iter().all(|line| line.speaker == "Me"));
    assert_eq!(f.core.permissions().system_audio, "denied");

    // "Open System Settings" is a request on a denied permission: the pane
    // opens and the answer is `denied`, the same as the next read.
    assert_eq!(
        f.core.request_permission(PermissionKind::SystemAudio),
        "denied"
    );
    assert_eq!(
        *f.host.settings_opened.lock().unwrap(),
        [PermissionKind::SystemAudio]
    );
    assert_eq!(f.core.permissions().system_audio, "denied");

    // Known to be denied: the next meeting is not refused, and the system
    // source is tried again, so access given meanwhile is noticed.
    f.host.system_audio_refused.store(false, Ordering::SeqCst);
    let id = f.core.start_meeting().unwrap().summary.id;
    f.wait_for_lines(&id, 2);
    f.core.stop_meeting().unwrap();
    assert_eq!(f.core.permissions().system_audio, "granted");
}

#[test]
fn a_meeting_that_loses_every_source_does_not_stay_recording() {
    // Short tapes: both sources end by themselves, with a tick to see it.
    let f = Fixture::new(&["small"], Duration::from_millis(50), |host| {
        host.system_audio_refused = AtomicBool::new(true);
        host.tape_ms = 400;
    });
    let id = f.core.start_meeting().unwrap().summary.id;
    f.wait_for("the recording to end by itself", || {
        f.core.recording_state().status == "idle"
    });
    // The window was told: the last states are stopping, then idle.
    let statuses = f.host.statuses();
    assert_eq!(statuses[statuses.len() - 2..], ["stopping", "idle"]);
    assert_eq!(f.core.get_meeting(&id).unwrap().summary.status, "complete");
    assert_eq!(f.core.permissions().system_audio, "denied");
}

#[test]
fn stop_gives_up_on_a_transcriber_call_that_never_returns_and_closes_the_meeting() {
    // Me's chunk is transcribed; the call for Them's never returns.
    let hold = Hold {
        from_call: 2,
        released: Arc::default(),
    };
    let f = Fixture::new(&["small"], NEVER, |host| host.hold = Some(hold.clone()));
    let id = f.core.start_meeting().unwrap().summary.id;
    f.wait_for_lines(&id, 1);
    f.wait_for("the second call to be stuck", || {
        f.host.languages.lock().unwrap().len() == 2
    });

    let asked = Instant::now();
    let stopped = f.core.stop_meeting().unwrap();
    // The limit under test is 300 ms; nothing else here takes seconds.
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(stopped.summary.status, "complete");
    assert_eq!(stopped.transcript_line_count, 1);
    let transcript = f.core.get_transcript(&id).unwrap();
    assert_eq!(transcript.len(), 1);
    assert_eq!(transcript[0].speaker, "Me");
    assert_eq!(f.core.recording_state(), dto::RecordingState::idle());
    assert_eq!(
        *f.host.on_disk_at_idle.lock().unwrap(),
        [vec![MeetingStatus::Complete]]
    );

    // Only the thread stuck in the call still holds the engine: the warm
    // cache let go of it, so the next meeting cannot get it. It goes once
    // the call returns.
    assert_eq!(f.warm_id(), None);
    assert!(f.host.engine_is_alive());
    hold.release();
    f.wait_for("the stuck thread to drop the engine", || {
        !f.host.engine_is_alive()
    });
    // It wrote nothing more.
    assert_eq!(f.core.get_transcript(&id).unwrap(), transcript);

    // A new meeting starts as usual, with a fresh engine.
    let loads = f.host.loaded.lock().unwrap().len();
    f.core.start_meeting().unwrap();
    assert_eq!(f.host.loaded.lock().unwrap().len(), loads + 1);
    f.core.stop_meeting().unwrap();
}

// ---- Warm model (030) ----

#[test]
fn the_model_warmed_at_launch_serves_every_meeting_without_a_reload() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    f.core.warm_up();
    f.wait_for_warm("small");

    for _ in 0..2 {
        f.core.start_meeting().unwrap();
        assert_eq!(f.core.recording_state().model_id.as_deref(), Some("small"));
        f.core.stop_meeting().unwrap();
        assert!(f.host.engine_is_alive());
    }
    assert_eq!(f.loaded(), ["ggml-small.bin"]);
}

#[test]
fn a_new_default_model_replaces_the_warm_one() {
    // No default: `small` is first in the catalog, so it is the pick.
    let f = Fixture::new(&["small", "large"], NEVER, |_| {});
    f.core.warm_up();
    f.wait_for_warm("small");
    let small = f.host.engine.lock().unwrap().clone();

    let patch = SettingsPatch {
        default_model_id: Patch::Set("large".to_owned()),
        ..SettingsPatch::default()
    };
    f.core.update_settings(&patch).unwrap();
    f.wait_for_warm("large");
    // One engine at a time: the old one is gone.
    assert!(small.upgrade().is_none());

    f.core.start_meeting().unwrap();
    assert_eq!(f.core.recording_state().model_id.as_deref(), Some("large"));
    f.core.stop_meeting().unwrap();
    assert_eq!(f.loaded(), ["ggml-small.bin", "ggml-large.bin"]);
}

#[test]
fn deleting_the_warm_model_frees_it_and_warms_the_next_pick() {
    let f = Fixture::new(&["small", "large"], NEVER, |_| {});
    let patch = SettingsPatch {
        default_model_id: Patch::Set("large".to_owned()),
        ..SettingsPatch::default()
    };
    f.core.update_settings(&patch).unwrap();
    f.wait_for_warm("large");
    let large = f.host.engine.lock().unwrap().clone();

    f.core.delete_model("large").unwrap();
    assert!(large.upgrade().is_none(), "freed before delete returns");
    assert!(!f.core.list_models()[1].installed);
    assert_eq!(f.core.get_settings().default_model_id, None);
    f.wait_for_warm("small");
    assert_eq!(f.loaded(), ["ggml-large.bin", "ggml-small.bin"]);

    // The last model too: nothing is left to warm.
    f.core.delete_model("small").unwrap();
    assert_eq!(f.warm_id(), None);
    assert!(!f.host.engine_is_alive());
}

#[test]
fn a_start_during_the_warm_up_waits_for_it_and_the_model_loads_once() {
    let (open_gate, gate): (Sender<()>, Receiver<()>) = channel();
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.load_gate = Mutex::new(Some(gate));
    });
    f.core.warm_up();
    f.wait_for("the warm-up to begin loading", || f.loaded().len() == 1);

    let core = Arc::clone(&f.core);
    let start = thread::spawn(move || core.start_meeting());
    f.wait_for("the start to begin", || f.core.is_starting());
    // The start waits without holding the recording lock, and says so.
    let state = f.core.recording_state();
    assert_eq!((state.status, state.starting), ("idle", true));
    assert_eq!(f.core.start_meeting().unwrap_err(), ALREADY_RECORDING);

    open_gate.send(()).unwrap();
    let meeting = start.join().unwrap().unwrap();
    assert_eq!(meeting.summary.status, "recording");
    assert_eq!(f.loaded(), ["ggml-small.bin"]);
    f.core.stop_meeting().unwrap();
}

#[test]
fn shutdown_frees_the_warm_engine() {
    let f = Fixture::new(&["small"], NEVER, |_| {});
    f.core.warm_up();
    f.wait_for_warm("small");
    assert!(f.host.engine_is_alive());

    f.core.shutdown();
    assert!(!f.host.engine_is_alive());
    assert_eq!(f.warm_id(), None);
    // Nothing warms up once the app is quitting.
    f.core.warm_up();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(f.loaded(), ["ggml-small.bin"]);
}

#[test]
fn shutdown_during_a_warm_up_waits_for_it_and_drops_what_it_loaded() {
    let (open_gate, gate): (Sender<()>, Receiver<()>) = channel();
    let f = Fixture::new(&["small"], NEVER, |host| {
        host.load_gate = Mutex::new(Some(gate));
    });
    f.core.warm_up();
    f.wait_for("the warm-up to begin loading", || f.loaded().len() == 1);

    let core = Arc::clone(&f.core);
    let shutdown = thread::spawn(move || core.shutdown());
    f.wait_for("shutdown to begin", || {
        f.core.quitting.load(Ordering::SeqCst)
    });
    thread::sleep(Duration::from_millis(50));
    assert!(!shutdown.is_finished(), "shutdown waits for the load");

    open_gate.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(!f.host.engine_is_alive());
    assert_eq!(f.warm_id(), None);
}
