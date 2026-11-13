//! The capture loop shared by device-backed sources: open a stream on the
//! current default device, turn its buffers into frames, and re-open when the
//! device changes, errors or goes quiet, without restarting the meeting clock.
//!
//! A source supplies a [`Backend`] that knows how to open its kind of stream
//! (cpal for the microphone; Core Audio taps for system audio in issue 003).
//! Everything else lives here, so it can be tested with a scripted backend
//! and no hardware.
//!
//! # Threads
//!
//! The device's audio callback does as little as possible: it downmixes its
//! buffer to mono `f32` and sends it to the capture thread
//! ([`StreamEvents::data`]). The capture thread owns the stream and the
//! [`Framer`], does the resampling and delivers frames. Heavy work never runs
//! on the real-time audio thread.

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mneme_core::Speaker;

use crate::convert::{ToF32, downmix_interleaved};
use crate::source::SourceState;
use crate::{AudioError, FrameSender, Framer};

/// A stream a [`Backend`] opened.
pub(crate) struct OpenStream<S> {
    /// Keeps the stream alive; dropping it must stop the callbacks. Never
    /// read, only held and dropped.
    #[allow(dead_code)]
    pub stream: S,
    /// Identity of the device the stream runs on, compared against
    /// [`Backend::current_device`] to notice a change of default device.
    pub device: String,
    /// Rate of the audio the callbacks deliver, in Hz.
    pub sample_rate: u32,
}

/// Opens streams for one kind of device.
pub(crate) trait Backend {
    /// The live stream. It does not need to be `Send`: it is created, held
    /// and dropped on the capture thread.
    type Stream;

    /// Identity of the device a stream opened now would use, or `None` when
    /// there is no such device.
    fn current_device(&mut self) -> Option<String>;

    /// Opens and starts a stream on the current default device. Its callbacks
    /// report through `events`.
    fn open(&mut self, events: StreamEvents) -> Result<OpenStream<Self::Stream>, AudioError>;
}

enum Event {
    Stop,
    Data {
        generation: u64,
        mono: Vec<f32>,
        first_sample_at: Instant,
    },
    StreamError {
        generation: u64,
        error: AudioError,
    },
}

/// What a stream's callbacks use to talk to the capture thread. Events from a
/// stream that has since been replaced are ignored.
#[derive(Clone)]
pub(crate) struct StreamEvents {
    tx: mpsc::Sender<Event>,
    generation: u64,
}

impl StreamEvents {
    /// Hands over one buffer of interleaved samples at the stream's rate.
    /// `first_sample_at` is when the buffer's first sample was captured.
    pub(crate) fn data<T: ToF32>(
        &self,
        interleaved: &[T],
        channels: usize,
        first_sample_at: Instant,
    ) {
        let mut mono = Vec::with_capacity(interleaved.len() / channels.max(1));
        downmix_interleaved(interleaved, channels, &mut mono);
        let _ = self.tx.send(Event::Data {
            generation: self.generation,
            mono,
            first_sample_at,
        });
    }

    /// Reports that the stream broke. The capture thread queues the error for
    /// the source's owner and re-opens.
    pub(crate) fn error(&self, error: AudioError) {
        let _ = self.tx.send(Event::StreamError {
            generation: self.generation,
            error,
        });
    }
}

/// How often the capture thread checks things and how patient it is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// Interval between checks for a changed default device and a stall.
    pub poll: Duration,
    /// A stream that delivers nothing for this long is re-opened.
    pub stall: Duration,
    /// Pause between attempts while no stream can be opened.
    pub retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(500),
            stall: Duration::from_secs(3),
            retry: Duration::from_secs(1),
        }
    }
}

