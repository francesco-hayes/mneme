//! The meeting clock: turns raw device buffers into stamped 16 kHz mono
//! [`AudioFrame`]s. See [`Framer`].

use std::time::{Duration, Instant};

use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker};

use crate::convert::{StreamResampler, ToF32, downmix_interleaved, downmix_planar};
use crate::{AudioError, FrameSender};

/// Length of the frames sources deliver, in milliseconds.
pub const FRAME_MS: u64 = 20;

/// Length of the frames sources deliver, in samples at 16 kHz. Only the last
/// frame before a gap or the end of a stream may be shorter.
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE as u64 * FRAME_MS / 1000) as usize;

const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;

/// How far behind the wall clock the sample clock may fall before the stream
/// is re-anchored. Far above callback jitter, far below a gap worth hiding.
const STALL_THRESHOLD: Duration = Duration::from_millis(500);

/// Where the running stream is pinned to the wall clock.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    /// Meeting time at which the stream's first input sample was captured.
    at_us: u64,
    /// Input frames received since then.
    input_frames: u64,
}

/// The meeting clock: converts device audio to 16 kHz mono frames, stamps
/// them in meeting time and sends them to a frame channel.
///
/// Every source (microphone, system audio, the fake) owns one `Framer` and
/// pushes whatever its device delivers into it. The framer converts
/// (downmix, resample), cuts the result into frames of [`FRAME_SAMPLES`] and
/// sends them with `start_ms` on the meeting clock.
///
/// # How frames are timed
///
/// Meeting time is measured from the shared start `Instant` both sources are
/// given. The wall clock is read only to **anchor** a stream: the capture time
/// of the first buffer after the stream opens fixes the meeting time of its
/// first sample (rounded up to a whole millisecond). From then on timestamps
/// are **sample counts**: each frame starts exactly where the previous one
/// ended, so `start_ms` advances by [`FRAME_MS`] per frame no matter when the
/// callbacks actually ran.
///
/// A stream is re-anchored, leaving a gap in `start_ms` that matches the gap
/// in the audio, in two cases:
///
/// - the source re-opened its device and called
///   [`set_input_format`](Framer::set_input_format);
/// - a buffer arrives more than half a second later than the sample count
///   says it should (the device stalled, the machine slept, or a slow device
///   clock has drifted that far).
///
/// Timestamps never go backwards: a re-anchor lands at or after the end of
/// the audio already delivered.
///
/// The sample clock is the device's clock. A device running fast relative to
/// the system clock is not corrected (that would need overlapping
/// timestamps), so over a long meeting the two channels can drift apart by
/// the difference between their device clocks.
#[derive(Debug)]
pub struct Framer {
    speaker: Speaker,
    start: Instant,
    sink: FrameSender,
    channels: usize,
    resampler: StreamResampler,
    anchor: Option<Anchor>,
    /// Meeting-clock position, in 16 kHz samples since `start`, of the sample
    /// after the last one accepted (sent or pending).
    position: u64,
    /// Converted samples not yet sent; always fewer than [`FRAME_SAMPLES`]
    /// between calls.
    pending: Vec<f32>,
    mono: Vec<f32>,
    resampled: Vec<f32>,
}

impl Framer {
    /// Creates a framer for a device that delivers `channels` channels at
    /// `sample_rate` Hz.
    ///
    /// `start` is the meeting's shared start instant and `sink` receives the
    /// frames, labeled with `speaker`.
    pub fn new(
        speaker: Speaker,
        start: Instant,
        sink: FrameSender,
        sample_rate: u32,
        channels: u16,
    ) -> Result<Self, AudioError> {
        if channels == 0 {
            return Err(AudioError::UnsupportedFormat("zero channels".to_owned()));
        }
        Ok(Self {
            speaker,
            start,
            sink,
            channels: usize::from(channels),
            resampler: StreamResampler::new(sample_rate)?,
            anchor: None,
            position: 0,
            pending: Vec::with_capacity(2 * FRAME_SAMPLES),
            mono: Vec::new(),
            resampled: Vec::new(),
        })
    }

