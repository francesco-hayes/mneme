//! [`Recorder`]: several sources run as one recording, on one clock, into one
//! stream of frames.
//!
//! This is the part every consumer of two channels needs and should not
//! write twice: the `mneme-record` tool uses it to write WAV files, and the
//! live session (issue 009) uses it to feed its chunkers.
//!
//! # What it takes care of
//!
//! - **One clock.** The start instant is read once, before anything is
//!   started, and every source is given that same instant. A source that
//!   comes up late stamps its first frame with the time it was really
//!   captured, so the channels line up however long each `start` took.
//! - **A start that blocks.** Each source is started on a thread of its own.
//!   [`SystemAudioSource::start`](crate::SystemAudioSource) can wait for as
//!   long as a macOS permission prompt is open; meanwhile the microphone is
//!   already delivering, and [`Recorder::start`] itself returns at once.
//! - **Start order.** Sources are started in the order given, each with a
//!   head start ([`RecorderOptions::head_start`]) before the next one is
//!   started alongside it. Put the microphone first: one report (not
//!   verified here) says AirPods only switch to their call profile, and so
//!   only deliver microphone audio, when the microphone is opened before the
//!   system audio tap.
//! - **Failures.** A source that fails to start, panics or dies mid-run
//!   never takes the others with it. What happened to each source is in
//!   [`Recorder::status`] while recording and in the [`RecorderReport`] that
//!   [`Recorder::stop`] returns.
//! - **The end of the stream.** The frame receiver ends (its iterator
//!   returns `None`) once every source has ended, and always after
//!   [`Recorder::stop`], even if a source was still stuck starting.
//!
//! # How frames travel
//!
//! Each source sends to a channel of its own, and its supervising thread
//! passes the frames on to the one receiver the caller holds. That hop is
//! what makes the points above possible: a source stuck in `start` holds a
//! sender the recorder can afford to wait on forever, the supervisor sees
//! the moment a source drops its sender (the one sure sign that it ended),
//! and frames are counted per source on the way through.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker};

use crate::{AudioError, AudioSource, FrameReceiver, FrameSender, frame_channel};

/// How often a supervising thread looks at the stop flag and collects its
/// source's errors while no frames arrive.
const POLL: Duration = Duration::from_millis(20);

/// Settings for [`Recorder::start_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderOptions {
    /// How long a source's `start` is given before the next source in the
    /// list is started alongside it. The next source starts as soon as the
    /// previous `start` has returned, or after this long if it has not.
    ///
    /// Zero starts every source at the same moment. The default is one
    /// second: long enough for a device to open, short enough that a
    /// permission prompt on one source does not hold up the other.
    pub head_start: Duration,
}

impl Default for RecorderOptions {
    fn default() -> Self {
        Self {
            head_start: Duration::from_secs(1),
        }
    }
}

/// Where a source is in its life inside a [`Recorder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePhase {
    /// `start` has not returned yet (or it is not this source's turn yet).
    /// A permission prompt may be open.
    Starting,
    /// `start` succeeded and the source has not ended.
    Running,
    /// `start` returned an error, or panicked. The source delivered nothing;
    /// the reason is the first entry of [`SourceStatus::errors`].
    FailedToStart,
    /// The source stopped delivering by itself: it failed for good, or (a
    /// fake) ran out of input. Any reason it gave is in
    /// [`SourceStatus::errors`].
    Ended,
    /// The source was running when [`Recorder::stop`] stopped it.
    Stopped,
    /// The recorder was stopped while this source was still starting. It
    /// contributed nothing; if its `start` ever returns it is stopped at
    /// once.
    Abandoned,
}

impl SourcePhase {
    /// `true` while the source may still deliver frames: starting or
    /// running.
    pub fn is_live(self) -> bool {
        matches!(self, SourcePhase::Starting | SourcePhase::Running)
    }

    /// `true` if the source is no longer delivering and nobody asked it to
    /// stop: it failed to start, ended by itself, or never got to start.
    ///
    /// For a live source each of those is a failure. A fake that has played
    /// all its input reads the same way; it is the only kind of source that
    /// ends by itself in good health.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            SourcePhase::FailedToStart | SourcePhase::Ended | SourcePhase::Abandoned
        )
    }
}

/// What a [`Recorder`] knows about one of its sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    /// The label on this source's frames.
    pub speaker: Speaker,
    /// Where the source is in its life.
    pub phase: SourcePhase,
    /// Every error so far, oldest first: the error `start` returned, then
    /// whatever the source queued (see [`AudioSource::take_errors`]). An
    /// error here does not mean the source ended; `phase` tells.
    pub errors: Vec<AudioError>,
    /// Meeting time, in milliseconds, at which `start` returned. `None`
    /// while starting, and for an abandoned source.
    pub started_ms: Option<u64>,
    /// Frames passed on to the stream.
    pub frames: u64,
    /// Samples in those frames.
    pub samples: u64,
    /// `start_ms` of the first frame.
    pub first_frame_ms: Option<u64>,
    /// Meeting time, in milliseconds, at which the latest frame ends.
    pub end_ms: Option<u64>,
}