/// A running capture thread. Dropping it stops capture.
pub(crate) struct Capture {
    tx: mpsc::Sender<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    /// Spawns the capture thread and waits for the first stream to open.
    ///
    /// `make_backend` runs on the capture thread, so neither the backend nor
    /// its streams need to be `Send`. On success `state` reads as running
    /// until the thread ends; errors after this point go to `state`.
    pub(crate) fn start<B, F>(
        thread_name: &str,
        make_backend: F,
        speaker: Speaker,
        start: Instant,
        sink: FrameSender,
        state: Arc<SourceState>,
        timing: Timing,
    ) -> Result<Self, AudioError>
    where
        B: Backend,
        F: FnOnce() -> B + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_tx = tx.clone();
        let thread = thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || {
                let mut backend = make_backend();
                let opened = backend.open(StreamEvents {
                    tx: thread_tx.clone(),
                    generation: 1,
                });
                let (opened, framer) = match opened.and_then(|opened| {
                    let framer = Framer::new(speaker, start, sink, opened.sample_rate, 1)?;
                    Ok((opened, framer))
                }) {
                    Ok(ready) => ready,
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                state.set_running(true);
                let _ = ready_tx.send(Ok(()));
                let now = Instant::now();
                Worker {
                    backend,
                    framer,
                    rx,
                    tx: thread_tx,
                    state: Arc::clone(&state),
                    timing,
                    generation: 1,
                    current: Some(opened),
                    last_data: now,
                    last_check: now,
                    next_attempt: now,
                    reopen: false,
                    done: false,
                    sink_closed: false,
                    last_reported: None,
                }
                .run();
                state.set_running(false);
            })
            .map_err(|e| AudioError::Other(format!("could not spawn capture thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioError::Other(
                    "capture thread ended before it started".to_owned(),
                ))
            }
        }
    }

    /// Stops capture and waits until the last frame is sent and the frame
    /// sender dropped.
    pub(crate) fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.tx.send(Event::Stop);
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker<B: Backend> {
    backend: B,
    framer: Framer,
    rx: mpsc::Receiver<Event>,
    tx: mpsc::Sender<Event>,
    state: Arc<SourceState>,
    timing: Timing,
    /// Number of the stream currently (or last) open.
    generation: u64,
    current: Option<OpenStream<B::Stream>>,
    last_data: Instant,
    last_check: Instant,
    next_attempt: Instant,
    /// The current stream must be replaced.
    reopen: bool,
    /// Capture is ending: stop was requested or nobody is listening.
    done: bool,
    /// The frame receiver is gone; nothing more can be delivered.
    sink_closed: bool,
    /// Last error queued during the current outage, so a failure repeated on
    /// every retry is reported once. Cleared when audio flows again.
    last_reported: Option<AudioError>,
}

