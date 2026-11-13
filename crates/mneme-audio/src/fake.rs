//! A source that plays samples from memory or a WAV file, for tests in this
//! crate and in the crates that consume audio. It needs no device and no
//! permission.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mneme_core::{SAMPLE_RATE, Speaker};

use crate::source::SourceState;
use crate::wav::read_wav_16k_mono;
use crate::{AudioError, AudioSource, FRAME_SAMPLES, FrameSender, Framer};

/// How fast a [`FakeSource`] delivers its frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pacing {
    /// As fast as the consumer takes them. Timestamps are the same as in
    /// real time; only the waiting is skipped.
    #[default]
    Fast,
    /// Each frame is delivered when a live device would deliver it: once the
    /// wall clock reaches the end of the frame, measured from the start
    /// instant.
    RealTime,
}

/// An [`AudioSource`] that plays a fixed buffer of 16 kHz mono samples.
///
/// Frames come out exactly as from a live source: [`FRAME_SAMPLES`] long,
/// contiguous, labeled with the given speaker. Timestamps are deterministic:
/// the first frame starts at the configured offset (zero by default) and each
/// one follows the last, whatever the pacing and however late `start` is
/// called. The source stops by itself after the last sample.
///
/// ```
/// use std::time::Instant;
/// use mneme_audio::{AudioSource, FakeSource, frame_channel};
/// use mneme_core::Speaker;
///
/// let mut source = FakeSource::from_samples(Speaker::Them, vec![0.1; 16_000]);
/// let (tx, rx) = frame_channel();
/// source.start(Instant::now(), tx).unwrap();
/// let frames: Vec<_> = rx.iter().collect(); // ends when the source is done
/// assert_eq!(frames.len(), 50);
/// assert_eq!(frames[49].start_ms, 980);
/// ```
pub struct FakeSource {
    speaker: Speaker,
    samples: Arc<Vec<f32>>,
    pacing: Pacing,
    offset: Duration,
    fail_at: Option<(usize, AudioError)>,
    start_error: Option<AudioError>,
    state: Arc<SourceState>,
    worker: Option<Worker>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    /// Set by the thread when it has delivered its last frame, just before
    /// it drops the frame sender.
    done: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl FakeSource {
    /// A source that plays `samples` (16 kHz mono) as `speaker`.
    pub fn from_samples(speaker: Speaker, samples: Vec<f32>) -> Self {
        Self {
            speaker,
            samples: Arc::new(samples),
            pacing: Pacing::default(),
            offset: Duration::ZERO,
            fail_at: None,
            start_error: None,
            state: SourceState::new(),
            worker: None,
        }
    }

    /// A source that plays a WAV file as `speaker`. The file is read now and
    /// converted to 16 kHz mono whatever its rate, channels and sample format.
    pub fn from_wav(speaker: Speaker, path: impl AsRef<Path>) -> Result<Self, AudioError> {
        Ok(Self::from_samples(speaker, read_wav_16k_mono(path)?))
    }

    /// Sets the pacing. The default is [`Pacing::Fast`].
    pub fn with_pacing(mut self, pacing: Pacing) -> Self {
        self.pacing = pacing;
        self
    }

    /// Makes the audio begin `offset` into the meeting instead of at zero,
    /// like a source that came up late. Rounded up to a whole millisecond.
    pub fn starting_at(mut self, offset: Duration) -> Self {
        self.offset = offset;
        self
    }

    /// Makes the source die after delivering `played` of audio: it queues
    /// `error`, stops, and reports not running.
    pub fn failing_after(mut self, played: Duration, error: AudioError) -> Self {
        let samples = (played.as_micros() * u128::from(SAMPLE_RATE) / 1_000_000) as usize;
        self.fail_at = Some((samples, error));
        self
    }

    /// Makes [`start`](AudioSource::start) fail with `error`.
    pub fn failing_to_start(mut self, error: AudioError) -> Self {
        self.start_error = Some(error);
        self
    }

    /// Length of the audio this source plays.
    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.samples.len() as u64 * 1_000_000 / u64::from(SAMPLE_RATE))
    }

    /// Blocks until the source has played everything (or failed) and dropped
    /// its frame sender. Returns at once if it is not running.
    pub fn wait_until_finished(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.thread.join();
        }
    }
}

