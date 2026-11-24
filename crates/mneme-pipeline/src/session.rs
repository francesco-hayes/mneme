//! [`Session`]: the handle to one running meeting.

use std::error::Error;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mneme_audio::{AudioSource, Recorder};
use mneme_core::Speaker;
use mneme_transcribe::Transcriber;

use crate::capture::{self, AudioKeeper, CaptureParts};
use crate::config::{SessionConfig, keep_audio_enabled};
use crate::shared::{BoxedSink, Hooks, Shared, SinkSlot, lock};
use crate::status::{SessionStatus, SessionSummary};
use crate::{TranscriptSink, worker};

/// What the capture thread waits for before it does anything.
type CaptureStart = (Arc<Shared>, CaptureParts, JoinHandle<()>);

/// Why [`Session::start`] did not start a session. When it returns one of
/// these, no source was started, no file was created and nothing is left
/// running; the sources, the sink and the session's reference to the
/// transcriber have been dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// The list of sources was empty.
    NoSources,
    /// Two sources carry the same speaker label. Each speaker has one
    /// chunker, which takes one stream.
    DuplicateSpeaker(Speaker),
    /// A thread of the session could not be created.
    Thread(String),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::NoSources => f.write_str("a session needs at least one audio source"),
            StartError::DuplicateSpeaker(speaker) => {
                write!(f, "two audio sources are labeled `{speaker}`")
            }
            StartError::Thread(message) => {
                write!(f, "could not start a thread for the session: {message}")
            }
        }
    }
}

impl Error for StartError {}

/// One meeting being recorded and transcribed.
///
/// [`start`](Self::start) returns at once with the session running;
/// [`status`](Self::status) can be read at any time from any thread;
/// [`stop`](Self::stop) ends the recording and returns when the transcript
/// is complete. The crate documentation has the whole picture.
///
/// The handle is `Send + Sync`: keep it in shared state and call any method
/// from any thread. The one thing not to do is call a method that waits
/// (`stop`, `wait`) from inside the transcriber or the sink, which run on
/// the thread being waited for.
///
/// Dropping a session stops it as [`stop`](Self::stop) does, and waits.
pub struct Session {
    shared: Arc<Shared>,
    /// The capture thread, until somebody has waited for it.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Session {
    /// Starts recording `sources` and transcribing them into `sink`.
    ///
    /// - `sources`: one or two sources with different speaker labels, the
    ///   microphone first (it is started first; see
    ///   [`Recorder`](mneme_audio::Recorder)). They are started in the
    ///   background on one shared clock. A source that fails to start,
    ///   starts late or dies does not stop the other: look at
    ///   [`SessionStatus::sources`].
    /// - `transcriber`: a loaded model. The session holds this reference
    ///   only until it is done: once [`stop`](Self::stop) has returned (or
    ///   the session is dropped) the caller's is the last one.
    /// - `sink`: where the lines go. It is moved to the transcription
    ///   thread and dropped there before `stop` returns.
    ///
    /// Returns at once; no device is open yet and no permission prompt has
    /// been answered when it does. If audio is kept
    /// ([`SessionConfig::audio_dir`]), the files are created before any
    /// source starts.
    pub fn start(
        sources: Vec<Box<dyn AudioSource>>,
        transcriber: Arc<dyn Transcriber>,
        sink: impl TranscriptSink + 'static,
        config: SessionConfig,
    ) -> Result<Session, StartError> {
        Self::start_with_hooks(
            sources,
            transcriber,
            Box::new(sink),
            config,
            Hooks::default(),
        )
    }

