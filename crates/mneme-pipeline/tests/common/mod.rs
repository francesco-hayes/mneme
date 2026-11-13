//! Helpers shared by the integration tests: synthetic audio the chunker
//! cuts predictably, a sink that records what it is given, a transcriber
//! that can be held at a gate, and a watchdog so that a deadlock fails a
//! test instead of hanging it.

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use mneme_audio::{AudioError, AudioSource, FakeSource, FrameSender, Pacing};
use mneme_core::{SAMPLE_RATE, Segment, Speaker};
use mneme_pipeline::{Session, SessionConfig, SessionStatus, SinkError, TranscriptSink};
use mneme_transcribe::{Language, TextSpan, TranscribeError, Transcriber, duration_ms};

const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;

/// RMS level of the low noise used as silence: -66 dBFS, well under the
/// chunker's minimum speech level.
const QUIET: f32 = 0.0005;
/// Amplitude of the tone used as speech.
const LOUD: f32 = 0.3;

/// How long any single test may run before it is declared stuck.
pub const TEST_LIMIT: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs a test body on a thread of its own and fails the test if it has not
/// finished within [`TEST_LIMIT`]. Without it a deadlock in `stop()` would
/// hang the whole suite.
pub fn watchdog(body: impl FnOnce() + Send + 'static) {
    let (done_tx, done_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("test-body".to_owned())
        .spawn(move || {
            body();
            let _ = done_tx.send(());
        })
        .unwrap();
    match done_rx.recv_timeout(TEST_LIMIT) {
        Ok(()) => thread.join().unwrap(),
        // The body panicked: pass its panic on.
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("the test did not finish within {TEST_LIMIT:?}: something is stuck")
        }
    }
}

