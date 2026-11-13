//! Signal helpers for unit tests.

use std::f64::consts::TAU;

/// A full-scale sine starting at phase zero.
pub(crate) fn sine(freq: f64, rate: u32, len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| (TAU * freq * i as f64 / f64::from(rate)).sin() as f32)
        .collect()
}

/// Frequency of a tone from its rising zero crossings, each located by linear
/// interpolation between the two samples around it.
pub(crate) fn zero_crossing_frequency(samples: &[f32], rate: u32) -> f64 {
    let mut crossings = Vec::new();
    for (i, pair) in samples.windows(2).enumerate() {
        let (a, b) = (f64::from(pair[0]), f64::from(pair[1]));
        if a < 0.0 && b >= 0.0 {
            crossings.push(i as f64 + a / (a - b));
        }
    }
    assert!(crossings.len() >= 2, "not enough zero crossings");
    let cycles = (crossings.len() - 1) as f64;
    cycles * f64::from(rate) / (crossings[crossings.len() - 1] - crossings[0])
}

/// Result of fitting `amplitude * sin(2π·freq·t + phase)` to a signal.
pub(crate) struct SineFit {
    pub amplitude: f64,
    /// Radians, relative to a sine that starts at phase zero at `t = 0`.
    pub phase: f64,
    /// RMS of what is left after subtracting the fitted sine.
    pub residual_rms: f64,
}

/// Least-squares fit of a sine of known frequency. `offset` is the index of
/// `samples[0]` in the original signal, so the phase refers to the original
/// time origin.
pub(crate) fn fit_sine(samples: &[f32], freq: f64, rate: u32, offset: usize) -> SineFit {
    let w = TAU * freq / f64::from(rate);
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, x) in samples.iter().enumerate() {
        let (s, c) = (w * (offset + i) as f64).sin_cos();
        let x = f64::from(*x);
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += x * s;
        xc += x * c;
    }
    // Solve [ss sc; sc cc] [a b]' = [xs xc]'.
    let det = ss * cc - sc * sc;
    let a = (xs * cc - xc * sc) / det;
    let b = (xc * ss - xs * sc) / det;
    let mut residual = 0.0;
    for (i, x) in samples.iter().enumerate() {
        let (s, c) = (w * (offset + i) as f64).sin_cos();
        residual += (f64::from(*x) - a * s - b * c).powi(2);
    }
    SineFit {
        amplitude: a.hypot(b),
        phase: b.atan2(a),
        residual_rms: (residual / samples.len() as f64).sqrt(),
    }
}

/// What a test sees of, and does to, a [`GatedSource`].
#[derive(Clone, Default)]
pub(crate) struct Probe(std::sync::Arc<std::sync::Mutex<ProbeState>>);

#[derive(Default)]
struct ProbeState {
    /// `start` may return. Set by [`Probe::release`].
    released: bool,
    panic_on_start: Option<String>,
    start_calls: usize,
    stop_calls: usize,
    start_instant: Option<std::time::Instant>,
    start_entered_at: Option<std::time::Instant>,
    start_returned_at: Option<std::time::Instant>,
    injected: Vec<crate::AudioError>,
    /// How long `stop` takes after the source has stopped delivering.
    stop_takes: std::time::Duration,
}

impl Probe {
    fn lock(&self) -> std::sync::MutexGuard<'_, ProbeState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lets a blocked `start` return.
    pub(crate) fn release(&self) {
        self.lock().released = true;
    }

    /// Makes `start` panic with `message` instead of starting.
    pub(crate) fn panic_on_start(&self, message: &str) {
        self.lock().panic_on_start = Some(message.to_owned());
    }

    /// Queues an error as a running source would after recovering from it.
    pub(crate) fn inject_error(&self, error: crate::AudioError) {
        self.lock().injected.push(error);
    }

    /// Makes `stop` slow, like a device that takes its time to close.
    pub(crate) fn stop_takes(&self, time: std::time::Duration) {
        self.lock().stop_takes = time;
    }

    pub(crate) fn start_calls(&self) -> usize {
        self.lock().start_calls
    }

    pub(crate) fn stop_calls(&self) -> usize {
        self.lock().stop_calls
    }

    /// The shared start instant the source was given.
    pub(crate) fn start_instant(&self) -> Option<std::time::Instant> {
        self.lock().start_instant
    }

    pub(crate) fn start_entered_at(&self) -> Option<std::time::Instant> {
        self.lock().start_entered_at
    }

    pub(crate) fn start_returned_at(&self) -> Option<std::time::Instant> {
        self.lock().start_returned_at
    }
}

/// Wraps a source so that a test can hold its `start` back (like a source
/// waiting on a permission prompt), make it panic, and see how it was called.
pub(crate) struct GatedSource {
    inner: crate::FakeSource,
    probe: Probe,
}

impl GatedSource {
    /// `start` blocks until [`Probe::release`].
    pub(crate) fn closed(inner: crate::FakeSource, probe: &Probe) -> Self {
        Self {
            inner,
            probe: probe.clone(),
        }
    }

    /// `start` does not block.
    pub(crate) fn open(inner: crate::FakeSource, probe: &Probe) -> Self {
        probe.release();
        Self::closed(inner, probe)
    }
}

impl crate::AudioSource for GatedSource {
    fn speaker(&self) -> mneme_core::Speaker {
        self.inner.speaker()
    }

    fn start(
        &mut self,
        start: std::time::Instant,
        sink: crate::FrameSender,
    ) -> Result<(), crate::AudioError> {
        {
            let mut probe = self.probe.lock();
            probe.start_calls += 1;
            probe.start_instant = Some(start);
            probe.start_entered_at = Some(std::time::Instant::now());
            if let Some(message) = probe.panic_on_start.clone() {
                drop(probe);
                panic!("{message}");
            }
        }
        while !self.probe.lock().released {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let result = self.inner.start(start, sink);
        self.probe.lock().start_returned_at = Some(std::time::Instant::now());
        result
    }

    fn stop(&mut self) {
        self.inner.stop();
        let wait = self.probe.lock().stop_takes;
        std::thread::sleep(wait);
        self.probe.lock().stop_calls += 1;
    }

    fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    fn take_errors(&mut self) -> Vec<crate::AudioError> {
        let mut errors = std::mem::take(&mut self.probe.lock().injected);
        errors.extend(self.inner.take_errors());
        errors
    }
}

/// A source that delivers exactly the frames it was given, at once, then
/// ends. For timelines a [`FakeSource`](crate::FakeSource) cannot produce:
/// gaps, short frames, overlaps.
pub(crate) struct FrameListSource {
    speaker: mneme_core::Speaker,
    frames: Vec<mneme_core::AudioFrame>,
}

impl FrameListSource {
    pub(crate) fn new(speaker: mneme_core::Speaker, frames: Vec<mneme_core::AudioFrame>) -> Self {
        Self { speaker, frames }
    }
}

impl crate::AudioSource for FrameListSource {
    fn speaker(&self) -> mneme_core::Speaker {
        self.speaker
    }

    fn start(
        &mut self,
        _start: std::time::Instant,
        sink: crate::FrameSender,
    ) -> Result<(), crate::AudioError> {
        for frame in self.frames.drain(..) {
            let _ = sink.send(frame);
        }
        Ok(())
    }

    fn stop(&mut self) {}

    fn is_running(&self) -> bool {
        false
    }

    fn take_errors(&mut self) -> Vec<crate::AudioError> {
        Vec::new()
    }
}