impl SourceStatus {
    fn new(speaker: Speaker) -> Self {
        Self {
            speaker,
            phase: SourcePhase::Starting,
            errors: Vec::new(),
            started_ms: None,
            frames: 0,
            samples: 0,
            first_frame_ms: None,
            end_ms: None,
        }
    }

    /// Length of the audio this source delivered, gaps not counted.
    pub fn captured(&self) -> Duration {
        Duration::from_micros(self.samples * 1_000_000 / u64::from(SAMPLE_RATE))
    }
}

/// What [`Recorder::stop`] returns: how long the recording ran and the final
/// state of every source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderReport {
    /// Meeting time at which `stop` was called.
    pub duration: Duration,
    /// One entry per source, in the order they were given.
    pub sources: Vec<SourceStatus>,
}

impl RecorderReport {
    /// The first source labeled `speaker`.
    pub fn source(&self, speaker: Speaker) -> Option<&SourceStatus> {
        self.sources.iter().find(|s| s.speaker == speaker)
    }

    /// Every error of every source, with the speaker it belongs to.
    pub fn errors(&self) -> Vec<(Speaker, AudioError)> {
        self.sources
            .iter()
            .flat_map(|s| s.errors.iter().map(|e| (s.speaker, e.clone())))
            .collect()
    }
}

/// Runs a set of sources (in Mneme: the microphone and system audio) as one
/// recording. See the [module documentation](self).
///
/// ```
/// use mneme_audio::{AudioSource, FakeSource, Recorder, SourcePhase};
/// use mneme_core::Speaker;
///
/// let mic = FakeSource::from_samples(Speaker::Me, vec![0.1; 3_200]);
/// let system = FakeSource::from_samples(Speaker::Them, vec![0.2; 3_200]);
/// let sources: Vec<Box<dyn AudioSource>> = vec![Box::new(mic), Box::new(system)];
///
/// let (recorder, frames) = Recorder::start(sources);
/// // Fake sources end by themselves; live ones run until `stop`.
/// let frames: Vec<_> = frames.iter().collect();
/// assert_eq!(frames.len(), 20);
///
/// let report = recorder.stop();
/// assert!(report.errors().is_empty());
/// assert_eq!(report.sources[0].phase, SourcePhase::Ended);
/// ```
///
/// Dropping a recorder stops it.
pub struct Recorder {
    start: Instant,
    stop: Arc<AtomicBool>,
    sources: Vec<Supervised>,
    stopped: bool,
}

struct Supervised {
    slot: Arc<Slot>,
    thread: Option<JoinHandle<()>>,
}

/// What the recorder and one supervising thread share.
struct Slot(Mutex<SlotInner>);

struct SlotInner {
    status: SourceStatus,
    /// The sender to the caller's receiver, parked here until the source has
    /// started. Whoever ends the source's part in the recording takes it: the
    /// supervisor when it begins forwarding, or `stop` when it abandons a
    /// source that is still starting, so that the stream can end.
    out: Option<FrameSender>,
}

impl Slot {
    fn lock(&self) -> MutexGuard<'_, SlotInner> {
        // Plain data, valid even if a holder panicked.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Recorder {
    /// Starts `sources` with the default [`RecorderOptions`] and returns the
    /// recorder with the receiver of every source's frames.
    ///
    /// Returns at once: the sources are started in the background, in order.
    /// Whether each one came up is in [`status`](Self::status).
    pub fn start(sources: Vec<Box<dyn AudioSource>>) -> (Self, FrameReceiver) {
        Self::start_with(sources, RecorderOptions::default())
    }

