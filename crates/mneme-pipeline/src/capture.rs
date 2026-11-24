//! The capture thread: owns the recorder, reads its frames, feeds the two
//! chunkers, queues their chunks, and at the end sees the session through to
//! its summary.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mneme_audio::{
    FrameReceiver, MIC_WAV, Recorder, RecorderReport, RecordingWriter, SYSTEM_WAV, SourcePhase,
};
use mneme_core::{AudioFrame, Speaker};
use mneme_transcribe::{Chunker, ChunkerConfig};

use crate::shared::{DrainStart, Hooks, Shared, SinkSlot, lock, panic_message};
use crate::status::{EndReason, KeptAudio, SessionState};

/// How long the thread waits for a frame before it looks at the stop flag,
/// and how often it looks at the sources.
const TICK: Duration = Duration::from_millis(20);

/// How often the kept audio's WAV headers are brought up to date, so that a
/// process that is killed leaves playable files.
const AUDIO_FLUSH_EVERY: Duration = Duration::from_secs(1);

/// What the capture thread is handed at start.
pub(crate) struct CaptureParts {
    pub(crate) recorder: Recorder,
    pub(crate) frames: FrameReceiver,
    pub(crate) chunker: ChunkerConfig,
    pub(crate) audio: Option<AudioKeeper>,
    /// The sink, to finalize here if the session gives up on the
    /// transcription thread.
    pub(crate) sink: SinkSlot,
    pub(crate) hooks: Hooks,
}

/// The body of the capture thread.
///
/// Whatever happens in capture, the queue is closed, the transcription
/// thread is waited for and a summary is published, so `stop()` always
/// returns.
pub(crate) fn run(shared: Arc<Shared>, parts: CaptureParts, worker: JoinHandle<()>) {
    // If anything below unwinds, the summary is still published.
    let _publish = PublishOnUnwind(&shared);
    let sink = Arc::clone(&parts.sink);

    // A panic in here drops the recorder, which stops the sources.
    let captured = catch_unwind(AssertUnwindSafe(|| capture(&shared, parts, &worker)));

    {
        let mut inner = lock(&shared.inner);
        let mut queue = lock(&shared.queue);
        match captured {
            Ok(captured) => {
                inner.end = Some(captured.end);
                inner.final_duration = Some(captured.report.duration);
                inner.sources = captured.report.sources;
                inner.audio = captured.audio;
            }
            Err(panic) => {
                inner.end = Some(EndReason::CaptureFailed);
                inner.final_duration = Some(shared.started_at.elapsed());
                inner.internal_errors.push(format!(
                    "the capture thread panicked: {}",
                    panic_message(panic.as_ref())
                ));
                // The files exist but were never finished: they hold what
                // was flushed last.
                inner.audio = inner.audio_dir.clone().map(|dir| KeptAudio {
                    files: vec![dir.join(MIC_WAV), dir.join(SYSTEM_WAV)],
                    dir,
                    error: Some(
                        "the capture thread panicked before the files were finished".to_owned(),
                    ),
                });
            }
        }
        inner.state = SessionState::Stopping;
        let now = Instant::now();
        inner.stopping_since.get_or_insert(now);
        inner.drain = Some(DrainStart {
            at: now,
            audio_ms: queue.waiting_ms() + queue.in_work_ms(),
            chunks: queue.len(),
        });
        // From here the transcription thread ends once the queue is empty.
        queue.close();
        shared.queue_changed.notify_all();
    }
    // The sources ending by themselves also makes an abandoned backlog go.
    shared.drop_abandoned();

    if wait_for_worker(&shared, &sink) {
        if let Err(panic) = worker.join() {
            let message = format!(
                "the transcription thread panicked: {}",
                panic_message(panic.as_ref())
            );
            shared.worker_died(&message);
            lock(&shared.inner).internal_errors.push(message);
        }
    } else {
        // Given up on: the thread is left to itself (dropping its handle
        // detaches it) and ends if its transcriber call ever returns.
        drop(worker);
    }
    shared.publish_summary();
}