impl std::fmt::Debug for FakeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSource")
            .field("speaker", &self.speaker)
            .field("samples", &self.samples.len())
            .field("pacing", &self.pacing)
            .field("offset", &self.offset)
            .field("running", &self.is_running())
            .finish()
    }
}

impl AudioSource for FakeSource {
    fn speaker(&self) -> Speaker {
        self.speaker
    }

    fn start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError> {
        // The worker drops its frame sender a moment before it reports
        // stopped. A consumer that saw the channel end may call `start` in
        // between; that is a finished source, not a running one.
        let finishing = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.done.load(Ordering::SeqCst));
        if self.is_running() && !finishing {
            return Err(AudioError::AlreadyRunning);
        }
        self.wait_until_finished();
        if let Some(error) = &self.start_error {
            return Err(error.clone());
        }

        let framer = Framer::new(self.speaker, start, sink, SAMPLE_RATE, 1)?;
        let playback = Playback {
            framer,
            start,
            samples: Arc::clone(&self.samples),
            pacing: self.pacing,
            offset: self.offset,
            fail_at: self.fail_at.clone(),
            state: Arc::clone(&self.state),
            stop: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
        };
        let stop = Arc::clone(&playback.stop);
        let done = Arc::clone(&playback.done);
        self.state.set_running(true);
        let thread = thread::Builder::new()
            .name(format!("mneme-fake-{}", self.speaker))
            .spawn(move || playback.run())
            .map_err(|e| {
                self.state.set_running(false);
                AudioError::Other(format!("could not spawn fake source thread: {e}"))
            })?;
        self.worker = Some(Worker { stop, done, thread });
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop.store(true, Ordering::SeqCst);
            let _ = worker.thread.join();
        }
    }

    fn is_running(&self) -> bool {
        self.state.is_running()
    }

    fn take_errors(&mut self) -> Vec<AudioError> {
        self.state.take_errors()
    }
}

impl Drop for FakeSource {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Playback {
    framer: Framer,
    start: Instant,
    samples: Arc<Vec<f32>>,
    pacing: Pacing,
    offset: Duration,
    fail_at: Option<(usize, AudioError)>,
    state: Arc<SourceState>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Playback {
    fn run(mut self) {
        let end = match &self.fail_at {
            Some((at, _)) => (*at).min(self.samples.len()),
            None => self.samples.len(),
        };
        let samples = Arc::clone(&self.samples);
        let mut pos = 0;
        let mut delivered = true;
        while pos < end {
            let next = (pos + FRAME_SAMPLES).min(end);
            // Meeting time at which the chunk begins and ends.
            let begins = self.start + self.offset + samples_to_duration(pos);
            let ends = self.start + self.offset + samples_to_duration(next);
            if self.pacing == Pacing::RealTime && !self.sleep_until(ends) {
                break;
            }
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            if let Err(err) = self.framer.push_mono(&samples[pos..next], begins) {
                self.state.report(err);
                delivered = false;
                break;
            }
            pos = next;
        }
        if delivered {
            let _ = self.framer.finish();
        }
        if pos >= end
            && let Some((_, error)) = self.fail_at.take()
        {
            self.state.report(error);
        }
        // Drop the frame sender before reporting stopped, so a consumer that
        // sees `is_running() == false` also sees the channel end. `done`
        // covers the moment in between (see `start`).
        self.done.store(true, Ordering::SeqCst);
        drop(self.framer);
        self.state.set_running(false);
    }

    /// Sleeps until `deadline` in short steps. Returns `false` if stop was
    /// requested meanwhile.
    fn sleep_until(&self, deadline: Instant) -> bool {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            thread::sleep(left.min(Duration::from_millis(10)));
        }
    }
}

fn samples_to_duration(samples: usize) -> Duration {
    Duration::from_micros(samples as u64 * 1_000_000 / u64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use mneme_core::AudioFrame;

    use super::*;
    use crate::testutil::sine;
    use crate::wav::WavWriter;
    use crate::{FRAME_MS, frame_channel};

    fn collect(source: &mut FakeSource, start: Instant) -> Vec<AudioFrame> {
        let (tx, rx) = frame_channel();
        source.start(start, tx).unwrap();
        // Ends when the source drops its sender.
        rx.iter().collect()
    }

    fn concat(frames: &[AudioFrame]) -> Vec<f32> {
        frames.iter().flat_map(|f| f.samples.clone()).collect()
    }

    #[test]
    fn fake_source_drives_a_sink_end_to_end() {
        // 1.21 s: sixty full frames and a half one.
        let samples = sine(440.0, 16_000, 19_360);
        let mut source = FakeSource::from_samples(Speaker::Them, samples.clone());
        assert_eq!(source.speaker(), Speaker::Them);
        assert_eq!(source.duration(), Duration::from_millis(1210));
        assert!(!source.is_running());

        let frames = collect(&mut source, Instant::now());
        source.wait_until_finished();

        assert!(!source.is_running());
        assert!(source.take_errors().is_empty());
        assert_eq!(frames.len(), 61);
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.speaker, Speaker::Them);
            assert_eq!(frame.start_ms, i as u64 * FRAME_MS);
        }
        assert!(
            frames[..60]
                .iter()
                .all(|f| f.samples.len() == FRAME_SAMPLES)
        );
        assert_eq!(frames[60].samples.len(), 160);
        // The audio arrives untouched.
        assert_eq!(concat(&frames), samples);
    }