    /// As [`start`](Self::start), with settings.
    pub fn start_with(
        sources: Vec<Box<dyn AudioSource>>,
        options: RecorderOptions,
    ) -> (Self, FrameReceiver) {
        let (out, frames) = frame_channel();
        let stop = Arc::new(AtomicBool::new(false));
        // The one instant every source measures from.
        let start = Instant::now();

        let mut supervised = Vec::with_capacity(sources.len());
        let mut turn: Option<mpsc::Receiver<()>> = None;
        for source in sources {
            let speaker = source.speaker();
            let slot = Arc::new(Slot(Mutex::new(SlotInner {
                status: SourceStatus::new(speaker),
                out: Some(out.clone()),
            })));
            let (started_tx, started_rx) = mpsc::channel();
            let supervisor = Supervisor {
                source,
                start,
                slot: Arc::clone(&slot),
                stop: Arc::clone(&stop),
            };
            let wait_for = turn.replace(started_rx);
            let head_start = options.head_start;
            let thread_slot = Arc::clone(&slot);
            let spawned = thread::Builder::new()
                .name(format!("mneme-recorder-{speaker}"))
                .spawn(move || {
                    let run = AssertUnwindSafe(|| supervisor.run(wait_for, started_tx, head_start));
                    if let Err(panic) = catch_unwind(run) {
                        fail_after_panic(&thread_slot, panic.as_ref());
                    }
                });
            let thread = match spawned {
                Ok(thread) => Some(thread),
                Err(err) => {
                    let mut inner = slot.lock();
                    inner.status.phase = SourcePhase::FailedToStart;
                    inner.status.errors.push(AudioError::Other(format!(
                        "could not spawn the thread for the {speaker} source: {err}"
                    )));
                    inner.out = None;
                    None
                }
            };
            supervised.push(Supervised { slot, thread });
        }
        // The recorder keeps no sender of its own: the stream ends when the
        // sources do.
        drop(out);

        let recorder = Self {
            start,
            stop,
            sources: supervised,
            stopped: false,
        };
        (recorder, frames)
    }

    /// The shared start instant: the zero of every frame's `start_ms`.
    pub fn started_at(&self) -> Instant {
        self.start
    }

    /// Meeting time now.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// The state of every source right now, in the order they were given.
    ///
    /// Cheap, and never waits for a source. A source reads as
    /// [`Ended`](SourcePhase::Ended) only once its last frame is in the
    /// stream, so a consumer that sees it and then empties the receiver has
    /// everything that source delivered.
    pub fn status(&self) -> Vec<SourceStatus> {
        self.sources
            .iter()
            .map(|s| s.slot.lock().status.clone())
            .collect()
    }

    /// `true` once no source is starting or running: nothing more will
    /// arrive, whatever the reason.
    pub fn is_finished(&self) -> bool {
        self.sources
            .iter()
            .all(|s| !s.slot.lock().status.phase.is_live())
    }

    /// Stops every source and reports what happened.
    ///
    /// When this returns, every frame of the recording is in the receiver
    /// and its senders are gone, so `frames.iter()` drains it and ends.
    ///
    /// Running sources are told to stop all at once (so none of them runs
    /// on while another is closing its device) and are then waited for. A
    /// source whose `start` has still not returned is not waited for: it is
    /// marked [`Abandoned`](SourcePhase::Abandoned), nothing it might
    /// deliver later reaches the receiver, and it is stopped as soon as its
    /// `start` comes back.
    pub fn stop(mut self) -> RecorderReport {
        self.shut_down()
    }

    fn shut_down(&mut self) -> RecorderReport {
        let duration = self.start.elapsed();
        self.stopped = true;
        self.stop.store(true, Ordering::SeqCst);
        for source in &mut self.sources {
            {
                let mut inner = source.slot.lock();
                if inner.status.phase == SourcePhase::Starting {
                    inner.status.phase = SourcePhase::Abandoned;
                    inner.out = None;
                    // Not joined: its `start` may never return.
                    source.thread = None;
                }
            }
            if let Some(thread) = source.thread.take() {
                let _ = thread.join();
            }
        }
        RecorderReport {
            duration,
            sources: self.status(),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.stopped {
            self.shut_down();
        }
    }
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("elapsed", &self.elapsed())
            .field("sources", &self.status())
            .finish()
    }
}

/// Records a panic of a source (or of its supervisor) as that source's
/// failure, and lets go of its sender so the stream can still end.
fn fail_after_panic(slot: &Slot, panic: &(dyn Any + Send)) {
    let message = panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned());
    let mut inner = slot.lock();
    inner.out = None;
    let phase = match inner.status.phase {
        SourcePhase::Starting => SourcePhase::FailedToStart,
        SourcePhase::Running => SourcePhase::Ended,
        // Already settled; a panic while winding down changes nothing.
        other => other,
    };
    inner.status.phase = phase;
    inner.status.errors.push(AudioError::Other(format!(
        "the audio source panicked: {message}"
    )));
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// Why a supervisor stopped forwarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The source stopped delivering without being asked to.
    ByItself,
    /// [`Recorder::stop`] was called (or the recorder dropped).
    StopRequested,
    /// The caller dropped the frame receiver.
    NobodyListening,
}

/// Owns one source for its whole life: starts it, passes its frames on,
/// collects its errors, stops it.
struct Supervisor {
    source: Box<dyn AudioSource>,
    start: Instant,
    slot: Arc<Slot>,
    stop: Arc<AtomicBool>,
}