/// Polls `condition` until it returns `Some`, or panics with `what` after
/// 20 seconds.
pub fn wait_for<T>(what: &str, mut condition: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(value) = condition() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Polls the session's status until `condition` holds and returns that
/// status.
pub fn wait_for_status(
    session: &Session,
    what: &str,
    mut condition: impl FnMut(&SessionStatus) -> bool,
) -> SessionStatus {
    wait_for(what, || {
        let status = session.status();
        condition(&status).then_some(status)
    })
}

/// A small deterministic random generator (xorshift), so the audio is the
/// same on every run.
struct Rng(u32);

impl Rng {
    /// Uniform in `-1.0..1.0`.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

/// Builds test audio piece by piece: low noise for silence, a tone for
/// speech. With the default chunker settings a burst that has at least
/// 700 ms of quiet after it (or the end of the stream) becomes one chunk
/// that starts 200 ms before the burst and ends 200 ms after it.
pub struct Tape {
    samples: Vec<f32>,
    rng: Rng,
}

impl Tape {
    pub fn new() -> Self {
        Tape {
            samples: Vec::new(),
            rng: Rng(0x9E37_79B9),
        }
    }

    /// Length so far, in milliseconds.
    pub fn ms(&self) -> u64 {
        self.samples.len() as u64 / SAMPLES_PER_MS
    }

    /// "Silence": low noise.
    pub fn quiet(mut self, ms: u64) -> Self {
        // Uniform noise in -a..a has an RMS of a / sqrt(3).
        let amplitude = QUIET * 3.0_f32.sqrt();
        for _ in 0..ms * SAMPLES_PER_MS {
            let sample = amplitude * self.rng.next();
            self.samples.push(sample);
        }
        self
    }

    /// "Speech": a 220 Hz tone plus a little noise.
    pub fn burst(mut self, ms: u64) -> Self {
        let step = std::f32::consts::TAU * 220.0 / SAMPLE_RATE as f32;
        for i in 0..ms * SAMPLES_PER_MS {
            let sample = LOUD * (i as f32 * step).sin() + LOUD * 0.1 * self.rng.next();
            self.samples.push(sample);
        }
        self
    }

    /// Quiet up to `at_ms` on the tape, then a burst of `ms`.
    pub fn burst_at(self, at_ms: u64, ms: u64) -> Self {
        let now = self.ms();
        assert!(at_ms >= now, "the tape is already at {now} ms");
        self.quiet(at_ms - now).burst(ms)
    }

    /// Quiet up to `total_ms` on the tape.
    pub fn until(self, total_ms: u64) -> Self {
        let now = self.ms();
        assert!(total_ms >= now, "the tape is already at {now} ms");
        self.quiet(total_ms - now)
    }

    /// Unbroken "speech" for `ms`: bursts of 200 ms with dips of 60 ms, far
    /// shorter than the pause that ends a chunk.
    pub fn talking(mut self, ms: u64) -> Self {
        let end = self.ms() + ms;
        while self.ms() < end {
            let on = 200.min(end - self.ms());
            self = self.burst(on);
            let off = 60.min(end - self.ms());
            self = self.quiet(off);
        }
        self
    }

    pub fn samples(self) -> Vec<f32> {
        self.samples
    }

    /// A fake source that plays the tape as `speaker`, as fast as it is
    /// read.
    pub fn source(self, speaker: Speaker) -> FakeSource {
        FakeSource::from_samples(speaker, self.samples)
    }

    /// A fake source that plays the tape as `speaker` in real time.
    pub fn live_source(self, speaker: Speaker) -> FakeSource {
        self.source(speaker).with_pacing(Pacing::RealTime)
    }
}

/// A tape with one burst of `burst_ms` at each of `starts_ms`, and quiet up
/// to `total_ms`.
pub fn bursts(starts_ms: &[u64], burst_ms: u64, total_ms: u64) -> Tape {
    starts_ms
        .iter()
        .fold(Tape::new(), |tape, &at| tape.burst_at(at, burst_ms))
        .until(total_ms)
}

/// Boxes sources for `Session::start`.
pub fn sources<const N: usize>(list: [FakeSource; N]) -> Vec<Box<dyn AudioSource>> {
    list.into_iter()
        .map(|source| Box::new(source) as Box<dyn AudioSource>)
        .collect()
}

/// The session settings the tests use: English, and no head start between
/// the sources so a test does not wait a second for the second one.
pub fn config() -> SessionConfig {
    let mut config = SessionConfig::new(Language::english());
    config.recorder.head_start = Duration::ZERO;
    config
}

/// What a [`RecordingSink`] saw, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkEvent {
    /// `append` was called with this segment and returned this.
    Append(Segment, Result<bool, SinkError>),
    /// `finalize` was called.
    Finalize,
}

#[derive(Default)]
struct SinkLog {
    events: Vec<SinkEvent>,
    /// Appends (counted from 1) that fail.
    fail_appends: Vec<usize>,
    /// Appends (counted from 1) that panic.
    panic_appends: Vec<usize>,
    appends: usize,
    fail_finalize: bool,
    panic_on_drop: bool,
    dropped: bool,
}

/// A sink that keeps everything it is given, and can be told to fail.
/// Clones share one log: give one to the session and keep one to look at.
#[derive(Clone, Default)]
pub struct RecordingSink {
    log: Arc<Mutex<SinkLog>>,
    /// Every append waits here first, if set.
    gate: Option<Gate>,
    /// Set on the clone handed to the session.
    owned_by_session: bool,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// The clone to hand to the session. Dropping it is recorded.
    pub fn for_session(&self) -> Self {
        RecordingSink {
            log: Arc::clone(&self.log),
            gate: self.gate.clone(),
            owned_by_session: true,
        }
    }

    /// Makes every append wait at `gate` before it does anything.
    pub fn gated(mut self, gate: &Gate) -> Self {
        self.gate = Some(gate.clone());
        self
    }

    /// Makes the `n`th append (counted from 1) return an error.
    pub fn failing_on_append(self, n: usize) -> Self {
        lock(&self.log).fail_appends.push(n);
        self
    }

    /// Makes the `n`th append (counted from 1) panic.
    pub fn panicking_on_append(self, n: usize) -> Self {
        lock(&self.log).panic_appends.push(n);
        self
    }

    /// Makes `finalize` return an error.
    pub fn failing_on_finalize(self) -> Self {
        lock(&self.log).fail_finalize = true;
        self
    }

    /// Makes dropping the session's clone panic.
    pub fn panicking_on_drop(self) -> Self {
        lock(&self.log).panic_on_drop = true;
        self
    }

    pub fn events(&self) -> Vec<SinkEvent> {
        lock(&self.log).events.clone()
    }

    /// Every segment `append` was called with, in call order, whatever it
    /// returned.
    pub fn offered(&self) -> Vec<Segment> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                SinkEvent::Append(segment, _) => Some(segment),
                SinkEvent::Finalize => None,
            })
            .collect()
    }

    /// The segments that were written (`append` returned `Ok(true)`), in
    /// call order.
    pub fn written(&self) -> Vec<Segment> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                SinkEvent::Append(segment, Ok(true)) => Some(segment),
                _ => None,
            })
            .collect()
    }

    /// The written segments in time order, as the finalized transcript has
    /// them.
    pub fn written_sorted(&self) -> Vec<Segment> {
        let mut segments = self.written();
        segments.sort_by_key(|segment| segment.start_ms);
        segments
    }

    /// The written lines of one speaker as `(start_ms, text)`, in time
    /// order.
    pub fn lines_of(&self, speaker: Speaker) -> Vec<(u64, String)> {
        self.written_sorted()
            .into_iter()
            .filter(|segment| segment.speaker == speaker)
            .map(|segment| (segment.start_ms, segment.text))
            .collect()
    }

    pub fn finalize_count(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| **event == SinkEvent::Finalize)
            .count()
    }

    /// `true` if `finalize` was called exactly once and nothing was
    /// appended after it.
    pub fn finalized_once_at_the_end(&self) -> bool {
        let events = self.events();
        self.finalize_count() == 1 && events.last() == Some(&SinkEvent::Finalize)
    }

    /// `true` once the session has dropped its clone.
    pub fn dropped_by_session(&self) -> bool {
        lock(&self.log).dropped
    }
}