/// Waits until the transcription thread ends or the session gives up on it.
/// `true` if it ended (or is about to: it has finalized the sink), `false`
/// if it was given up on, in which case its chunk in work is listed as
/// dropped and the sink has been finalized here.
fn wait_for_worker(shared: &Shared, sink: &SinkSlot) -> bool {
    let mut inner = lock(&shared.inner);
    while !inner.worker_exited && !shared.giving_up() {
        inner = shared
            .worker_changed
            .wait(inner)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    if inner.worker_exited {
        return true;
    }
    drop(inner);
    !shared.detach_worker(sink)
}

/// Publishes the summary if the capture thread unwinds past its own
/// handling, so that nobody waits for it forever.
struct PublishOnUnwind<'a>(&'a Shared);

impl Drop for PublishOnUnwind<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            {
                let mut inner = lock(&self.0.inner);
                inner
                    .internal_errors
                    .push("the capture thread failed while finishing the session".to_owned());
            }
            lock(&self.0.queue).close();
            self.0.queue_changed.notify_all();
            self.0.publish_summary();
        }
    }
}

struct Captured {
    end: EndReason,
    report: RecorderReport,
    audio: Option<KeptAudio>,
}

/// Runs the recording: frames in, chunks out, until a stop request or until
/// every source has ended. Then stops the sources and flushes the chunkers.
fn capture(shared: &Shared, parts: CaptureParts, worker: &JoinHandle<()>) -> Captured {
    let CaptureParts {
        recorder,
        frames,
        chunker,
        audio,
        sink: _,
        hooks,
    } = parts;
    let mut state = CaptureState {
        shared,
        me: Chunker::new(Speaker::Me, chunker.clone()),
        them: Chunker::new(Speaker::Them, chunker),
        audio,
        live: vec![true; recorder.status().len()],
        frames_seen: 0,
        hooks,
    };

    let mut last_look: Option<Instant> = None;
    let end = loop {
        match frames.recv_timeout(TICK) {
            Ok(frame) => state.on_frame(frame),
            Err(RecvTimeoutError::Timeout) => {}
            // Every source has let go of its sender: nothing more can come.
            Err(RecvTimeoutError::Disconnected) => {
                break if shared.stop_requested() {
                    EndReason::StopRequested
                } else {
                    EndReason::SourcesEnded
                };
            }
        }
        if shared.stop_requested() {
            break EndReason::StopRequested;
        }
        if last_look.is_none_or(|at| at.elapsed() >= TICK) {
            last_look = Some(Instant::now());
            state.look_at_sources(&recorder, &frames, worker);
        }
    };

    shared.begin_stopping();
    // Stopping a device is the one step that could take long: leave valid
    // files behind first.
    if let Some(audio) = &mut state.audio {
        audio.flush();
    }
    // Returns once every running source has sent its last frame. A source
    // still stuck in `start` is not waited for.
    let report = recorder.stop();
    // Everything captured up to the stop is in the channel, and it ends.
    for frame in frames.iter() {
        state.on_frame(frame);
    }
    // The last words of both speakers.
    state.flush(Speaker::Me);
    state.flush(Speaker::Them);

    let at_least = match end {
        EndReason::StopRequested => report.duration,
        // The sources ended by themselves: the files end where the audio does.
        EndReason::SourcesEnded | EndReason::CaptureFailed => Duration::ZERO,
    };
    let audio = state.audio.take().map(|audio| audio.finish(at_least));
    Captured { end, report, audio }
}

struct CaptureState<'a> {
    shared: &'a Shared,
    me: Chunker,
    them: Chunker,
    audio: Option<AudioKeeper>,
    /// Per source, in the recorder's order: was it starting or running when
    /// last looked at?
    live: Vec<bool>,
    frames_seen: u64,
    hooks: Hooks,
}