impl Supervisor {
    fn run(
        mut self,
        wait_for: Option<mpsc::Receiver<()>>,
        started: mpsc::Sender<()>,
        head_start: Duration,
    ) {
        if let Some(previous) = wait_for {
            self.wait_for_turn(&previous, head_start);
        }
        if self.stop.load(Ordering::SeqCst) {
            // Stopped before this source's turn came.
            let mut inner = self.slot.lock();
            inner.status.phase = SourcePhase::Abandoned;
            inner.out = None;
            return;
        }

        let (tx, rx) = frame_channel();
        let result = self.source.start(self.start, tx);
        let started_ms = elapsed_ms(self.start);
        // Lets the next source start.
        drop(started);

        let out = {
            let mut inner = self.slot.lock();
            if inner.status.phase == SourcePhase::Abandoned {
                // The recording ended while `start` was blocked.
                drop(inner);
                if result.is_ok() {
                    self.source.stop();
                }
                return;
            }
            inner.status.started_ms = Some(started_ms);
            match result {
                Ok(()) => {
                    inner.status.phase = SourcePhase::Running;
                    inner.out.take()
                }
                Err(error) => {
                    inner.status.errors.push(error);
                    inner.out = None;
                    drop(inner);
                    let queued = self.source.take_errors();
                    let mut inner = self.slot.lock();
                    inner.status.errors.extend(queued);
                    inner.status.phase = SourcePhase::FailedToStart;
                    return;
                }
            }
        };
        let Some(out) = out else {
            // Cannot happen: the sender is only taken here or on abandoning.
            self.source.stop();
            return;
        };

        // Forward until the source ends, the consumer goes away or the
        // recorder stops.
        let mut last_poll = Instant::now();
        let mut end = loop {
            match rx.recv_timeout(POLL) {
                Ok(frame) => {
                    if !self.forward(&out, frame) {
                        break End::NobodyListening;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                // The source dropped its sender: the one sure sign it ended.
                Err(RecvTimeoutError::Disconnected) => break End::ByItself,
            }
            if self.stop.load(Ordering::SeqCst) {
                break End::StopRequested;
            }
            if last_poll.elapsed() >= POLL {
                last_poll = Instant::now();
                self.collect_errors();
                if !self.source.is_running() {
                    break End::ByItself;
                }
            }
        };

        // A source that had already ended when the stop request came in was
        // not stopped by it.
        let was_running = self.source.is_running();
        // Returns once the source's last frame is sent and its sender gone.
        self.source.stop();
        if end != End::NobodyListening {
            for frame in rx.try_iter() {
                if !self.forward(&out, frame) {
                    end = End::NobodyListening;
                    break;
                }
            }
        }
        self.collect_errors();

        let mut inner = self.slot.lock();
        inner.status.phase = match end {
            End::StopRequested if was_running => SourcePhase::Stopped,
            End::StopRequested | End::ByItself => SourcePhase::Ended,
            End::NobodyListening => {
                // What a source itself reports when its receiver is gone.
                inner.status.errors.push(AudioError::SinkClosed);
                SourcePhase::Ended
            }
        };
        // Only now let go of the sender: a consumer that sees the phase
        // change has every frame, and one that sees the stream end sees the
        // final phase.
        drop(inner);
        drop(out);
    }

    /// Waits until the previous source's `start` has returned, the head
    /// start has passed, or the recorder is stopped.
    fn wait_for_turn(&self, previous: &mpsc::Receiver<()>, head_start: Duration) {
        let deadline = Instant::now() + head_start;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            match previous.recv_timeout(left.min(POLL)) {
                Err(RecvTimeoutError::Timeout) => {}
                // The previous supervisor dropped its end: its start is over.
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Counts a frame and passes it on. `false` if the caller's receiver is
    /// gone.
    fn forward(&self, out: &FrameSender, frame: AudioFrame) -> bool {
        {
            let mut inner = self.slot.lock();
            let status = &mut inner.status;
            let samples = frame.samples.len() as u64;
            let end_ms = frame.start_ms + (samples * 1000).div_ceil(u64::from(SAMPLE_RATE));
            status.frames += 1;
            status.samples += samples;
            status.first_frame_ms.get_or_insert(frame.start_ms);
            status.end_ms = Some(status.end_ms.map_or(end_ms, |end| end.max(end_ms)));
        }
        out.send(frame).is_ok()
    }

    fn collect_errors(&mut self) {
        let errors = self.source.take_errors();
        if !errors.is_empty() {
            self.slot.lock().status.errors.extend(errors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{FrameListSource, GatedSource, Probe};
    use crate::{FakeSource, Pacing};

    fn boxed(sources: Vec<FakeSource>) -> Vec<Box<dyn AudioSource>> {
        sources
            .into_iter()
            .map(|s| Box::new(s) as Box<dyn AudioSource>)
            .collect()
    }

    fn immediate() -> RecorderOptions {
        RecorderOptions {
            head_start: Duration::ZERO,
        }
    }

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn starts(frames: &[AudioFrame], speaker: Speaker) -> Vec<u64> {
        frames
            .iter()
            .filter(|f| f.speaker == speaker)
            .map(|f| f.start_ms)
            .collect()
    }

    /// A minute of audio delivered in real time: a source that is still
    /// running whenever the test looks.
    fn endless(speaker: Speaker) -> FakeSource {
        FakeSource::from_samples(speaker, vec![0.1; 960_000]).with_pacing(Pacing::RealTime)
    }

    #[test]
    fn two_sources_share_one_clock_and_one_stream() {
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.1; 3_200]);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.2; 1_600])
            .starting_at(Duration::from_millis(60));
        let (recorder, rx) = Recorder::start(boxed(vec![mic, system]));

        // Both fakes run out; the stream ends without `stop`.
        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert_eq!(
            starts(&frames, Speaker::Me),
            (0..10).map(|i| i * 20).collect::<Vec<_>>()
        );
        assert_eq!(
            starts(&frames, Speaker::Them),
            (0..5).map(|i| 60 + i * 20).collect::<Vec<_>>()
        );
        // The audio is passed on untouched.
        assert!(frames.iter().all(|f| {
            f.samples
                .iter()
                .all(|s| *s == if f.speaker == Speaker::Me { 0.1 } else { 0.2 })
        }));

        // The stream ending means every source has settled.
        assert!(recorder.is_finished());
        let report = recorder.stop();
        assert!(report.errors().is_empty());
        let mic = report.source(Speaker::Me).unwrap();
        assert_eq!(mic.phase, SourcePhase::Ended);
        assert_eq!((mic.frames, mic.samples), (10, 3_200));
        assert_eq!((mic.first_frame_ms, mic.end_ms), (Some(0), Some(200)));
        assert_eq!(mic.captured(), Duration::from_millis(200));
        assert!(mic.started_ms.is_some());
        let system = report.source(Speaker::Them).unwrap();
        assert_eq!(system.phase, SourcePhase::Ended);
        assert_eq!((system.frames, system.samples), (5, 1_600));
        assert_eq!(
            (system.first_frame_ms, system.end_ms),
            (Some(60), Some(160))
        );
    }

    #[test]
    fn every_source_is_given_the_same_start_instant() {
        let probe_a = Probe::default();
        let probe_b = Probe::default();
        let a = GatedSource::open(
            FakeSource::from_samples(Speaker::Me, vec![0.0; 320]),
            &probe_a,
        );
        // The second source's start is held back for a while.
        let b = GatedSource::closed(
            FakeSource::from_samples(Speaker::Them, vec![0.0; 320]),
            &probe_b,
        );
        let before = Instant::now();
        let (recorder, rx) = Recorder::start_with(vec![Box::new(a), Box::new(b)], immediate());
        let after = Instant::now();
        thread::sleep(Duration::from_millis(40));
        probe_b.release();

        assert_eq!(rx.iter().count(), 2);
        let start = recorder.started_at();
        assert!(before <= start && start <= after);
        assert_eq!(probe_a.start_instant(), Some(start));
        assert_eq!(probe_b.start_instant(), Some(start));
        // The late one is on record as late, on that same clock.
        let report = recorder.stop();
        assert!(report.sources[1].started_ms.unwrap() >= 40);
    }

    #[test]
    fn a_source_that_fails_to_start_does_not_stop_the_other() {
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.1; 16_000])
            .failing_to_start(AudioError::NoInputDevice);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.2; 16_000]);
        let (recorder, rx) = Recorder::start(boxed(vec![mic, system]));

        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert!(starts(&frames, Speaker::Me).is_empty());
        assert_eq!(starts(&frames, Speaker::Them).len(), 50);