    #[test]
    fn fake_source_feeds_a_wav_writer_through_the_trait() {
        // The path the recorder takes: Box<dyn AudioSource> -> channel -> WAV.
        let dir = std::env::temp_dir().join(format!("mneme-audio-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input-48k-stereo.wav");
        let output = dir.join("output.wav");

        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec).unwrap();
        for s in sine(1_000.0, 48_000, 48_000) {
            let v = (s * 16_000.0) as i16;
            writer.write_sample(v).unwrap();
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let mut source: Box<dyn AudioSource> =
            Box::new(FakeSource::from_wav(Speaker::Me, &input).unwrap());
        let (tx, rx) = frame_channel();
        source.start(Instant::now(), tx).unwrap();
        let mut wav = WavWriter::create(&output).unwrap();
        let mut next_ms = 0;
        for frame in rx {
            assert_eq!(frame.speaker, Speaker::Me);
            assert_eq!(frame.start_ms, next_ms);
            next_ms += FRAME_MS;
            wav.write(&frame.samples).unwrap();
        }
        source.stop();
        assert!(!source.is_running());
        assert_eq!(wav.samples_written(), 16_000);
        wav.finalize().unwrap();

        let recorded = read_wav_16k_mono(&output).unwrap();
        assert_eq!(recorded.len(), 16_000);
        let freq = crate::testutil::zero_crossing_frequency(&recorded[800..15_200], 16_000);
        assert!((freq - 1_000.0).abs() < 1.0, "tone came out at {freq} Hz");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_offset_shifts_every_timestamp() {
        let mut source = FakeSource::from_samples(Speaker::Me, vec![0.5; 960])
            .starting_at(Duration::from_millis(1500));
        let starts: Vec<u64> = collect(&mut source, Instant::now())
            .iter()
            .map(|f| f.start_ms)
            .collect();
        assert_eq!(starts, [1500, 1520, 1540]);
    }

    #[test]
    fn timestamps_do_not_depend_on_when_start_is_called() {
        // The shared instant is 300 ms old by the time this source starts.
        let start = Instant::now() - Duration::from_millis(300);
        let mut source = FakeSource::from_samples(Speaker::Me, vec![0.5; 640]);
        let starts: Vec<u64> = collect(&mut source, start)
            .iter()
            .map(|f| f.start_ms)
            .collect();
        assert_eq!(starts, [0, 20]);
    }

    #[test]
    fn real_time_pacing_takes_as_long_as_the_audio() {
        // 200 ms of audio beginning 100 ms into the meeting.
        let mut source = FakeSource::from_samples(Speaker::Them, vec![0.1; 3_200])
            .with_pacing(Pacing::RealTime)
            .starting_at(Duration::from_millis(100));
        let start = Instant::now();
        let (tx, rx) = frame_channel();
        source.start(start, tx).unwrap();

        let mut frames = Vec::new();
        for frame in rx {
            // No frame is delivered before its audio would have ended.
            let ends = Duration::from_millis(frame.start_ms + FRAME_MS);
            assert!(
                start.elapsed() >= ends,
                "frame at {} ms came early",
                frame.start_ms
            );
            frames.push(frame);
        }
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(300), "took {elapsed:?}");
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
        assert_eq!(frames.len(), 10);
        assert_eq!(frames[0].start_ms, 100);
        assert_eq!(frames[9].start_ms, 280);
    }

    #[test]
    fn stop_interrupts_real_time_playback() {
        // A minute of audio; stop after a moment.
        let mut source =
            FakeSource::from_samples(Speaker::Me, vec![0.0; 960_000]).with_pacing(Pacing::RealTime);
        let (tx, rx) = frame_channel();
        let begun = Instant::now();
        source.start(begun, tx).unwrap();
        assert!(source.is_running());
        assert_eq!(
            source.start(begun, frame_channel().0),
            Err(AudioError::AlreadyRunning)
        );
        let first = rx.recv().unwrap();
        assert_eq!(first.start_ms, 0);
        source.stop();
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert!(!source.is_running());
        // The sender is gone: the stream ends instead of blocking.
        let rest: Vec<AudioFrame> = rx.iter().collect();
        assert!(rest.len() < 250);
        assert!(source.take_errors().is_empty());
    }

    #[test]
    fn a_source_told_to_fail_stops_and_reports_it() {
        let error = AudioError::Other("permission revoked".to_owned());
        let mut source = FakeSource::from_samples(Speaker::Them, vec![0.3; 16_000])
            .failing_after(Duration::from_millis(250), error.clone());
        let frames = collect(&mut source, Instant::now());
        source.wait_until_finished();

        assert!(!source.is_running());
        assert_eq!(source.take_errors(), [error]);
        assert!(source.take_errors().is_empty());
        // Exactly the audio before the failure was delivered.
        assert_eq!(concat(&frames).len(), 4_000);
        assert_eq!(frames.last().unwrap().start_ms, 240);
    }

    #[test]
    fn a_source_told_not_to_start_returns_the_error() {
        let mut source = FakeSource::from_samples(Speaker::Me, vec![0.0; 320])
            .failing_to_start(AudioError::NoInputDevice);
        let (tx, rx) = frame_channel();
        assert_eq!(
            source.start(Instant::now(), tx),
            Err(AudioError::NoInputDevice)
        );
        assert!(!source.is_running());
        assert!(rx.recv().is_err());
    }

    #[test]
    fn a_dropped_receiver_ends_the_source() {
        let mut source = FakeSource::from_samples(Speaker::Me, vec![0.0; 16_000]);
        let (tx, rx) = frame_channel();
        drop(rx);
        source.start(Instant::now(), tx).unwrap();
        source.wait_until_finished();
        assert!(!source.is_running());
        assert_eq!(source.take_errors(), [AudioError::SinkClosed]);
    }

    #[test]
    fn a_finished_source_can_be_started_again() {
        let mut source = FakeSource::from_samples(Speaker::Me, vec![0.2; 640]);
        let first = collect(&mut source, Instant::now());
        let second = collect(&mut source, Instant::now());
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn two_sources_share_one_channel_and_one_clock() {
        // What the recorder does: one sender cloned into both sources.
        let start = Instant::now();
        let mut me = FakeSource::from_samples(Speaker::Me, vec![0.1; 3_200]);
        let mut them = FakeSource::from_samples(Speaker::Them, vec![0.2; 1_600])
            .starting_at(Duration::from_millis(60));
        let (tx, rx) = frame_channel();
        me.start(start, tx.clone()).unwrap();
        them.start(start, tx).unwrap();

        let frames: Vec<AudioFrame> = rx.iter().collect();
        let starts = |speaker| -> Vec<u64> {
            frames
                .iter()
                .filter(|f| f.speaker == speaker)
                .map(|f| f.start_ms)
                .collect()
        };
        assert_eq!(
            starts(Speaker::Me),
            (0..10).map(|i| i * 20).collect::<Vec<_>>()
        );
        assert_eq!(
            starts(Speaker::Them),
            (0..5).map(|i| 60 + i * 20).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_empty_source_finishes_at_once() {
        let mut source = FakeSource::from_samples(Speaker::Me, Vec::new());
        assert!(collect(&mut source, Instant::now()).is_empty());
        source.wait_until_finished();
        assert!(!source.is_running());
    }

    #[test]
    fn a_missing_wav_file_is_an_error() {
        assert!(matches!(
            FakeSource::from_wav(Speaker::Me, "/nonexistent/file.wav"),
            Err(AudioError::Wav(_))
        ));
    }
}