    /// The speaker label put on every frame.
    pub fn speaker(&self) -> Speaker {
        self.speaker
    }

    /// Meeting time, in milliseconds, up to which audio has been accepted.
    pub fn position_ms(&self) -> u64 {
        self.position / SAMPLES_PER_MS
    }

    /// Switches to a new device format after a re-open.
    ///
    /// Audio still inside the framer is flushed first (ending in a short
    /// frame), and the next buffer re-anchors the stream to the wall clock,
    /// so the meeting clock carries on across the gap. On error the framer
    /// keeps its previous format.
    pub fn set_input_format(&mut self, sample_rate: u32, channels: u16) -> Result<(), AudioError> {
        if channels == 0 {
            return Err(AudioError::UnsupportedFormat("zero channels".to_owned()));
        }
        let resampler = if sample_rate == self.resampler.input_rate() {
            None
        } else {
            Some(StreamResampler::new(sample_rate)?)
        };
        self.end_stream()?;
        if let Some(resampler) = resampler {
            self.resampler = resampler;
        }
        self.channels = usize::from(channels);
        Ok(())
    }

    /// Pushes one device buffer of interleaved frames in the current format.
    ///
    /// `first_sample_at` is when the first sample of the buffer was captured.
    /// It is used to anchor a new stream and to notice stalls; between those
    /// events timing comes from the sample count alone.
    ///
    /// Returns [`AudioError::SinkClosed`] once the frame receiver is gone.
    pub fn push_interleaved<T: ToF32>(
        &mut self,
        data: &[T],
        first_sample_at: Instant,
    ) -> Result<(), AudioError> {
        let mut mono = std::mem::take(&mut self.mono);
        mono.clear();
        downmix_interleaved(data, self.channels, &mut mono);
        let result = self.push_mono(&mono, first_sample_at);
        self.mono = mono;
        result
    }

    /// Pushes one device buffer delivered as one slice per channel. See
    /// [`push_interleaved`](Self::push_interleaved).
    pub fn push_planar<T: ToF32>(
        &mut self,
        channels: &[&[T]],
        first_sample_at: Instant,
    ) -> Result<(), AudioError> {
        let mut mono = std::mem::take(&mut self.mono);
        mono.clear();
        downmix_planar(channels, &mut mono);
        let result = self.push_mono(&mono, first_sample_at);
        self.mono = mono;
        result
    }

    /// Pushes one buffer that is already mono `f32` at the device rate. See
    /// [`push_interleaved`](Self::push_interleaved).
    pub fn push_mono(&mut self, mono: &[f32], first_sample_at: Instant) -> Result<(), AudioError> {
        if mono.is_empty() {
            return Ok(());
        }
        let at_us = first_sample_at
            .saturating_duration_since(self.start)
            .as_micros() as u64;
        let rate = u64::from(self.resampler.input_rate());

        if let Some(anchor) = self.anchor {
            let expected_us = anchor.at_us + anchor.input_frames * 1_000_000 / rate;
            if at_us > expected_us + STALL_THRESHOLD.as_micros() as u64 {
                self.end_stream()?;
            }
        }
        let anchor = self.anchor.get_or_insert_with(|| {
            // Start on a whole millisecond, at or after everything already
            // delivered.
            let wall = at_us.div_ceil(1000) * SAMPLES_PER_MS;
            let floor = self.position.div_ceil(SAMPLES_PER_MS) * SAMPLES_PER_MS;
            self.position = wall.max(floor);
            Anchor {
                at_us,
                input_frames: 0,
            }
        });
        anchor.input_frames += mono.len() as u64;

        let mut resampled = std::mem::take(&mut self.resampled);
        resampled.clear();
        let result = self
            .resampler
            .process(mono, &mut resampled)
            .and_then(|()| self.accept(&resampled));
        self.resampled = resampled;
        result
    }