        let report = recorder.stop();
        let mic = report.source(Speaker::Me).unwrap();
        assert_eq!(mic.phase, SourcePhase::FailedToStart);
        assert!(mic.phase.is_failure());
        assert_eq!(mic.errors, [AudioError::NoInputDevice]);
        assert_eq!(
            (mic.frames, mic.first_frame_ms, mic.end_ms),
            (0, None, None)
        );
        let system = report.source(Speaker::Them).unwrap();
        assert_eq!(system.phase, SourcePhase::Ended);
        assert_eq!(system.samples, 16_000);
        assert!(system.errors.is_empty());
        assert_eq!(report.errors(), [(Speaker::Me, AudioError::NoInputDevice)]);
    }

    #[test]
    fn a_source_that_dies_mid_run_is_reported_while_the_other_carries_on() {
        let died = AudioError::Stream("device unplugged".to_owned());
        let mic = endless(Speaker::Me);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.2; 16_000])
            .failing_after(Duration::from_millis(250), died.clone());
        let (recorder, rx) = Recorder::start(boxed(vec![mic, system]));

        // Visible while the recording is still going.
        wait_until("the failure to show", || {
            recorder.status()[1].phase == SourcePhase::Ended
        });
        let status = recorder.status();
        assert_eq!(status[1].errors, std::slice::from_ref(&died));
        // Everything it delivered before dying is already in the stream.
        assert_eq!(status[1].samples, 4_000);
        assert_eq!(status[1].end_ms, Some(250));
        let so_far: Vec<AudioFrame> = rx.try_iter().collect();
        assert_eq!(
            so_far
                .iter()
                .filter(|f| f.speaker == Speaker::Them)
                .map(|f| f.samples.len())
                .sum::<usize>(),
            4_000
        );
        assert_eq!(status[0].phase, SourcePhase::Running);
        assert!(!recorder.is_finished());

        // The microphone keeps delivering after the other channel died.
        let later = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(later.speaker, Speaker::Me);

        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Stopped);
        assert!(!report.sources[0].phase.is_failure());
        assert!(report.sources[0].errors.is_empty());
        assert_eq!(report.sources[1].phase, SourcePhase::Ended);
        assert_eq!(report.errors(), [(Speaker::Them, died)]);
        // Stopped: the stream ends.
        assert!(rx.iter().all(|f| f.speaker == Speaker::Me));
    }

    #[test]
    fn a_start_that_blocks_does_not_hold_up_the_other_source() {
        let probe = Probe::default();
        let mic = endless(Speaker::Me);
        // Like a system audio source waiting on a permission prompt.
        let system = GatedSource::closed(
            FakeSource::from_samples(Speaker::Them, vec![0.2; 1_600]),
            &probe,
        );
        let begun = Instant::now();
        let (recorder, rx) = Recorder::start(vec![Box::new(mic), Box::new(system)]);
        // Starting the recorder did not wait for anything.
        assert!(begun.elapsed() < Duration::from_secs(1));

        // Microphone frames flow while the other start is stuck.
        wait_until("the blocked start to be entered", || {
            probe.start_calls() == 1
        });
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.speaker, Speaker::Me);
        let status = recorder.status();
        assert_eq!(status[0].phase, SourcePhase::Running);
        assert_eq!(status[1].phase, SourcePhase::Starting);
        assert_eq!(status[1].started_ms, None);

        // The prompt is answered: the late source joins the same stream.
        let released_ms = recorder.elapsed().as_millis() as u64;
        probe.release();
        wait_until("the late source to finish", || {
            recorder.status()[1].phase == SourcePhase::Ended
        });
        let status = recorder.status();
        assert!(status[1].started_ms.unwrap() >= released_ms);
        assert_eq!(status[1].samples, 1_600);
        let them: usize = rx
            .try_iter()
            .filter(|f| f.speaker == Speaker::Them)
            .map(|f| f.samples.len())
            .sum();
        assert_eq!(them, 1_600);
        recorder.stop();
    }

    #[test]
    fn stop_does_not_wait_for_a_start_that_never_returns() {
        let probe = Probe::default();
        let mic = endless(Speaker::Me);
        let system = GatedSource::closed(
            FakeSource::from_samples(Speaker::Them, vec![0.2; 16_000]),
            &probe,
        );
        let (recorder, rx) =
            Recorder::start_with(vec![Box::new(mic), Box::new(system)], immediate());
        wait_until("the blocked start to be entered", || {
            probe.start_calls() == 1
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let stopping = Instant::now();
        let report = recorder.stop();
        assert!(stopping.elapsed() < Duration::from_secs(2));
        assert_eq!(report.sources[0].phase, SourcePhase::Stopped);
        let system = &report.sources[1];
        assert_eq!(system.phase, SourcePhase::Abandoned);
        assert!(system.phase.is_failure());
        assert_eq!((system.frames, system.started_ms), (0, None));

        // The stream ends although that source still holds its start.
        assert!(rx.iter().all(|f| f.speaker == Speaker::Me));
        assert_eq!(probe.stop_calls(), 0);

        // When its start finally returns, it is stopped at once and nothing
        // it captured reaches anyone.
        probe.release();
        wait_until("the abandoned source to be stopped", || {
            probe.stop_calls() == 1
        });
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn sources_start_in_order_and_the_next_waits_for_the_previous() {
        let first = Probe::default();
        let second = Probe::default();
        let mic = GatedSource::closed(
            FakeSource::from_samples(Speaker::Me, vec![0.0; 320]),
            &first,
        );
        let system = GatedSource::open(
            FakeSource::from_samples(Speaker::Them, vec![0.0; 320]),
            &second,
        );
        // A head start far longer than the test: only the first start
        // returning can let the second begin.
        let options = RecorderOptions {
            head_start: Duration::from_secs(60),
        };
        let (recorder, rx) = Recorder::start_with(vec![Box::new(mic), Box::new(system)], options);
        wait_until("the first start to be entered", || first.start_calls() == 1);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(second.start_calls(), 0, "started before its turn");
        assert_eq!(recorder.status()[1].phase, SourcePhase::Starting);

        first.release();
        assert_eq!(rx.iter().count(), 2);
        assert!(second.start_entered_at().unwrap() >= first.start_returned_at().unwrap());
        recorder.stop();
    }

    #[test]
    fn the_head_start_bounds_how_long_the_next_source_waits() {
        let first = Probe::default();
        let second = Probe::default();
        let mic = GatedSource::closed(
            FakeSource::from_samples(Speaker::Me, vec![0.0; 320]),
            &first,
        );
        let system = GatedSource::open(
            FakeSource::from_samples(Speaker::Them, vec![0.3; 320]),
            &second,
        );
        let options = RecorderOptions {
            head_start: Duration::from_millis(60),
        };
        let (recorder, rx) = Recorder::start_with(vec![Box::new(mic), Box::new(system)], options);

        // The first start never returns; the second goes ahead anyway.
        let frame = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(frame.speaker, Speaker::Them);
        let waited = second
            .start_entered_at()
            .unwrap()
            .duration_since(recorder.started_at());
        assert!(
            waited >= Duration::from_millis(60),
            "waited only {waited:?}"
        );
        assert_eq!(first.start_returned_at(), None);
        assert_eq!(recorder.status()[0].phase, SourcePhase::Starting);

        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Abandoned);
        first.release();
    }

    #[test]
    fn stopping_before_a_sources_turn_never_starts_it() {
        let first = Probe::default();
        let second = Probe::default();
        let mic = GatedSource::closed(
            FakeSource::from_samples(Speaker::Me, vec![0.0; 320]),
            &first,
        );
        let system = GatedSource::open(
            FakeSource::from_samples(Speaker::Them, vec![0.0; 320]),
            &second,
        );
        let options = RecorderOptions {
            head_start: Duration::from_secs(60),
        };
        let (recorder, rx) = Recorder::start_with(vec![Box::new(mic), Box::new(system)], options);
        wait_until("the first start to be entered", || first.start_calls() == 1);

        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Abandoned);
        assert_eq!(report.sources[1].phase, SourcePhase::Abandoned);
        assert_eq!(rx.iter().count(), 0);
        first.release();
        wait_until("the first source to be stopped", || first.stop_calls() == 1);
        thread::sleep(Duration::from_millis(60));
        assert_eq!(second.start_calls(), 0);
    }

    #[test]
    fn stop_stops_running_sources_and_ends_the_stream() {
        let mic = endless(Speaker::Me);
        let system = endless(Speaker::Them);
        let (recorder, rx) = Recorder::start_with(boxed(vec![mic, system]), immediate());
        wait_until("both to run", || {
            recorder
                .status()
                .iter()
                .all(|s| s.phase == SourcePhase::Running)
        });
        thread::sleep(Duration::from_millis(100));
        assert!(!recorder.is_finished());

        let report = recorder.stop();
        assert!(report.duration >= Duration::from_millis(100));
        assert!(report.duration < Duration::from_secs(30));
        let frames: Vec<AudioFrame> = rx.iter().collect();
        for (source, speaker) in report.sources.iter().zip([Speaker::Me, Speaker::Them]) {
            assert_eq!(source.phase, SourcePhase::Stopped);
            assert!(source.errors.is_empty());
            // The report counts exactly what came out of the stream.
            let delivered: Vec<&AudioFrame> =
                frames.iter().filter(|f| f.speaker == speaker).collect();
            assert!(!delivered.is_empty());
            assert_eq!(source.frames, delivered.len() as u64);
            assert_eq!(source.first_frame_ms, Some(0));
            assert_eq!(source.end_ms, Some(delivered.last().unwrap().start_ms + 20));
        }
    }

    #[test]
    fn sources_are_stopped_together_not_one_after_the_other() {
        // The microphone takes 400 ms to close. System audio must not go on
        // capturing for that long just because it is second in the list.
        let probe = Probe::default();
        probe.stop_takes(Duration::from_millis(400));
        let mic = GatedSource::open(endless(Speaker::Me), &probe);
        let system = endless(Speaker::Them);
        let (recorder, rx) =
            Recorder::start_with(vec![Box::new(mic), Box::new(system)], immediate());
        wait_until("both to deliver", || {
            recorder.status().iter().all(|s| s.frames > 0)
        });

        let report = recorder.stop();
        let mic_end = report.sources[0].end_ms.unwrap();
        let system_end = report.sources[1].end_ms.unwrap();
        assert!(
            system_end < mic_end + 200,
            "the microphone ended at {mic_end} ms, system audio ran on to {system_end} ms"
        );
        assert!(
            report
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Stopped)
        );
        drop(rx);
    }

    #[test]
    fn errors_of_a_source_that_keeps_running_are_collected() {
        let probe = Probe::default();
        let mic = GatedSource::open(endless(Speaker::Me), &probe);
        let (recorder, _rx) = Recorder::start(vec![Box::new(mic)]);
        wait_until("the source to run", || {
            recorder.status()[0].phase == SourcePhase::Running
        });

        // A device hiccup the source recovered from by itself.
        probe.inject_error(AudioError::Stream("re-opened".to_owned()));
        wait_until("the error to be collected", || {
            !recorder.status()[0].errors.is_empty()
        });
        assert_eq!(recorder.status()[0].phase, SourcePhase::Running);
        probe.inject_error(AudioError::NoInputDevice);

        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Stopped);
        assert_eq!(
            report.sources[0].errors,
            [
                AudioError::Stream("re-opened".to_owned()),
                AudioError::NoInputDevice
            ]
        );
    }

    #[test]
    fn a_source_that_panics_is_a_failed_source_not_a_dead_recorder() {
        let probe = Probe::default();
        probe.panic_on_start("tap exploded");
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.1; 3_200]);
        let system = GatedSource::open(
            FakeSource::from_samples(Speaker::Them, vec![0.2; 3_200]),
            &probe,
        );
        let (recorder, rx) = Recorder::start(vec![Box::new(mic), Box::new(system)]);

        // The stream still ends, with the healthy source's audio.
        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert_eq!(starts(&frames, Speaker::Me).len(), 10);
        assert!(starts(&frames, Speaker::Them).is_empty());
        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Ended);
        assert_eq!(report.sources[1].phase, SourcePhase::FailedToStart);
        assert_eq!(
            report.sources[1].errors,
            [AudioError::Other(
                "the audio source panicked: tap exploded".to_owned()
            )]
        );
    }

    #[test]
    fn a_consumer_that_goes_away_ends_the_sources() {
        let probe = Probe::default();
        let mic = GatedSource::open(endless(Speaker::Me), &probe);
        let (recorder, rx) = Recorder::start(vec![Box::new(mic)]);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(rx);

        wait_until("the source to be ended", || recorder.is_finished());
        assert_eq!(probe.stop_calls(), 1);
        let report = recorder.stop();
        assert_eq!(report.sources[0].phase, SourcePhase::Ended);
        assert_eq!(report.sources[0].errors, [AudioError::SinkClosed]);
    }

    #[test]
    fn dropping_the_recorder_stops_its_sources() {
        let probe = Probe::default();
        let mic = GatedSource::open(endless(Speaker::Me), &probe);
        let (recorder, rx) = Recorder::start(vec![Box::new(mic)]);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(recorder);
        assert_eq!(probe.stop_calls(), 1);
        // And the stream ends.
        let _rest: Vec<AudioFrame> = rx.iter().collect();
    }

    #[test]
    fn gaps_and_short_frames_pass_through_unchanged() {
        let frames = vec![
            AudioFrame {
                speaker: Speaker::Them,
                start_ms: 100,
                samples: vec![0.5; 320],
            },
            AudioFrame {
                speaker: Speaker::Them,
                start_ms: 120,
                samples: vec![0.5; 100],
            },
            // A device change: 1.9 s without audio.
            AudioFrame {
                speaker: Speaker::Them,
                start_ms: 2_000,
                samples: vec![0.25; 320],
            },
        ];
        let source = FrameListSource::new(Speaker::Them, frames.clone());
        let (recorder, rx) = Recorder::start(vec![Box::new(source)]);
        assert_eq!(rx.iter().collect::<Vec<_>>(), frames);
        let report = recorder.stop();
        let status = &report.sources[0];
        assert_eq!((status.frames, status.samples), (3, 740));
        assert_eq!(
            (status.first_frame_ms, status.end_ms),
            (Some(100), Some(2_020))
        );
        // 740 samples are 46.25 ms of audio, whatever the span they cover.
        assert_eq!(status.captured(), Duration::from_micros(46_250));
    }

    #[test]
    fn the_recorder_can_be_handed_to_another_thread() {
        fn assert_send_and_sync<T: Send + Sync>() {}
        assert_send_and_sync::<Recorder>();
        assert_send_and_sync::<RecorderReport>();
    }

    #[test]
    fn a_recorder_with_no_sources_is_finished_at_once() {
        let (recorder, rx) = Recorder::start(Vec::new());
        assert!(rx.recv().is_err());
        assert!(recorder.is_finished());
        assert!(recorder.stop().sources.is_empty());
    }
}