    pub(crate) fn start_with_hooks(
        sources: Vec<Box<dyn AudioSource>>,
        transcriber: Arc<dyn Transcriber>,
        sink: BoxedSink,
        config: SessionConfig,
        hooks: Hooks,
    ) -> Result<Session, StartError> {
        if sources.is_empty() {
            return Err(StartError::NoSources);
        }
        for (n, source) in sources.iter().enumerate() {
            let speaker = source.speaker();
            if sources[..n].iter().any(|other| other.speaker() == speaker) {
                return Err(StartError::DuplicateSpeaker(speaker));
            }
        }

        // Both threads exist before anything else happens, each waiting for
        // its work. If one cannot be created, the other finds its channel
        // closed and ends: no source was started and no file was created.
        let (to_worker, worker_start) = mpsc::channel::<Arc<Shared>>();
        let sink: SinkSlot = Arc::new(Mutex::new(Some(sink)));
        let worker_sink = Arc::clone(&sink);
        let worker = thread::Builder::new()
            .name("mneme-transcribe".to_owned())
            .spawn(move || {
                // The transcriber lives and dies with this closure, whether
                // or not it gets to run the session; so does the sink,
                // unless the session gives up on this thread.
                if let Ok(shared) = worker_start.recv() {
                    let _exit = WorkerExit(Arc::clone(&shared));
                    worker::run(shared, transcriber, worker_sink, hooks);
                }
            })
            .map_err(|e| StartError::Thread(e.to_string()))?;
        let (to_capture, capture_start) = mpsc::channel::<CaptureStart>();
        let capture = thread::Builder::new()
            .name("mneme-session".to_owned())
            .spawn(move || {
                if let Ok((shared, parts, worker)) = capture_start.recv() {
                    capture::run(shared, parts, worker);
                }
            });
        let capture = match capture {
            Ok(capture) => capture,
            Err(e) => {
                // Wait for the other thread to be gone, so the transcriber
                // and the sink are dropped before this returns.
                drop(to_worker);
                let _ = worker.join();
                return Err(StartError::Thread(e.to_string()));
            }
        };

        // Only with the development switch, and before any source starts.
        let audio = match config.audio_dir {
            Some(dir) if keep_audio_enabled() => Some(AudioKeeper::create(dir)),
            _ => None,
        };

        // Returns at once: the sources come up on threads of their own.
        let (recorder, frames) = Recorder::start_with(sources, config.recorder);
        let shared = Arc::new(Shared::new(
            recorder.started_at(),
            config.language,
            config.max_backlog,
            recorder.status(),
        ));
        if let Some(audio) = &audio {
            let mut inner = lock(&shared.inner);
            inner.audio_dir = Some(audio.dir().to_owned());
            inner.audio_error = audio.error().map(str::to_owned);
        }
        let parts = CaptureParts {
            recorder,
            frames,
            chunker: config.chunker,
            audio,
            sink,
            hooks,
        };

        // Neither send can fail while its thread is waiting, and nothing
        // else can have ended those threads. If one did fail, what was sent
        // comes back in the error and is dropped here, which stops the
        // sources and ends the other thread.
        to_worker
            .send(Arc::clone(&shared))
            .map_err(|_| StartError::Thread("the transcription thread is gone".to_owned()))?;
        to_capture
            .send((Arc::clone(&shared), parts, worker))
            .map_err(|_| StartError::Thread("the capture thread is gone".to_owned()))?;

        Ok(Session {
            shared,
            thread: Mutex::new(Some(capture)),
        })
    }

    /// A snapshot of the session: state, meeting time, lines written,
    /// backlog, each source's phase and errors.
    ///
    /// Cheap, and never waits for a source, the transcriber or the sink,
    /// whatever they are doing. Safe to call once a second or much more
    /// often.
    pub fn status(&self) -> SessionStatus {
        self.shared.status()
    }

    /// Meeting time now, on the clock the frames and the transcript lines
    /// are stamped with. Stands still once capture has stopped.
    pub fn elapsed(&self) -> Duration {
        let stopped_at = lock(&self.shared.inner).final_duration;
        stopped_at.unwrap_or_else(|| self.shared.started_at.elapsed())
    }

    /// The instant meeting time is measured from.
    pub fn started_at(&self) -> Instant {
        self.shared.started_at
    }

    /// `true` once the summary is ready and [`stop`](Self::stop) would
    /// return at once.
    pub fn is_done(&self) -> bool {
        lock(&self.shared.inner).summary.is_some()
    }

    /// Asks the session to stop and returns without waiting. The status
    /// reads [`Stopping`](crate::SessionState::Stopping) from this moment.
    /// Calling it again, or after the session has ended, does nothing.
    pub fn request_stop(&self) {
        self.shared.begin_stopping();
        self.shared.stop.store(true, Ordering::SeqCst);
        // If the backlog was abandoned beforehand, this is when it goes.
        self.shared.drop_abandoned();
    }

    /// Gives up on the audio that is still waiting for the transcriber when
    /// the session stops. Returns without waiting.
    ///
    /// For quitting the app when the user will not wait for a long backlog.
    /// It takes effect once the session is stopping (call it before or
    /// after `stop`, from any thread; while the meeting is still recording
    /// it changes nothing yet): every chunk still waiting is thrown away,
    /// the chunk the transcriber is working on is finished and its lines
    /// are written, the sink is finalized, and `stop` returns. What is
    /// lost is exactly the speech in those chunks; each is listed in
    /// [`SessionSummary::dropped`] with its speaker and time. Lines already
    /// written are never touched.
    ///
    /// A transcriber call cannot be interrupted, so `stop` can still take
    /// as long as one chunk takes.
    pub fn abandon_backlog(&self) {
        self.shared.abandon.store(true, Ordering::SeqCst);
        self.shared.drop_abandoned();
    }