impl TranscriptSink for RecordingSink {
    fn append(&mut self, segment: &Segment) -> Result<bool, SinkError> {
        if let Some(gate) = &self.gate {
            gate.pass();
        }
        let mut log = lock(&self.log);
        log.appends += 1;
        let n = log.appends;
        if log.panic_appends.contains(&n) {
            let error = Err(SinkError::new("panicked"));
            log.events.push(SinkEvent::Append(segment.clone(), error));
            drop(log);
            panic!("scripted sink panic on append {n}");
        }
        let result = if log.fail_appends.contains(&n) {
            Err(SinkError::new(format!("scripted failure on append {n}")))
        } else {
            // As the store's writer does for a segment with no text.
            Ok(!segment.text.trim().is_empty())
        };
        log.events
            .push(SinkEvent::Append(segment.clone(), result.clone()));
        result
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        let mut log = lock(&self.log);
        log.events.push(SinkEvent::Finalize);
        if log.fail_finalize {
            Err(SinkError::new("scripted finalize failure"))
        } else {
            Ok(())
        }
    }
}

impl Drop for RecordingSink {
    fn drop(&mut self) {
        if !self.owned_by_session {
            return;
        }
        let panic_now = {
            let mut log = lock(&self.log);
            log.dropped = true;
            log.panic_on_drop
        };
        if panic_now && !thread::panicking() {
            panic!("scripted sink panic on drop");
        }
    }
}

/// A gate a [`TestTranscriber`], a [`RecordingSink`] or a
/// [`SlowStartSource`] waits at. Closed until opened. Clones share one gate.
#[derive(Clone, Default)]
pub struct Gate {
    state: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    pub fn closed() -> Self {
        Self::default()
    }

    pub fn open(&self) {
        *lock(&self.state.0) = true;
        self.state.1.notify_all();
    }