    /// Flushes everything still inside the framer: the resampler's tail and a
    /// final frame shorter than [`FRAME_SAMPLES`]. Call it when the source
    /// stops. The framer stays usable; a later buffer starts a new stream.
    pub fn finish(&mut self) -> Result<(), AudioError> {
        self.end_stream()
    }

    /// Ends the running stream: flush, then forget the anchor so the next
    /// buffer re-anchors.
    fn end_stream(&mut self) -> Result<(), AudioError> {
        if self.anchor.take().is_none() {
            return Ok(());
        }
        let mut resampled = std::mem::take(&mut self.resampled);
        resampled.clear();
        let result = self
            .resampler
            .finish(&mut resampled)
            .and_then(|()| self.accept(&resampled));
        self.resampled = resampled;
        result?;

        if !self.pending.is_empty() {
            let samples = std::mem::take(&mut self.pending);
            let start_ms = (self.position - samples.len() as u64) / SAMPLES_PER_MS;
            self.send(start_ms, samples)?;
        }
        Ok(())
    }

    /// Takes converted samples onto the clock and sends every full frame.
    fn accept(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        self.pending.extend_from_slice(samples);
        self.position += samples.len() as u64;
        let mut sent = 0;
        while self.pending.len() - sent >= FRAME_SAMPLES {
            let start_ms = (self.position - (self.pending.len() - sent) as u64) / SAMPLES_PER_MS;
            let frame = self.pending[sent..sent + FRAME_SAMPLES].to_vec();
            sent += FRAME_SAMPLES;
            if let Err(err) = self.send(start_ms, frame) {
                self.pending.drain(..sent);
                return Err(err);
            }
        }
        self.pending.drain(..sent);
        Ok(())
    }

    fn send(&self, start_ms: u64, samples: Vec<f32>) -> Result<(), AudioError> {
        self.sink
            .send(AudioFrame {
                speaker: self.speaker,
                start_ms,
                samples,
            })
            .map_err(|_| AudioError::SinkClosed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::testutil::{sine, zero_crossing_frequency};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn frame_end_ms(frame: &AudioFrame) -> u64 {
        frame.start_ms + (frame.samples.len() as u64).div_ceil(SAMPLES_PER_MS)
    }

    /// Asserts that frames follow each other with no gap and no overlap.
    fn assert_contiguous(frames: &[AudioFrame]) {
        for pair in frames.windows(2) {
            assert_eq!(pair[0].samples.len(), FRAME_SAMPLES);
            assert_eq!(pair[1].start_ms, pair[0].start_ms + FRAME_MS);
        }
    }

    #[test]
    fn frame_constants_agree() {
        assert_eq!(FRAME_SAMPLES, 320);
        assert_eq!(FRAME_SAMPLES as u64, FRAME_MS * SAMPLES_PER_MS);
    }

    #[test]
    fn timestamps_count_samples_and_ignore_callback_jitter() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 48_000, 2).unwrap();

        // Three seconds of 48 kHz stereo in 512-frame callbacks. The first
        // buffer was captured 250 ms into the meeting; later capture times
        // wobble by up to 15 ms either way, as callback scheduling does.
        let mono = sine(440.0, 48_000, 144_000);
        let stereo: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
        let jitter_ms = [0i64, 9, -7, 15, -15, 3, -11, 6];
        for (i, chunk) in stereo.chunks(512 * 2).enumerate() {
            let nominal_us = 250_000 + (i as i64 * 512 * 1_000_000 / 48_000);
            let jitter_us = if i == 0 { 0 } else { jitter_ms[i % 8] * 1000 };
            let at = start + Duration::from_micros((nominal_us + jitter_us) as u64);
            framer.push_interleaved(chunk, at).unwrap();
        }
        framer.finish().unwrap();
        drop(framer);

        let frames: Vec<AudioFrame> = rx.iter().collect();
        assert!(frames.iter().all(|f| f.speaker == Speaker::Me));
        assert_eq!(frames[0].start_ms, 250);
        // Every frame but the last is full and starts where the previous ended.
        assert_contiguous(&frames);
        let total: usize = frames.iter().map(|f| f.samples.len()).sum();
        assert_eq!(total, 48_000);
        assert_eq!(frames.len(), 150);
        assert_eq!(frames[149].start_ms, 250 + 149 * FRAME_MS);

        let audio: Vec<f32> = frames.iter().flat_map(|f| f.samples.clone()).collect();
        let freq = zero_crossing_frequency(&audio[800..audio.len() - 800], 16_000);
        assert!((freq - 440.0).abs() < 1.0, "tone came out at {freq} Hz");
    }