impl<B: Backend> Worker<B> {
    fn run(mut self) {
        while !self.done {
            match self.rx.recv_timeout(self.timing.poll) {
                Ok(event) => self.handle(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.done {
                break;
            }
            let now = Instant::now();
            if now.duration_since(self.last_check) >= self.timing.poll {
                self.last_check = now;
                self.check_stream(now);
            }
            if (self.reopen || self.current.is_none()) && now >= self.next_attempt {
                self.open_stream(now);
            }
        }

        // Stop the callbacks, then deliver what they had already queued.
        self.current = None;
        self.drain();
        if !self.sink_closed {
            let _ = self.framer.finish();
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Stop => self.done = true,
            Event::Data {
                generation,
                mono,
                first_sample_at,
            } if generation == self.generation => self.on_data(&mono, first_sample_at),
            Event::StreamError { generation, error } if generation == self.generation => {
                self.report(error);
                self.reopen = true;
            }
            // From a stream that has been replaced.
            Event::Data { .. } | Event::StreamError { .. } => {}
        }
    }

    /// Handles everything already queued, without waiting. Called once the
    /// current stream is closed, so that its last buffers are not lost.
    fn drain(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.handle(event);
        }
    }

    fn on_data(&mut self, mono: &[f32], first_sample_at: Instant) {
        if self.sink_closed {
            return;
        }
        self.last_data = Instant::now();
        self.last_reported = None;
        match self.framer.push_mono(mono, first_sample_at) {
            Ok(()) => {}
            Err(AudioError::SinkClosed) => self.close_sink(),
            Err(err) => {
                self.report(err);
                self.reopen = true;
            }
        }
    }

    /// Nobody is listening any more: end capture.
    fn close_sink(&mut self) {
        self.state.report(AudioError::SinkClosed);
        self.sink_closed = true;
        self.done = true;
    }

    /// Notices a changed default device and a stream that went quiet.
    fn check_stream(&mut self, now: Instant) {
        let Some(current) = &self.current else {
            return;
        };
        if self.reopen {
            return;
        }
        if self.backend.current_device().as_deref() != Some(current.device.as_str()) {
            // Not an error: the user plugged something in or picked another
            // device. Follow it.
            self.reopen = true;
        } else if now.duration_since(self.last_data) >= self.timing.stall {
            let device = current.device.clone();
            self.report(AudioError::Stream(format!(
                "no audio from {device} for {:.1} s; re-opening",
                self.timing.stall.as_secs_f32()
            )));
            self.reopen = true;
        }
    }

    /// Replaces the current stream with one on the present default device.
    /// On failure the old stream is gone and the attempt repeats after
    /// `timing.retry` until it works or capture is stopped.
    fn open_stream(&mut self, now: Instant) {
        // Close the old stream first: some devices cannot be opened twice.
        self.current = None;
        self.drain();
        if self.done {
            return;
        }
        self.generation += 1;
        let events = StreamEvents {
            tx: self.tx.clone(),
            generation: self.generation,
        };
        let opened = self.backend.open(events).and_then(|opened| {
            self.framer.set_input_format(opened.sample_rate, 1)?;
            Ok(opened)
        });
        match opened {
            Ok(opened) => {
                self.current = Some(opened);
                self.reopen = false;
                self.last_data = now;
            }
            // Flushing the old stream found the receiver gone.
            Err(AudioError::SinkClosed) => self.close_sink(),
            Err(err) => {
                self.report(err);
                self.next_attempt = now + self.timing.retry;
            }
        }
    }

    fn report(&mut self, error: AudioError) {
        if self.last_reported.as_ref() != Some(&error) {
            self.last_reported = Some(error.clone());
            self.state.report(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use mneme_core::AudioFrame;

    use super::*;
    use crate::frame_channel;
    use crate::testutil::sine;

    /// What the test and the scripted backend share.
    #[derive(Default)]
    struct Script {
        /// The "system default device" and the rate it delivers.
        default_device: Option<(String, u32)>,
        /// Every device a stream was opened on, in order.
        opened: Vec<String>,
        /// How many streams are open right now.
        live_streams: usize,
        /// The newest stream's callback handle.
        events: Option<StreamEvents>,
        /// Number of upcoming `open` calls that fail.
        failing_opens: usize,
    }

    #[derive(Clone, Default)]
    struct ScriptedBackend(Arc<Mutex<Script>>);

    struct ScriptedStream(Arc<Mutex<Script>>);

    impl Drop for ScriptedStream {
        fn drop(&mut self) {
            self.0.lock().unwrap().live_streams -= 1;
        }
    }

    impl Backend for ScriptedBackend {
        type Stream = ScriptedStream;

        fn current_device(&mut self) -> Option<String> {
            let script = self.0.lock().unwrap();
            script.default_device.as_ref().map(|(name, _)| name.clone())
        }

        fn open(&mut self, events: StreamEvents) -> Result<OpenStream<ScriptedStream>, AudioError> {
            let mut script = self.0.lock().unwrap();
            if script.failing_opens > 0 {
                script.failing_opens -= 1;
                return Err(AudioError::Device("device is busy".to_owned()));
            }
            let (device, sample_rate) = script
                .default_device
                .clone()
                .ok_or(AudioError::NoInputDevice)?;
            script.opened.push(device.clone());
            script.live_streams += 1;
            script.events = Some(events);
            Ok(OpenStream {
                stream: ScriptedStream(Arc::clone(&self.0)),
                device,
                sample_rate,
            })
        }
    }

    impl ScriptedBackend {
        fn with_device(name: &str, rate: u32) -> Self {
            let backend = Self::default();
            backend.set_device(Some((name, rate)));
            backend
        }

        fn set_device(&self, device: Option<(&str, u32)>) {
            self.0.lock().unwrap().default_device = device.map(|(n, r)| (n.to_owned(), r));
        }

        fn events(&self) -> StreamEvents {
            self.0.lock().unwrap().events.clone().unwrap()
        }

        fn opened(&self) -> Vec<String> {
            self.0.lock().unwrap().opened.clone()
        }

        fn live_streams(&self) -> usize {
            self.0.lock().unwrap().live_streams
        }

        /// Plays `seconds` of a tone into the newest stream as stereo
        /// callbacks of 10 ms, the first captured at `from_ms` meeting time.
        fn play(&self, start: Instant, from_ms: u64, rate: u32, seconds: f64) {
            let events = self.events();
            let mono = sine(440.0, rate, (f64::from(rate) * seconds) as usize);
            let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
            let per_callback = rate as usize / 100;
            for (i, chunk) in stereo.chunks(per_callback * 2).enumerate() {
                let at = start + Duration::from_millis(from_ms + 10 * i as u64);
                events.data(chunk, 2, at);
            }
        }
    }

    fn fast() -> Timing {
        Timing {
            poll: Duration::from_millis(5),
            stall: Duration::from_secs(60),
            retry: Duration::from_millis(5),
        }
    }

    fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn start_capture(
        backend: &ScriptedBackend,
        start: Instant,
        timing: Timing,
    ) -> (
        Result<Capture, AudioError>,
        Arc<SourceState>,
        crate::FrameReceiver,
    ) {
        let (tx, rx) = frame_channel();
        let state = SourceState::new();
        let thread_backend = backend.clone();
        let capture = Capture::start(
            "test-capture",
            move || thread_backend,
            Speaker::Me,
            start,
            tx,
            Arc::clone(&state),
            timing,
        );
        (capture, state, rx)
    }

    fn total_samples(frames: &[AudioFrame]) -> usize {
        frames.iter().map(|f| f.samples.len()).sum()
    }

    #[test]
    fn frames_flow_from_callbacks_to_the_sink() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("built-in", 48_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        let mut capture = capture.unwrap();
        assert!(state.is_running());

        backend.play(start, 40, 48_000, 1.0);
        capture.stop();

        assert!(!state.is_running());
        assert_eq!(backend.live_streams(), 0);
        // stop() dropped the sender, so this ends.
        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert_eq!(total_samples(&frames), 16_000);
        assert_eq!(frames[0].start_ms, 40);
        for pair in frames.windows(2) {
            assert_eq!(pair[1].start_ms, pair[0].start_ms + 20);
        }
        assert!(state.take_errors().is_empty());
    }

    #[test]
    fn a_failed_first_open_fails_start() {
        let backend = ScriptedBackend::default();
        let (capture, state, rx) = start_capture(&backend, Instant::now(), fast());
        assert_eq!(capture.err(), Some(AudioError::NoInputDevice));
        assert!(!state.is_running());
        // The sender is gone, so a consumer is not left waiting.
        assert!(rx.recv().is_err());
    }

    #[test]
    fn a_new_default_device_is_followed_and_the_clock_kept() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("built-in", 48_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        let mut capture = capture.unwrap();

        backend.play(start, 0, 48_000, 0.5);
        let old_events = backend.events();

        // Headphones with a 44.1 kHz microphone become the default input.
        backend.set_device(Some(("headset", 44_100)));
        wait_until("the re-open", || backend.opened().len() == 2);
        assert_eq!(backend.opened(), ["built-in", "headset"]);
        assert_eq!(backend.live_streams(), 1);

        // A late callback from the closed stream must not reach the sink.
        old_events.data(&[1.0f32; 960], 2, start + Duration::from_millis(600));
        backend.play(start, 800, 44_100, 0.5);
        capture.stop();

        let frames: Vec<AudioFrame> = rx.iter().collect();
        let split = frames.iter().position(|f| f.start_ms >= 800).unwrap();
        let (before, after) = frames.split_at(split);
        assert_eq!(before[0].start_ms, 0);
        assert_eq!(total_samples(before), 8_000);
        assert_eq!(after[0].start_ms, 800);
        assert_eq!(total_samples(after), 8_000);
        // Nothing from the stale callback: no frame lies between the streams.
        assert_eq!(before.last().unwrap().start_ms, 480);
        // Following a device is not an error.
        assert!(state.take_errors().is_empty());
    }

    #[test]
    fn a_stream_error_is_reported_and_the_stream_reopened() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("usb-mic", 48_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        let mut capture = capture.unwrap();

        backend.play(start, 0, 48_000, 0.2);
        backend
            .events()
            .error(AudioError::Stream("device disconnected".to_owned()));
        wait_until("the re-open", || backend.opened().len() == 2);
        assert!(state.is_running());

        backend.play(start, 1000, 48_000, 0.2);
        capture.stop();

        assert_eq!(
            state.take_errors(),
            [AudioError::Stream("device disconnected".to_owned())]
        );
        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert_eq!(total_samples(&frames), 6_400);
        assert_eq!(frames.last().unwrap().start_ms, 1180);
    }

    #[test]
    fn capture_survives_a_spell_with_no_device() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("usb-mic", 48_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        let mut capture = capture.unwrap();
        backend.play(start, 0, 48_000, 0.1);

        // The only microphone is unplugged. Many retries, one report.
        backend.set_device(None);
        wait_until("the failed re-open", || backend.live_streams() == 0);
        thread::sleep(Duration::from_millis(50));
        assert!(state.is_running());
        assert_eq!(state.take_errors(), [AudioError::NoInputDevice]);

        // It comes back, busy at first.
        {
            let mut script = backend.0.lock().unwrap();
            script.failing_opens = 2;
            script.default_device = Some(("usb-mic".to_owned(), 48_000));
        }
        wait_until("the re-open", || backend.opened().len() == 2);
        backend.play(start, 3000, 48_000, 0.1);
        capture.stop();

        assert_eq!(
            state.take_errors(),
            [AudioError::Device("device is busy".to_owned())]
        );
        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert_eq!(total_samples(&frames), 3_200);
        assert_eq!(frames.last().unwrap().start_ms, 3080);
    }

    #[test]
    fn a_silent_stream_is_reopened() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("built-in", 48_000);
        let timing = Timing {
            stall: Duration::from_millis(40),
            ..fast()
        };
        let (capture, state, _rx) = start_capture(&backend, start, timing);
        let mut capture = capture.unwrap();

        // No callbacks at all: the stream is replaced, and replaced again.
        wait_until("two re-opens", || backend.opened().len() >= 3);
        assert_eq!(backend.live_streams(), 1);
        capture.stop();

        // Reported once, not once per attempt.
        let errors = state.take_errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(matches!(&errors[0], AudioError::Stream(m) if m.contains("no audio")));
    }

    #[test]
    fn capture_ends_when_the_receiver_is_dropped() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("built-in", 16_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        let _capture = capture.unwrap();
        drop(rx);
        backend.play(start, 0, 16_000, 0.1);
        wait_until("the capture thread to end", || !state.is_running());
        assert_eq!(backend.live_streams(), 0);
        assert_eq!(state.take_errors(), [AudioError::SinkClosed]);
    }

    #[test]
    fn dropping_the_handle_stops_capture() {
        let start = Instant::now();
        let backend = ScriptedBackend::with_device("built-in", 16_000);
        let (capture, state, rx) = start_capture(&backend, start, fast());
        backend.play(start, 0, 16_000, 0.05);
        drop(capture.unwrap());
        assert!(!state.is_running());
        assert_eq!(backend.live_streams(), 0);
        assert_eq!(total_samples(&rx.iter().collect::<Vec<_>>()), 800);
    }
}