    fn pass(&self) {
        let mut open = lock(&self.state.0);
        while !*open {
            open = self
                .state
                .1
                .wait(open)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Opens a gate when dropped. Keep one in a test that closes a gate, so a
/// failed assertion cannot leave the session's drop waiting forever.
pub struct OpenOnDrop(pub Gate);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}

type Reply = dyn Fn(usize, &[f32]) -> Result<Vec<TextSpan>, TranscribeError> + Send + Sync;

/// A transcriber for what `FakeTranscriber` cannot do: text that depends on
/// the audio (so a line can be matched to its chunk whatever order the
/// chunks were served in), a gate to hold it inside a call, and panics.
pub struct TestTranscriber {
    reply: Box<Reply>,
    gate: Option<Gate>,
    /// Calls that have begun.
    entered: AtomicUsize,
    /// Calls that have returned.
    finished: AtomicUsize,
    in_call: AtomicBool,
    overlapped: AtomicBool,
}

impl TestTranscriber {
    /// Replies with what `reply` returns for the call number (from 1) and
    /// the samples.
    pub fn new(
        reply: impl Fn(usize, &[f32]) -> Result<Vec<TextSpan>, TranscribeError> + Send + Sync + 'static,
    ) -> Self {
        TestTranscriber {
            reply: Box::new(reply),
            gate: None,
            entered: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
            in_call: AtomicBool::new(false),
            overlapped: AtomicBool::new(false),
        }
    }

    /// One span over the whole chunk whose text is the chunk's length:
    /// `1000 ms`. Bursts of different lengths give lines that name their
    /// chunk.
    pub fn naming_lengths() -> Self {
        Self::new(|_, samples| Ok(vec![whole(samples, length_text(samples))]))
    }

    /// Every call waits at `gate` before it replies.
    pub fn gated(mut self, gate: &Gate) -> Self {
        self.gate = Some(gate.clone());
        self
    }

    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    pub fn finished(&self) -> usize {
        self.finished.load(Ordering::SeqCst)
    }

    /// `true` if two calls ever ran at the same time.
    pub fn overlapped(&self) -> bool {
        self.overlapped.load(Ordering::SeqCst)
    }
}

impl Transcriber for TestTranscriber {
    fn transcribe(
        &self,
        samples: &[f32],
        _language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        let call = self.entered.fetch_add(1, Ordering::SeqCst) + 1;
        if self.in_call.swap(true, Ordering::SeqCst) {
            self.overlapped.store(true, Ordering::SeqCst);
        }
        if let Some(gate) = &self.gate {
            gate.pass();
        }
        // Count the call as over even if the reply panics.
        struct Finish<'a>(&'a TestTranscriber);
        impl Drop for Finish<'_> {
            fn drop(&mut self) {
                self.0.in_call.store(false, Ordering::SeqCst);
                self.0.finished.fetch_add(1, Ordering::SeqCst);
            }
        }
        let _finish = Finish(self);
        (self.reply)(call, samples)
    }
}

/// The text [`TestTranscriber::naming_lengths`] gives a chunk of this
/// length.
pub fn length_text(samples: &[f32]) -> String {
    format!("{} ms", duration_ms(samples.len()))
}

/// One span over the whole slice.
pub fn whole(samples: &[f32], text: impl Into<String>) -> TextSpan {
    TextSpan {
        start_ms: 0,
        end_ms: duration_ms(samples.len()),
        text: text.into(),
    }
}

pub fn span(start_ms: u64, end_ms: u64, text: &str) -> TextSpan {
    TextSpan {
        start_ms,
        end_ms,
        text: text.to_owned(),
    }
}

pub fn engine_error(message: &str) -> TranscribeError {
    TranscribeError::Engine {
        message: message.to_owned(),
    }
}

/// A source whose `start` blocks until a gate is opened, like the system
/// audio source while a permission prompt is on screen. Then it plays its
/// fake.
pub struct SlowStartSource {
    inner: FakeSource,
    gate: Gate,
    /// Set when `start` has been entered.
    pub entered: Arc<AtomicBool>,
}

impl SlowStartSource {
    pub fn new(inner: FakeSource, gate: &Gate) -> Self {
        SlowStartSource {
            inner,
            gate: gate.clone(),
            entered: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl AudioSource for SlowStartSource {
    fn speaker(&self) -> Speaker {
        self.inner.speaker()
    }

    fn start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError> {
        self.entered.store(true, Ordering::SeqCst);
        self.gate.pass();
        self.inner.start(start, sink)
    }

    fn stop(&mut self) {
        self.inner.stop();
    }

    fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    fn take_errors(&mut self) -> Vec<AudioError> {
        self.inner.take_errors()
    }
}