    #[test]
    fn the_first_buffer_anchors_to_a_whole_millisecond() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Them, start, tx, 16_000, 1).unwrap();
        framer
            .push_mono(&[0.1; 640], start + Duration::from_micros(12_300))
            .unwrap();
        let frames: Vec<AudioFrame> = rx.try_iter().collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].start_ms, 13);
        assert_eq!(frames[1].start_ms, 33);
        assert_eq!(frames[0].speaker, Speaker::Them);
        assert_eq!(framer.position_ms(), 53);
    }

    #[test]
    fn a_buffer_captured_before_the_start_instant_anchors_at_zero() {
        let start = Instant::now() + ms(50);
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 16_000, 1).unwrap();
        framer.push_mono(&[0.0; 320], Instant::now()).unwrap();
        assert_eq!(rx.try_recv().unwrap().start_ms, 0);
    }

    #[test]
    fn finish_sends_the_tail_as_a_short_frame() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 16_000, 1).unwrap();
        framer.push_mono(&[0.5; 400], start).unwrap();
        framer.finish().unwrap();
        // A second finish has nothing left to send.
        framer.finish().unwrap();
        let frames: Vec<AudioFrame> = rx.try_iter().collect();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].start_ms, frames[0].samples.len()), (0, 320));
        assert_eq!((frames[1].start_ms, frames[1].samples.len()), (20, 80));
    }

    #[test]
    fn reopening_on_a_new_format_keeps_the_meeting_clock() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 48_000, 2).unwrap();

        // One second from the first device, captured from 100 ms on.
        let first: Vec<f32> = sine(440.0, 48_000, 48_000)
            .iter()
            .flat_map(|s| [*s, *s])
            .collect();
        for (i, chunk) in first.chunks(480 * 2).enumerate() {
            framer
                .push_interleaved(chunk, start + ms(100 + 10 * i as u64))
                .unwrap();
        }

        // The device changes: 400 ms pass before the new one, 44.1 kHz mono,
        // delivers its first buffer at 1500 ms.
        framer.set_input_format(44_100, 1).unwrap();
        let second = sine(440.0, 44_100, 44_100);
        for (i, chunk) in second.chunks(441).enumerate() {
            framer
                .push_interleaved(chunk, start + ms(1500 + 10 * i as u64))
                .unwrap();
        }
        framer.finish().unwrap();

        let frames: Vec<AudioFrame> = rx.try_iter().collect();
        let split = frames.iter().position(|f| f.start_ms >= 1500).unwrap();
        let (before, after) = frames.split_at(split);

        // First device: 100 ms to 1100 ms, complete and contiguous.
        assert_eq!(before[0].start_ms, 100);
        assert_contiguous(before);
        assert_eq!(
            before.iter().map(|f| f.samples.len()).sum::<usize>(),
            16_000
        );
        assert_eq!(frame_end_ms(before.last().unwrap()), 1100);

        // Second device: picks up at its real capture time, not at 1100 ms
        // and not back at zero.
        assert_eq!(after[0].start_ms, 1500);
        assert_contiguous(after);
        assert_eq!(after.iter().map(|f| f.samples.len()).sum::<usize>(), 16_000);
        assert_eq!(frame_end_ms(after.last().unwrap()), 2500);
    }

    #[test]
    fn a_reanchor_never_moves_the_clock_backwards() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 16_000, 1).unwrap();
        // 1.01 s of audio anchored at 0: the clock stands at 1010 ms.
        framer.push_mono(&vec![0.0; 16_160], start).unwrap();
        // The re-opened device claims its first buffer was captured at 900 ms.
        framer.set_input_format(16_000, 1).unwrap();
        framer.push_mono(&[0.0; 320], start + ms(900)).unwrap();

        let frames: Vec<AudioFrame> = rx.try_iter().collect();
        let last = frames.last().unwrap();
        assert_eq!(last.start_ms, 1010);
        for pair in frames.windows(2) {
            assert!(pair[1].start_ms >= frame_end_ms(&pair[0]));
        }
    }

    #[test]
    fn a_stalled_stream_is_reanchored_forward() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Them, start, tx, 16_000, 1).unwrap();
        // 100 ms of audio in 10 ms buffers, then nothing for two seconds.
        for i in 0..10 {
            framer.push_mono(&[0.2; 160], start + ms(10 * i)).unwrap();
        }
        framer.push_mono(&[0.2; 160], start + ms(2100)).unwrap();
        framer.push_mono(&[0.2; 160], start + ms(2110)).unwrap();

        let frames: Vec<AudioFrame> = rx.try_iter().collect();
        let starts: Vec<u64> = frames.iter().map(|f| f.start_ms).collect();
        assert_eq!(starts, [0, 20, 40, 60, 80, 2100]);
    }

    #[test]
    fn lateness_below_the_stall_threshold_is_not_a_gap() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 16_000, 1).unwrap();
        framer.push_mono(&[0.0; 320], start).unwrap();
        // 400 ms late, then 300 ms early: both within tolerance.
        framer.push_mono(&[0.0; 320], start + ms(420)).unwrap();
        framer.push_mono(&[0.0; 320], start).unwrap();
        let starts: Vec<u64> = rx.try_iter().map(|f| f.start_ms).collect();
        assert_eq!(starts, [0, 20, 40]);
    }

    #[test]
    fn planar_and_interleaved_input_give_the_same_frames() {
        let start = Instant::now();
        let left = sine(300.0, 16_000, 640);
        let right = vec![0.0f32; 640];
        let interleaved: Vec<f32> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .collect();

        let (tx_a, rx_a) = mpsc::channel();
        let mut a = Framer::new(Speaker::Them, start, tx_a, 16_000, 2).unwrap();
        a.push_interleaved(&interleaved, start).unwrap();

        let (tx_b, rx_b) = mpsc::channel();
        let mut b = Framer::new(Speaker::Them, start, tx_b, 16_000, 2).unwrap();
        b.push_planar(&[&left[..], &right[..]], start).unwrap();

        let a: Vec<AudioFrame> = rx_a.try_iter().collect();
        let b: Vec<AudioFrame> = rx_b.try_iter().collect();
        assert_eq!(a.len(), 2);
        assert_eq!(a, b);
        assert!((a[0].samples[10] - left[10] / 2.0).abs() < 1e-7);
    }

    #[test]
    fn a_dropped_receiver_is_reported_as_sink_closed() {
        let start = Instant::now();
        let (tx, rx) = mpsc::channel();
        let mut framer = Framer::new(Speaker::Me, start, tx, 16_000, 1).unwrap();
        drop(rx);
        assert_eq!(
            framer.push_mono(&[0.0; 320], start),
            Err(AudioError::SinkClosed)
        );
    }

    #[test]
    fn invalid_formats_are_rejected() {
        let (tx, _rx) = mpsc::channel();
        let start = Instant::now();
        assert!(Framer::new(Speaker::Me, start, tx.clone(), 48_000, 0).is_err());
        assert!(Framer::new(Speaker::Me, start, tx.clone(), 0, 1).is_err());
        let mut framer = Framer::new(Speaker::Me, start, tx, 48_000, 1).unwrap();
        assert!(framer.set_input_format(0, 1).is_err());
        assert!(framer.set_input_format(48_000, 0).is_err());
        // Still usable in its previous format.
        framer.push_mono(&[0.0; 480], start).unwrap();
    }
}