impl CaptureState<'_> {
    fn on_frame(&mut self, frame: AudioFrame) {
        self.frames_seen += 1;
        if self.hooks.capture_panics_at_frame == Some(self.frames_seen) {
            panic!("test hook: the capture thread panics");
        }
        // From frames, not chunks: the chunkers drop the silence.
        if let Some(audio) = &mut self.audio
            && let Some(error) = audio.write(&frame)
        {
            lock(&self.shared.inner).audio_error = Some(error);
        }
        let chunker = match frame.speaker {
            Speaker::Me => &mut self.me,
            Speaker::Them => &mut self.them,
        };
        for chunk in chunker.push(&frame) {
            self.shared.enqueue(chunk);
        }
    }

    /// Ends a speaker's stream: the chunk in progress, if it holds speech,
    /// goes to the queue.
    fn flush(&mut self, speaker: Speaker) {
        let chunker = match speaker {
            Speaker::Me => &mut self.me,
            Speaker::Them => &mut self.them,
        };
        if let Some(chunk) = chunker.flush() {
            self.shared.enqueue(chunk);
        }
    }

    /// Copies the sources' state into the status, and flushes the chunker of
    /// a source that has just ended or failed, so its last words are kept
    /// now and not only when the meeting stops.
    fn look_at_sources(
        &mut self,
        recorder: &Recorder,
        frames: &FrameReceiver,
        worker: &JoinHandle<()>,
    ) {
        let sources = recorder.status();
        let mut ended = Vec::new();
        for (live, source) in self.live.iter_mut().zip(&sources) {
            if *live && !source.phase.is_live() {
                *live = false;
                ended.push(source.speaker);
            }
        }
        if !ended.is_empty() {
            // A source reads as ended only once its last frame is in the
            // stream, so everything it delivered is in the channel now.
            while let Ok(frame) = frames.try_recv() {
                self.on_frame(frame);
            }
            for speaker in ended {
                self.flush(speaker);
            }
        }

        {
            let mut inner = lock(&self.shared.inner);
            let any_started = sources.iter().any(|source| {
                matches!(
                    source.phase,
                    SourcePhase::Running | SourcePhase::Ended | SourcePhase::Stopped
                )
            });
            if inner.state == SessionState::Starting && any_started {
                inner.state = SessionState::Recording;
            }
            inner.sources = sources;
        }

        // The transcription thread only returns once the queue is closed,
        // which happens after capture. If it is gone now, it died.
        if worker.is_finished() {
            self.shared
                .worker_died("the transcription thread died during the recording");
        }
    }
}

/// Writes `mic.wav` and `system.wav` under `MNEME_KEEP_AUDIO=1`. A failure
/// stops the writing and is reported; it never touches the session.
pub(crate) struct AudioKeeper {
    dir: PathBuf,
    writer: Option<RecordingWriter>,
    created: bool,
    error: Option<String>,
    last_flush: Instant,
}

impl AudioKeeper {
    /// Creates the folder if needed and both files in it.
    pub(crate) fn create(dir: PathBuf) -> Self {
        let (writer, error) = match RecordingWriter::create(&dir) {
            Ok(writer) => (Some(writer), None),
            Err(error) => (None, Some(error.to_string())),
        };
        AudioKeeper {
            dir,
            created: writer.is_some(),
            writer,
            error,
            last_flush: Instant::now(),
        }
    }

    /// The folder the files are in.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The reason the files could not be created, if they could not.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Writes a frame at its place in meeting time. Returns the error if
    /// this write is the one that failed.
    fn write(&mut self, frame: &AudioFrame) -> Option<String> {
        let writer = self.writer.as_mut()?;
        let mut result = writer.write(frame);
        if result.is_ok() && self.last_flush.elapsed() >= AUDIO_FLUSH_EVERY {
            self.last_flush = Instant::now();
            result = writer.flush();
        }
        let error = result.err()?.to_string();
        self.fail(error.clone());
        Some(error)
    }

    fn flush(&mut self) {
        if let Some(writer) = self.writer.as_mut()
            && let Err(error) = writer.flush()
        {
            self.fail(error.to_string());
        }
    }

    /// Stops writing. The files keep what was flushed last.
    fn fail(&mut self, error: String) {
        self.error = Some(error);
        self.writer = None;
    }

    /// Pads both files to one length (at least `at_least`) and finishes
    /// them.
    fn finish(mut self, at_least: Duration) -> KeptAudio {
        if let Some(writer) = self.writer.take()
            && let Err(error) = writer.finish(at_least)
        {
            self.error = Some(error.to_string());
        }
        let files = if self.created {
            vec![self.dir.join(MIC_WAV), self.dir.join(SYSTEM_WAV)]
        } else {
            Vec::new()
        };
        KeptAudio {
            dir: self.dir,
            files,
            error: self.error,
        }
    }
}