    /// Gives up on the transcriber: the session stops waiting for it and
    /// ends now. Returns without waiting.
    ///
    /// For a transcriber call that never returns, which would otherwise make
    /// [`stop`](Self::stop) wait forever; [`stop_or_give_up`](Self::stop_or_give_up)
    /// calls this when a call runs too long. It stops the session and
    /// abandons the backlog as [`abandon_backlog`](Self::abandon_backlog)
    /// does, and also stops waiting for the chunk in work: that chunk is
    /// listed in [`SessionSummary::dropped`] as
    /// [`TranscriberStuck`](crate::DropReason::TranscriberStuck), the sink
    /// is finalized on the session's capture thread, and `stop` returns.
    /// Lines already written are kept.
    ///
    /// A call into the transcriber cannot be interrupted, so the
    /// transcription thread stays inside it, holding its reference to the
    /// transcriber. If the call ever returns, the thread writes nothing,
    /// lets go of the transcriber and ends; until then the transcriber (a
    /// loaded model) stays alive, even after the caller has dropped its own
    /// reference. A `WhisperEngine` still alive when the process exits makes
    /// whisper.cpp abort the process (issue 005).
    ///
    /// If the transcription thread is writing lines or finalizing when this
    /// is called, the session lets it finish that first. Calling it again,
    /// or after the session has ended, does nothing.
    pub fn give_up_transcriber(&self) {
        self.request_stop();
        self.abandon_backlog();
        self.shared.begin_giving_up();
    }

    /// As [`stop`](Self::stop), but gives up on the transcriber
    /// ([`give_up_transcriber`](Self::give_up_transcriber)) if a single call
    /// into it runs for `call_limit` or longer.
    ///
    /// The limit is per call, not for the whole drain: a long backlog that
    /// the transcriber works through, however slowly, is transcribed to the
    /// end. A call that fails is retried once, and each try has its own
    /// limit.
    pub fn stop_or_give_up(&self, call_limit: Duration) -> SessionSummary {
        self.request_stop();
        // Often enough that a stuck call is given up soon after the limit.
        let poll = (call_limit / 10).clamp(Duration::from_millis(1), Duration::from_secs(1));
        loop {
            if let Some(summary) = self.wait_timeout(poll) {
                return summary;
            }
            if self
                .shared
                .call_running_for()
                .is_some_and(|running| running >= call_limit)
            {
                self.give_up_transcriber();
                return self.wait();
            }
        }
    }

    /// Stops the session and returns when the transcript is complete.
    ///
    /// In order: the sources are stopped (a source still stuck in its start
    /// is not waited for), the frames they delivered are read to the end,
    /// both chunkers are flushed so the last words are kept, everything
    /// queued is transcribed and written, the sink is finalized, and the
    /// transcriber and the sink are let go of.
    ///
    /// This takes as long as the backlog needs; [`status`](Self::status)
    /// shows the progress from another thread
    /// ([`SessionStatus::drain`]), and [`abandon_backlog`](Self::abandon_backlog)
    /// cuts it short.
    ///
    /// Callable from any thread, any number of times, also at once: every
    /// call returns the same summary, and a call after the session has
    /// ended returns it immediately.
    pub fn stop(&self) -> SessionSummary {
        self.request_stop();
        self.wait()
    }

    /// Waits until the session is done, without asking it to stop, and
    /// returns the summary. A session ends without being asked when every
    /// source has ended ([`EndReason::SourcesEnded`](crate::EndReason)).
    pub fn wait(&self) -> SessionSummary {
        let mut inner = lock(&self.shared.inner);
        let summary = loop {
            if let Some(summary) = &inner.summary {
                break summary.clone();
            }
            inner = self
                .shared
                .done
                .wait(inner)
                .unwrap_or_else(PoisonError::into_inner);
        };
        drop(inner);
        self.join_thread();
        summary
    }

    /// As [`wait`](Self::wait), but gives up after `timeout` and returns
    /// `None`. The session carries on either way.
    pub fn wait_timeout(&self, timeout: Duration) -> Option<SessionSummary> {
        let deadline = Instant::now() + timeout;
        let mut inner = lock(&self.shared.inner);
        let summary = loop {
            if let Some(summary) = &inner.summary {
                break summary.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            inner = self
                .shared
                .done
                .wait_timeout(inner, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        drop(inner);
        self.join_thread();
        Some(summary)
    }

    /// Waits for the capture thread to be gone. It publishes the summary as
    /// its last act, so this is a matter of microseconds.
    fn join_thread(&self) {
        let thread = lock(&self.thread).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// Tells the capture thread that the transcription thread has ended, also
/// when it panics.
struct WorkerExit(Arc<Shared>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.0.worker_exited();
    }
}

impl Drop for Session {
    /// Stops the session and waits for it, exactly as [`Session::stop`]
    /// does, so that no thread, device or reference to the transcriber
    /// outlives the handle. Call [`Session::abandon_backlog`] first if the
    /// wait must be short.
    fn drop(&mut self) {
        self.stop();
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status = self.status();
        f.debug_struct("Session")
            .field("state", &status.state)
            .field("elapsed", &status.elapsed)
            .field("lines", &status.lines)
            .field("backlog", &status.backlog)
            .finish_non_exhaustive()
    }
}
