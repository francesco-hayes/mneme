//! Writing a recording to disk as two WAV files that line up: `mic.wav` and
//! `system.wav`.
//!
//! - [`TrackWriter`] writes one speaker's frames to a WAV in which sample
//!   `n` is meeting time `n / 16 000` s, whatever the source did: silence
//!   goes in wherever the source delivered nothing.
//! - [`RecordingWriter`] is the pair of them, one per speaker. Feed it every
//!   frame of the stream; when it finishes, both files have the same length.
//! - [`record_to_wav`] is the whole recording loop behind `mneme-record`:
//!   run a [`Recorder`], write its frames, stop on a signal, a time limit or
//!   when every source has ended.
//!
//! # How the two files are kept in line
//!
//! A frame's `start_ms` is its position on the clock both sources share, so
//! it is also its position in the file: frame at `start_ms` goes to sample
//! `start_ms * 16`. Before writing a frame the writer pads with silence up
//! to that sample. That one rule covers a source that came up late (silence
//! at the front), a device change or stall (silence in the middle), and at
//! the end both files are padded to the same length, which covers a source
//! that died early or never started.
//!
//! Nothing is ever shifted: a frame that would start before the audio
//! already written (no source in this crate produces one) loses its
//! overlapping samples rather than pushing everything after it late.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker};

use crate::recorder::{Recorder, RecorderOptions, SourcePhase, SourceStatus};
use crate::{AudioError, AudioSource, WavWriter};

/// File name of the microphone channel ([`Speaker::Me`]).
pub const MIC_WAV: &str = "mic.wav";

/// File name of the system audio channel ([`Speaker::Them`]).
pub const SYSTEM_WAV: &str = "system.wav";

const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;

/// How long the recording loop waits for a frame before looking at the stop
/// flag, the time limit and the sources.
const TICK: Duration = Duration::from_millis(25);

/// How often the WAV headers are brought up to date while recording, so that
/// a process that is killed leaves playable files.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

fn duration_to_samples(duration: Duration) -> u64 {
    (duration.as_micros() * u128::from(SAMPLE_RATE) / 1_000_000) as u64
}

fn samples_to_duration(samples: u64) -> Duration {
    Duration::from_micros(samples * 1_000_000 / u64::from(SAMPLE_RATE))
}

/// Writes one speaker's frames to a WAV file laid out in meeting time.
///
/// See the [module documentation](self) for the rule.
#[derive(Debug)]
pub struct TrackWriter {
    speaker: Speaker,
    path: PathBuf,
    wav: WavWriter,
    /// Samples past this position are not written.
    limit: Option<u64>,
    captured: u64,
    padded: u64,
    dropped: u64,
    gaps: u32,
    peak: f32,
    first_audio: Option<u64>,
    audio_end: u64,
}

impl TrackWriter {
    /// Creates (or truncates) the WAV file at `path` for frames of
    /// `speaker`.
    pub fn create(speaker: Speaker, path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let path = path.as_ref().to_path_buf();
        Ok(Self {
            speaker,
            wav: WavWriter::create(&path)?,
            path,
            limit: None,
            captured: 0,
            padded: 0,
            dropped: 0,
            gaps: 0,
            peak: 0.0,
            first_audio: None,
            audio_end: 0,
        })
    }

    /// Cuts the track off at `limit` of meeting time: audio after it is not
    /// written.
    pub fn limited_to(mut self, limit: Duration) -> Self {
        self.limit = Some(duration_to_samples(limit));
        self
    }

    /// Samples in the file so far, silence included.
    pub fn samples(&self) -> u64 {
        self.wav.samples_written()
    }

    /// Writes a frame at its place in meeting time, padding with silence up
    /// to it first. The frame's speaker label is not looked at.
    pub fn write_frame(&mut self, frame: &AudioFrame) -> Result<(), AudioError> {
        let mut samples = &frame.samples[..];
        let mut at = frame.start_ms * SAMPLES_PER_MS;
        let position = self.wav.samples_written();
        if at < position {
            // Keep the timeline: drop what overlaps the audio already there.
            let overlap = ((position - at) as usize).min(samples.len());
            self.dropped += overlap as u64;
            samples = &samples[overlap..];
            at = position;
        }
        if let Some(limit) = self.limit {
            let room = limit.saturating_sub(at).min(samples.len() as u64) as usize;
            self.dropped += (samples.len() - room) as u64;
            samples = &samples[..room];
        }
        if samples.is_empty() {
            return Ok(());
        }
        if at > position {
            self.wav.write_silence(at - position)?;
            self.padded += at - position;
            if self.captured > 0 {
                self.gaps += 1;
            }
        }
        self.wav.write(samples)?;
        self.captured += samples.len() as u64;
        self.peak = samples.iter().fold(self.peak, |peak, s| peak.max(s.abs()));
        self.first_audio.get_or_insert(at);
        self.audio_end = at + samples.len() as u64;
        Ok(())
    }

    /// Brings the file's header up to date, so that it is a valid WAV even if
    /// the process dies afterwards.
    pub fn flush(&mut self) -> Result<(), AudioError> {
        self.wav.flush()
    }

    /// Pads the track with silence to `samples` in total, if it is shorter,
    /// and finishes the file.
    pub fn finish(mut self, samples: u64) -> Result<TrackSummary, AudioError> {
        let position = self.wav.samples_written();
        if samples > position {
            self.wav.write_silence(samples - position)?;
            self.padded += samples - position;
        }
        let summary = TrackSummary {
            speaker: self.speaker,
            path: self.path,
            samples: self.wav.samples_written(),
            captured_samples: self.captured,
            padded_samples: self.padded,
            dropped_samples: self.dropped,
            gaps: self.gaps,
            peak: self.peak,
            first_audio_ms: self.first_audio.map(|s| s / SAMPLES_PER_MS),
            last_audio_ms: self.first_audio.map(|_| self.audio_end / SAMPLES_PER_MS),
        };
        self.wav.finalize()?;
        Ok(summary)
    }
}

/// What went into one WAV file.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackSummary {
    /// Whose audio the file holds.
    pub speaker: Speaker,
    /// Where the file is.
    pub path: PathBuf,
    /// Length of the file in samples: captured plus padded.
    pub samples: u64,
    /// Samples that came from the source.
    pub captured_samples: u64,
    /// Samples of silence written where the source delivered nothing: before
    /// its first frame, in gaps, and after its last frame.
    pub padded_samples: u64,
    /// Samples of the source that were not written: past the time limit, or
    /// overlapping audio already written.
    pub dropped_samples: u64,
    /// Number of holes in the source's audio between its first and last
    /// frame (device changes, stalls).
    pub gaps: u32,
    /// Largest absolute sample value captured. Exactly zero means the source
    /// delivered nothing but digital silence.
    pub peak: f32,
    /// Meeting time of the first captured sample, if there was one.
    pub first_audio_ms: Option<u64>,
    /// Meeting time of the end of the last captured sample.
    pub last_audio_ms: Option<u64>,
}

impl TrackSummary {
    /// Length of the file.
    pub fn duration(&self) -> Duration {
        samples_to_duration(self.samples)
    }

    /// How much of the file came from the source.
    pub fn captured(&self) -> Duration {
        samples_to_duration(self.captured_samples)
    }

    /// How much of the file is silence written in place of missing audio.
    pub fn padded(&self) -> Duration {
        samples_to_duration(self.padded_samples)
    }

    /// `true` if the source delivered audio and every sample of it would be
    /// written as zero: digital silence, not a quiet room.
    pub fn is_silent(&self) -> bool {
        self.captured_samples > 0 && self.peak < 0.5 / 32_768.0
    }
}

/// Writes a two-channel recording as `mic.wav` ([`Speaker::Me`]) and
/// `system.wav` ([`Speaker::Them`]) in one folder, both in meeting time and
/// of the same length.
///
/// ```no_run
/// use std::time::Duration;
/// use mneme_audio::{AudioSource, MicSource, Recorder, RecordingWriter, SystemAudioSource};
///
/// let sources: Vec<Box<dyn AudioSource>> =
///     vec![Box::new(MicSource::new()), Box::new(SystemAudioSource::new())];
/// let mut wavs = RecordingWriter::create("/tmp/recording")?;
/// let (recorder, frames) = Recorder::start(sources);
/// for frame in frames.iter().take(500) {
///     wavs.write(&frame)?;
/// }
/// let report = recorder.stop();
/// for frame in frames.iter() {
///     wavs.write(&frame)?; // what was captured up to the stop
/// }
/// wavs.finish(report.duration)?;
/// # Ok::<(), mneme_audio::AudioError>(())
/// ```
#[derive(Debug)]
pub struct RecordingWriter {
    mic: TrackWriter,
    system: TrackWriter,
}

impl RecordingWriter {
    /// Creates `dir` if needed and both files in it, replacing files of
    /// those names.
    pub fn create(dir: impl AsRef<Path>) -> Result<Self, AudioError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(|e| {
            AudioError::Wav(format!("could not create folder {}: {e}", dir.display()))
        })?;
        Ok(Self {
            mic: TrackWriter::create(Speaker::Me, dir.join(MIC_WAV))?,
            system: TrackWriter::create(Speaker::Them, dir.join(SYSTEM_WAV))?,
        })
    }

    /// Cuts both files off at `limit` of meeting time.
    pub fn limited_to(mut self, limit: Duration) -> Self {
        self.mic = self.mic.limited_to(limit);
        self.system = self.system.limited_to(limit);
        self
    }

    /// Writes a frame to its speaker's file, at its place in meeting time.
    pub fn write(&mut self, frame: &AudioFrame) -> Result<(), AudioError> {
        match frame.speaker {
            Speaker::Me => self.mic.write_frame(frame),
            Speaker::Them => self.system.write_frame(frame),
        }
    }

    /// Brings both headers up to date. Call it now and then during a long
    /// recording: files flushed this way stay playable if the process dies.
    pub fn flush(&mut self) -> Result<(), AudioError> {
        self.mic.flush()?;
        self.system.flush()
    }

    /// Pads both files with silence to one length and finishes them. The
    /// length is that of the longer file, or `at_least` if that is longer
    /// still (pass the recording's duration, or zero to end where the audio
    /// ends).
    ///
    /// Returns the summaries of `mic.wav` and `system.wav`, in that order.
    pub fn finish(self, at_least: Duration) -> Result<[TrackSummary; 2], AudioError> {
        let mut samples = self.mic.samples().max(self.system.samples());
        let mut at_least = duration_to_samples(at_least);
        if let Some(limit) = self.mic.limit {
            at_least = at_least.min(limit);
        }
        samples = samples.max(at_least);
        // Finish both even if the first fails: one good file beats none.
        let mic = self.mic.finish(samples);
        let system = self.system.finish(samples);
        Ok([mic?, system?])
    }
}

/// Settings for [`record_to_wav`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordOptions {
    /// Stop after this much meeting time. The files are then exactly this
    /// long. `None` records until told to stop.
    pub limit: Option<Duration>,
    /// How the sources are started.
    pub recorder: RecorderOptions,
}

/// Why a recording ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The stop flag was set (Ctrl-C in `mneme-record`).
    Interrupted,
    /// The time limit was reached.
    LimitReached,
    /// Every source failed to start or ended by itself; there was nothing
    /// left to record.
    SourcesEnded,
}

/// Something that happened during [`record_to_wav`], reported as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordEvent {
    /// A source moved to a new phase.
    Phase {
        /// Which source.
        speaker: Speaker,
        /// The phase it is in now.
        phase: SourcePhase,
    },
    /// A source reported an error. It may well still be running.
    Error {
        /// Which source.
        speaker: Speaker,
        /// What it reported.
        error: AudioError,
    },
    /// The loop is ending and is about to stop the sources.
    Stopping(StopReason),
}

/// The result of [`record_to_wav`].
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    /// Why it ended.
    pub reason: StopReason,
    /// Length of both files.
    pub duration: Duration,
    /// `mic.wav` and `system.wav`, in that order.
    pub tracks: [TrackSummary; 2],
    /// The final state of every source, in the order given, with all its
    /// errors.
    pub sources: Vec<SourceStatus>,
}

impl Recording {
    /// `true` if any source failed to start, ended by itself or never got to
    /// start. The files are complete and aligned either way.
    pub fn any_source_failed(&self) -> bool {
        self.sources.iter().any(|s| s.phase.is_failure())
    }
}

/// Records `sources` into `mic.wav` and `system.wav` in `out_dir` until
/// `stop` becomes `true`, the time limit in `options` is reached, or every
/// source has ended. This is the loop behind the `mneme-record` tool.
///
/// The sources are run by a [`Recorder`]: one clock, a source that fails or
/// dies does not stop the other, a `start` that blocks does not hold the
/// other up. Both files are always finished properly and have the same
/// length:
///
/// | Ended by | Length of both files |
/// | --- | --- |
/// | the time limit | exactly the limit |
/// | the stop flag | the meeting time at which the sources were told to stop (or the end of the last audio, if a source delivered a frame reaching past it) |
/// | every source ending | up to the last audio captured |
///
/// `on_event` is told about phase changes and source errors while they
/// happen (the same information is in the returned [`Recording`]).
///
/// Returns an error only if a file cannot be created or written; the
/// sources are stopped first. Source failures are not errors here: they are
/// in [`Recording::sources`].
pub fn record_to_wav(
    sources: Vec<Box<dyn AudioSource>>,
    out_dir: &Path,
    options: &RecordOptions,
    stop: &AtomicBool,
    on_event: &mut dyn FnMut(RecordEvent),
) -> Result<Recording, AudioError> {
    // Before any source is started: no permission prompt for a recording
    // that has nowhere to go.
    let mut writer = RecordingWriter::create(out_dir)?;
    if let Some(limit) = options.limit {
        writer = writer.limited_to(limit);
    }

    // If a write fails below, dropping the recorder stops the sources.
    let (recorder, frames) = Recorder::start_with(sources, options.recorder);
    let mut seen = Seen::default();
    let mut last_look = Instant::now();
    let mut last_flush = Instant::now();
    let reason = loop {
        match frames.recv_timeout(TICK) {
            Ok(frame) => writer.write(&frame)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break StopReason::SourcesEnded,
        }
        if stop.load(Ordering::SeqCst) {
            break StopReason::Interrupted;
        }
        if options
            .limit
            .is_some_and(|limit| recorder.elapsed() >= limit)
        {
            break StopReason::LimitReached;
        }
        if last_look.elapsed() >= TICK {
            last_look = Instant::now();
            seen.report_news(&recorder.status(), on_event);
        }
        if last_flush.elapsed() >= FLUSH_EVERY {
            last_flush = Instant::now();
            writer.flush()?;
        }
    };

    // Stopping a real device is the one step here that could hang; leave
    // valid files behind first.
    writer.flush()?;
    on_event(RecordEvent::Stopping(reason));
    let report = recorder.stop();
    // Everything captured up to the stop is in the channel, and it ends.
    for frame in frames.iter() {
        writer.write(&frame)?;
    }
    seen.report_news(&report.sources, on_event);

    let at_least = match reason {
        StopReason::LimitReached => options.limit.unwrap_or(report.duration),
        StopReason::Interrupted => report.duration,
        StopReason::SourcesEnded => Duration::ZERO,
    };
    let tracks = writer.finish(at_least)?;
    Ok(Recording {
        reason,
        duration: tracks[0].duration(),
        tracks,
        sources: report.sources,
    })
}

/// What has already been passed to the event callback, per source.
#[derive(Default)]
struct Seen(Vec<(SourcePhase, usize)>);

impl Seen {
    fn report_news(&mut self, sources: &[SourceStatus], on_event: &mut dyn FnMut(RecordEvent)) {
        self.0
            .resize(sources.len(), (SourcePhase::Starting, 0usize));
        for (source, (phase, errors)) in sources.iter().zip(&mut self.0) {
            for error in source.errors.iter().skip(*errors) {
                on_event(RecordEvent::Error {
                    speaker: source.speaker,
                    error: error.clone(),
                });
            }
            *errors = source.errors.len();
            if source.phase != *phase {
                *phase = source.phase;
                on_event(RecordEvent::Phase {
                    speaker: source.speaker,
                    phase: source.phase,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;
    use crate::testutil::{FrameListSource, GatedSource, Probe, sine};
    use crate::wav::f32_to_i16;
    use crate::{FakeSource, Pacing, read_wav_16k_mono};

    /// A fresh folder for one test's files.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mneme-audio-recording-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn frame(speaker: Speaker, start_ms: u64, samples: Vec<f32>) -> AudioFrame {
        AudioFrame {
            speaker,
            start_ms,
            samples,
        }
    }

    /// What a WAV holds, as the 16-bit values in the file.
    fn read_i16(path: &Path) -> Vec<i16> {
        // A file with a stale header would not read back at its full length.
        let reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        let samples: Vec<i16> = reader
            .into_samples::<i16>()
            .collect::<Result<_, _>>()
            .unwrap();
        // The header's sizes agree with the file: it was finished properly.
        assert_eq!(
            std::fs::metadata(path).unwrap().len(),
            44 + 2 * samples.len() as u64
        );
        samples
    }

    fn as_i16(samples: &[f32]) -> Vec<i16> {
        samples.iter().map(|s| f32_to_i16(*s)).collect()
    }

    fn all_zero(samples: &[i16]) -> bool {
        samples.iter().all(|s| *s == 0)
    }

    fn boxed(sources: Vec<FakeSource>) -> Vec<Box<dyn AudioSource>> {
        sources
            .into_iter()
            .map(|s| Box::new(s) as Box<dyn AudioSource>)
            .collect()
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    fn record(
        sources: Vec<Box<dyn AudioSource>>,
        dir: &Path,
        options: RecordOptions,
    ) -> (Recording, Vec<RecordEvent>) {
        let mut events = Vec::new();
        let recording =
            record_to_wav(sources, dir, &options, &never(), &mut |e| events.push(e)).unwrap();
        (recording, events)
    }

    /// An event callback that sets `stop` once `after` has passed since a
    /// source was first seen running, like someone pressing Ctrl-C then.
    /// The recording's clock started before that moment, so the recording is
    /// at least `after` long.
    fn interrupt_after(stop: &Arc<AtomicBool>, after: Duration) -> impl FnMut(RecordEvent) {
        let stop = Arc::clone(stop);
        let mut armed = false;
        move |event| {
            let running = matches!(
                event,
                RecordEvent::Phase {
                    phase: SourcePhase::Running,
                    ..
                }
            );
            if running && !armed {
                armed = true;
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    thread::sleep(after);
                    stop.store(true, Ordering::SeqCst);
                });
            }
        }
    }

    /// A tone at 0.5 of full scale.
    fn tone(freq: f64, len: usize) -> Vec<f32> {
        sine(freq, 16_000, len).iter().map(|s| s * 0.5).collect()
    }

    // ---- TrackWriter ----

    #[test]
    fn a_track_places_frames_by_their_timestamp() {
        let dir = temp_dir("track");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.wav");
        let mut track = TrackWriter::create(Speaker::Them, &path).unwrap();

        // Came up 100 ms late; one hole of 60 ms after a short frame.
        track
            .write_frame(&frame(Speaker::Them, 100, vec![0.5; 320]))
            .unwrap();
        track
            .write_frame(&frame(Speaker::Them, 120, vec![0.5; 100]))
            .unwrap();
        assert_eq!(track.samples(), 2_020);
        track
            .write_frame(&frame(Speaker::Them, 200, vec![-0.25; 320]))
            .unwrap();
        // Died there; the recording went on to 400 ms.
        let summary = track.finish(6_400).unwrap();

        let wav = read_i16(&path);
        assert_eq!(wav.len(), 6_400);
        assert!(all_zero(&wav[..1_600]));
        assert!(wav[1_600..2_020].iter().all(|s| *s == 16_384));
        assert!(all_zero(&wav[2_020..3_200]));
        assert!(wav[3_200..3_520].iter().all(|s| *s == -8_192));
        assert!(all_zero(&wav[3_520..]));

        assert_eq!(summary.speaker, Speaker::Them);
        assert_eq!(summary.path, path);
        assert_eq!(summary.samples, 6_400);
        assert_eq!(summary.captured_samples, 740);
        assert_eq!(summary.padded_samples, 6_400 - 740);
        assert_eq!(summary.dropped_samples, 0);
        assert_eq!(summary.gaps, 1);
        assert_eq!(summary.peak, 0.5);
        assert_eq!(summary.first_audio_ms, Some(100));
        assert_eq!(summary.last_audio_ms, Some(220));
        assert_eq!(summary.duration(), Duration::from_millis(400));
        assert_eq!(summary.captured(), Duration::from_micros(46_250));
        assert!(!summary.is_silent());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_track_never_shifts_audio_to_make_room() {
        let dir = temp_dir("overlap");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.wav");
        let mut track = TrackWriter::create(Speaker::Me, &path).unwrap();
        track
            .write_frame(&frame(Speaker::Me, 0, vec![0.5; 320]))
            .unwrap();
        // Claims to start 5 ms before the previous frame ended.
        track
            .write_frame(&frame(Speaker::Me, 15, vec![0.25; 320]))
            .unwrap();
        // Entirely inside what is written: nothing of it is used.
        track
            .write_frame(&frame(Speaker::Me, 0, vec![1.0; 160]))
            .unwrap();
        track
            .write_frame(&frame(Speaker::Me, 35, vec![-0.5; 160]))
            .unwrap();
        let summary = track.finish(0).unwrap();

        let wav = read_i16(&path);
        // 0..20 ms first frame, 20..35 ms the rest of the second, on time.
        assert_eq!(wav.len(), 720);
        assert!(wav[..320].iter().all(|s| *s == 16_384));
        assert!(wav[320..560].iter().all(|s| *s == 8_192));
        assert!(wav[560..].iter().all(|s| *s == -16_384));
        assert_eq!(summary.captured_samples, 720);
        assert_eq!(summary.dropped_samples, 80 + 160);
        assert_eq!(summary.padded_samples, 0);
        assert_eq!(summary.gaps, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_limited_track_cuts_off_at_the_limit() {
        let dir = temp_dir("limit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.wav");
        let mut track = TrackWriter::create(Speaker::Me, &path)
            .unwrap()
            .limited_to(Duration::from_millis(50));
        track
            .write_frame(&frame(Speaker::Me, 20, vec![0.5; 320]))
            .unwrap();
        // Straddles the limit: 10 ms of it fits.
        track
            .write_frame(&frame(Speaker::Me, 40, vec![0.5; 320]))
            .unwrap();
        // Past it: not even padding is written for it.
        track
            .write_frame(&frame(Speaker::Me, 500, vec![0.5; 320]))
            .unwrap();
        assert_eq!(track.samples(), 800);
        let summary = track.finish(800).unwrap();
        assert_eq!(read_i16(&path).len(), 800);
        assert_eq!(summary.captured_samples, 480);
        assert_eq!(summary.padded_samples, 320);
        assert_eq!(summary.dropped_samples, 160 + 320);
        assert_eq!(summary.last_audio_ms, Some(50));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn digital_silence_is_told_from_quiet_audio_and_from_no_audio() {
        let dir = temp_dir("silent");
        std::fs::create_dir_all(&dir).unwrap();

        let mut track = TrackWriter::create(Speaker::Them, dir.join("zeros.wav")).unwrap();
        track
            .write_frame(&frame(Speaker::Them, 0, vec![0.0; 320]))
            .unwrap();
        let zeros = track.finish(0).unwrap();
        assert!(zeros.is_silent());
        assert_eq!(zeros.peak, 0.0);

        let mut track = TrackWriter::create(Speaker::Them, dir.join("quiet.wav")).unwrap();
        track
            .write_frame(&frame(Speaker::Them, 0, vec![0.001; 320]))
            .unwrap();
        assert!(!track.finish(0).unwrap().is_silent());

        // No frames at all is a different finding: nothing was captured.
        let track = TrackWriter::create(Speaker::Them, dir.join("empty.wav")).unwrap();
        let empty = track.finish(1_600).unwrap();
        assert!(!empty.is_silent());
        assert_eq!((empty.captured_samples, empty.padded_samples), (0, 1_600));
        assert_eq!((empty.first_audio_ms, empty.last_audio_ms), (None, None));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- RecordingWriter ----

    #[test]
    fn the_pair_routes_by_speaker_and_ends_at_one_length() {
        let dir = temp_dir("pair");
        // The folder is created, parents included.
        let nested = dir.join("a/b");
        let mut wavs = RecordingWriter::create(&nested).unwrap();
        wavs.write(&frame(Speaker::Them, 40, vec![0.25; 320]))
            .unwrap();
        wavs.write(&frame(Speaker::Me, 0, vec![0.5; 320])).unwrap();
        wavs.flush().unwrap();
        // Flushed files are valid while still being written.
        assert_eq!(read_wav_16k_mono(nested.join(MIC_WAV)).unwrap().len(), 320);
        let [mic, system] = wavs.finish(Duration::ZERO).unwrap();

        assert_eq!(mic.path, nested.join("mic.wav"));
        assert_eq!(system.path, nested.join("system.wav"));
        assert_eq!((mic.speaker, system.speaker), (Speaker::Me, Speaker::Them));
        assert_eq!((mic.samples, system.samples), (960, 960));
        let mic = read_i16(&mic.path);
        let system = read_i16(&system.path);
        assert!(mic[..320].iter().all(|s| *s == 16_384) && all_zero(&mic[320..]));
        assert!(all_zero(&system[..640]) && system[640..].iter().all(|s| *s == 8_192));

        // A requested length beyond the audio pads both.
        let mut wavs = RecordingWriter::create(&nested).unwrap();
        wavs.write(&frame(Speaker::Me, 0, vec![0.5; 320])).unwrap();
        let [mic, system] = wavs.finish(Duration::from_millis(100)).unwrap();
        assert_eq!((mic.samples, system.samples), (1_600, 1_600));
        assert_eq!(read_i16(&system.path).len(), 1_600);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_cannot_be_created_is_an_error() {
        let dir = temp_dir("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file"), b"not a folder").unwrap();
        let err = RecordingWriter::create(dir.join("file/sub")).unwrap_err();
        assert!(matches!(err, AudioError::Wav(_)), "{err:?}");
        // The loop fails the same way, before any source is started.
        let probe = Probe::default();
        let source = GatedSource::open(
            FakeSource::from_samples(Speaker::Me, vec![0.0; 320]),
            &probe,
        );
        let result = record_to_wav(
            vec![Box::new(source)],
            &dir.join("file/sub"),
            &RecordOptions::default(),
            &never(),
            &mut |_| {},
        );
        assert!(matches!(result, Err(AudioError::Wav(_))));
        assert_eq!(probe.start_calls(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ---- The recording loop ----

    #[test]
    fn two_sources_give_two_files_of_one_length_with_offsets_kept() {
        let dir = temp_dir("two-sources");
        // The microphone from the start, for 1 s. System audio comes up
        // 500 ms late and outlasts it by 300 ms.
        let mic_audio = tone(440.0, 16_000);
        let system_audio = tone(880.0, 12_800);
        let mic = FakeSource::from_samples(Speaker::Me, mic_audio.clone());
        let system = FakeSource::from_samples(Speaker::Them, system_audio.clone())
            .starting_at(Duration::from_millis(500));
        let (recording, _) = record(boxed(vec![mic, system]), &dir, RecordOptions::default());

        assert_eq!(recording.reason, StopReason::SourcesEnded);
        assert_eq!(recording.duration, Duration::from_millis(1_300));
        // Fakes run out of input; that is their way of ending.
        assert!(
            recording
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Ended && s.errors.is_empty())
        );

        let mic = read_i16(&dir.join("mic.wav"));
        let system = read_i16(&dir.join("system.wav"));
        // Same length.
        assert_eq!(mic.len(), 20_800);
        assert_eq!(system.len(), 20_800);
        // The microphone's audio sits at 0, then silence to the end.
        assert_eq!(mic[..16_000], as_i16(&mic_audio)[..]);
        assert!(all_zero(&mic[16_000..]));
        // The late source is padded at the front by exactly its lateness.
        assert!(all_zero(&system[..8_000]));
        assert_eq!(system[8_000..], as_i16(&system_audio)[..]);

        let [mic, system] = &recording.tracks;
        assert_eq!((mic.captured_samples, mic.padded_samples), (16_000, 4_800));
        assert_eq!(
            (system.captured_samples, system.padded_samples),
            (12_800, 8_000)
        );
        assert_eq!(
            (mic.first_audio_ms, mic.last_audio_ms),
            (Some(0), Some(1_000))
        );
        assert_eq!(
            (system.first_audio_ms, system.last_audio_ms),
            (Some(500), Some(1_300))
        );
        assert_eq!((mic.gaps, system.gaps), (0, 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_source_that_fails_mid_run_leaves_the_other_file_complete() {
        let dir = temp_dir("mid-run-failure");
        let died = AudioError::Stream("tap went away".to_owned());
        let mic_audio = tone(440.0, 32_000);
        let system_audio = tone(880.0, 32_000);
        let mic = FakeSource::from_samples(Speaker::Me, mic_audio.clone());
        let system = FakeSource::from_samples(Speaker::Them, system_audio.clone())
            .failing_after(Duration::from_millis(250), died.clone());
        let (recording, events) = record(boxed(vec![mic, system]), &dir, RecordOptions::default());

        // The healthy channel is whole.
        let mic = read_i16(&dir.join("mic.wav"));
        assert_eq!(mic, as_i16(&mic_audio));
        // The failed one has what it captured, then silence to the same end.
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!(system.len(), 32_000);
        assert_eq!(system[..4_000], as_i16(&system_audio)[..4_000]);
        assert!(all_zero(&system[4_000..]));

        // And the failure is reported: in the result ...
        assert!(recording.any_source_failed());
        assert_eq!(recording.sources[0].phase, SourcePhase::Ended);
        assert!(recording.sources[0].errors.is_empty());
        assert_eq!(recording.sources[1].phase, SourcePhase::Ended);
        assert_eq!(recording.sources[1].errors, std::slice::from_ref(&died));
        assert_eq!(recording.sources[1].end_ms, Some(250));
        let [mic, system] = &recording.tracks;
        assert_eq!((mic.captured_samples, mic.padded_samples), (32_000, 0));
        assert_eq!(
            (system.captured_samples, system.padded_samples),
            (4_000, 28_000)
        );
        assert_eq!(system.last_audio_ms, Some(250));
        // ... and as an event, once.
        let reported: Vec<&RecordEvent> = events
            .iter()
            .filter(|e| matches!(e, RecordEvent::Error { .. }))
            .collect();
        assert_eq!(
            reported,
            [&RecordEvent::Error {
                speaker: Speaker::Them,
                error: died
            }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_source_that_dies_while_the_other_records_on_in_real_time() {
        let dir = temp_dir("dies-in-real-time");
        let died = AudioError::Stream("device unplugged".to_owned());
        // A microphone that runs until stopped, and system audio that dies
        // 100 ms in. The recording goes on for 400 ms.
        let mic =
            FakeSource::from_samples(Speaker::Me, vec![0.5; 960_000]).with_pacing(Pacing::RealTime);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.25; 960_000])
            .with_pacing(Pacing::RealTime)
            .failing_after(Duration::from_millis(100), died.clone());
        let options = RecordOptions {
            limit: Some(Duration::from_millis(400)),
            ..RecordOptions::default()
        };
        let (recording, events) = record(boxed(vec![mic, system]), &dir, options);

        assert_eq!(recording.reason, StopReason::LimitReached);
        assert_eq!(recording.sources[0].phase, SourcePhase::Stopped);
        assert_eq!(recording.sources[1].phase, SourcePhase::Ended);
        assert_eq!(recording.sources[1].errors, [died]);
        let mic = read_i16(&dir.join("mic.wav"));
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!((mic.len(), system.len()), (6_400, 6_400));
        // The microphone kept recording well past the other's death.
        assert!(mic[..4_800].iter().all(|s| *s == 16_384));
        assert!(system[..1_600].iter().all(|s| *s == 8_192));
        assert!(all_zero(&system[1_600..]));
        // The death was announced while recording, before the stop.
        let ended = events
            .iter()
            .position(|e| {
                *e == RecordEvent::Phase {
                    speaker: Speaker::Them,
                    phase: SourcePhase::Ended,
                }
            })
            .expect("no event for the source ending");
        let stopping = events
            .iter()
            .position(|e| matches!(e, RecordEvent::Stopping(_)))
            .unwrap();
        assert!(ended < stopping, "{events:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_source_that_fails_to_start_gives_a_silent_file_of_the_same_length() {
        let dir = temp_dir("start-failure");
        let denied = AudioError::PermissionDenied("system audio".to_owned());
        let mic_audio = tone(440.0, 8_000);
        let mic = FakeSource::from_samples(Speaker::Me, mic_audio.clone());
        let system = FakeSource::from_samples(Speaker::Them, vec![0.5; 8_000])
            .failing_to_start(denied.clone());
        let (recording, events) = record(boxed(vec![mic, system]), &dir, RecordOptions::default());

        assert_eq!(read_i16(&dir.join("mic.wav")), as_i16(&mic_audio));
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!(system.len(), 8_000);
        assert!(all_zero(&system));

        assert!(recording.any_source_failed());
        assert_eq!(recording.sources[1].phase, SourcePhase::FailedToStart);
        assert_eq!(recording.sources[1].errors, std::slice::from_ref(&denied));
        let system = &recording.tracks[1];
        assert_eq!((system.captured_samples, system.padded_samples), (0, 8_000));
        // Nothing captured is not "captured silence".
        assert!(!system.is_silent());
        assert!(events.contains(&RecordEvent::Error {
            speaker: Speaker::Them,
            error: denied
        }));
        assert!(events.contains(&RecordEvent::Phase {
            speaker: Speaker::Them,
            phase: SourcePhase::FailedToStart
        }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_source_starting_still_gives_two_valid_files() {
        let dir = temp_dir("nothing-starts");
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.5; 320])
            .failing_to_start(AudioError::NoInputDevice);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.5; 320])
            .failing_to_start(AudioError::NoOutputDevice);
        let begun = Instant::now();
        let (recording, _) = record(boxed(vec![mic, system]), &dir, RecordOptions::default());
        // It does not sit there recording nothing.
        assert!(begun.elapsed() < Duration::from_secs(10));
        assert_eq!(recording.reason, StopReason::SourcesEnded);
        assert_eq!(recording.duration, Duration::ZERO);
        assert!(read_i16(&dir.join("mic.wav")).is_empty());
        assert!(read_i16(&dir.join("system.wav")).is_empty());
        assert_eq!(
            recording
                .sources
                .iter()
                .flat_map(|s| s.errors.clone())
                .collect::<Vec<_>>(),
            [AudioError::NoInputDevice, AudioError::NoOutputDevice]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_gap_in_a_source_becomes_silence_of_the_same_length() {
        let dir = temp_dir("gap");
        // System audio with a device change: 1 s without frames, then audio
        // again at its real time.
        let system = FrameListSource::new(
            Speaker::Them,
            vec![
                frame(Speaker::Them, 0, vec![0.5; 320]),
                frame(Speaker::Them, 20, vec![0.5; 200]),
                frame(Speaker::Them, 1_020, vec![0.25; 320]),
                frame(Speaker::Them, 1_040, vec![0.25; 320]),
            ],
        );
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.5; 16_960]);
        let (recording, _) = record(
            vec![Box::new(mic), Box::new(system)],
            &dir,
            RecordOptions::default(),
        );
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!(system.len(), 16_960);
        assert!(system[..520].iter().all(|s| *s == 16_384));
        assert!(all_zero(&system[520..16_320]));
        assert!(system[16_320..].iter().all(|s| *s == 8_192));
        assert_eq!(read_i16(&dir.join("mic.wav")).len(), 16_960);
        let track = &recording.tracks[1];
        assert_eq!(track.gaps, 1);
        assert_eq!(track.padded_samples, 15_800);
        assert_eq!(track.captured_samples, 1_160);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_time_limit_gives_files_of_exactly_that_length() {
        let dir = temp_dir("limit-loop");
        let mic =
            FakeSource::from_samples(Speaker::Me, vec![0.5; 960_000]).with_pacing(Pacing::RealTime);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.25; 960_000])
            .with_pacing(Pacing::RealTime)
            .starting_at(Duration::from_millis(100));
        let options = RecordOptions {
            limit: Some(Duration::from_millis(300)),
            ..RecordOptions::default()
        };
        let begun = Instant::now();
        let (recording, events) = record(boxed(vec![mic, system]), &dir, options);
        let took = begun.elapsed();
        assert!(took >= Duration::from_millis(300), "took {took:?}");
        assert!(took < Duration::from_secs(20), "took {took:?}");

        assert_eq!(recording.reason, StopReason::LimitReached);
        assert_eq!(recording.duration, Duration::from_millis(300));
        assert!(!recording.any_source_failed());
        assert!(
            recording
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Stopped)
        );
        let mic = read_i16(&dir.join("mic.wav"));
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!((mic.len(), system.len()), (4_800, 4_800));
        // The microphone's audio starts at 0; the other's 100 ms in.
        assert_eq!(mic[0], 16_384);
        assert!(all_zero(&system[..1_600]));
        assert_eq!(system[1_600], 8_192);
        // Whatever each source captured before the stop is in its file;
        // every sample is either that audio or padding, and audio never
        // follows padding once the source has started.
        let captured = recording.tracks[0].captured_samples as usize;
        assert!(captured > 0 && captured <= 4_800);
        assert!(mic[..captured].iter().all(|s| *s == 16_384));
        assert!(all_zero(&mic[captured..]));
        assert_eq!(
            recording.tracks[0].padded_samples as usize,
            4_800 - captured
        );
        assert!(events.contains(&RecordEvent::Stopping(StopReason::LimitReached)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn audio_stamped_past_the_limit_is_left_out() {
        let dir = temp_dir("limit-fast");
        // Fast fakes hand over a second of audio at once; the limit still
        // holds for the files.
        let mic = FakeSource::from_samples(Speaker::Me, vec![0.5; 16_000]);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.25; 16_000]);
        let options = RecordOptions {
            limit: Some(Duration::from_millis(250)),
            ..RecordOptions::default()
        };
        let (recording, _) = record(boxed(vec![mic, system]), &dir, options);
        assert_eq!(recording.duration, Duration::from_millis(250));
        assert_eq!(read_i16(&dir.join("mic.wav")).len(), 4_000);
        assert_eq!(read_i16(&dir.join("system.wav")).len(), 4_000);
        assert_eq!(recording.tracks[0].dropped_samples, 12_000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_stop_flag_ends_the_recording_with_both_files_finished() {
        let dir = temp_dir("interrupted");
        let mic =
            FakeSource::from_samples(Speaker::Me, vec![0.5; 960_000]).with_pacing(Pacing::RealTime);
        let system = FakeSource::from_samples(Speaker::Them, vec![0.25; 960_000])
            .with_pacing(Pacing::RealTime);
        // Ctrl-C, 200 ms in.
        let stop = Arc::new(AtomicBool::new(false));
        let begun = Instant::now();
        let recording = record_to_wav(
            boxed(vec![mic, system]),
            &dir,
            &RecordOptions::default(),
            &stop,
            &mut interrupt_after(&stop, Duration::from_millis(200)),
        )
        .unwrap();
        let took = begun.elapsed();

        assert_eq!(recording.reason, StopReason::Interrupted);
        assert!(!recording.any_source_failed());
        assert!(recording.duration >= Duration::from_millis(200));
        assert!(recording.duration <= took);
        assert!(
            recording
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Stopped)
        );
        // `read_i16` checks each header against its file: both were
        // finished, not left as they were when the flag was set.
        let mic = read_i16(&dir.join("mic.wav"));
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!(mic.len(), system.len());
        assert_eq!(mic.len() as u64, recording.tracks[0].samples);
        assert!(mic.len() >= 3_200);
        // Captured audio first, then at most a short tail of padding up to
        // the moment of the stop.
        for (wav, track, value) in [
            (&mic, &recording.tracks[0], 16_384),
            (&system, &recording.tracks[1], 8_192),
        ] {
            let captured = track.captured_samples as usize;
            assert!(captured >= 1_600, "only {captured} samples captured");
            assert!(wav[..captured].iter().all(|s| *s == value));
            assert!(all_zero(&wav[captured..]));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn interrupting_while_a_start_is_blocked_still_finishes_both_files() {
        let dir = temp_dir("interrupted-while-starting");
        let probe = Probe::default();
        let mic =
            FakeSource::from_samples(Speaker::Me, vec![0.5; 960_000]).with_pacing(Pacing::RealTime);
        // A permission prompt nobody answers.
        let system = GatedSource::closed(
            FakeSource::from_samples(Speaker::Them, vec![0.25; 16_000]),
            &probe,
        );
        let options = RecordOptions {
            limit: None,
            recorder: RecorderOptions {
                head_start: Duration::ZERO,
            },
        };
        let stop = Arc::new(AtomicBool::new(false));
        let begun = Instant::now();
        let recording = record_to_wav(
            vec![Box::new(mic), Box::new(system)],
            &dir,
            &options,
            &stop,
            &mut interrupt_after(&stop, Duration::from_millis(150)),
        )
        .unwrap();
        assert!(begun.elapsed() < Duration::from_secs(20));

        assert_eq!(recording.reason, StopReason::Interrupted);
        assert!(recording.any_source_failed());
        assert_eq!(recording.sources[0].phase, SourcePhase::Stopped);
        assert_eq!(recording.sources[1].phase, SourcePhase::Abandoned);
        let mic = read_i16(&dir.join("mic.wav"));
        let system = read_i16(&dir.join("system.wav"));
        assert_eq!(mic.len(), system.len());
        assert!(mic.len() >= 2_400);
        assert_eq!(mic[0], 16_384);
        assert!(all_zero(&system));
        probe.release();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn events_tell_each_phase_change_once_and_in_order() {
        let dir = temp_dir("events");
        let mic =
            FakeSource::from_samples(Speaker::Me, vec![0.5; 960_000]).with_pacing(Pacing::RealTime);
        let options = RecordOptions {
            limit: Some(Duration::from_millis(150)),
            ..RecordOptions::default()
        };
        let (_, events) = record(boxed(vec![mic]), &dir, options);
        assert_eq!(
            events,
            [
                RecordEvent::Phase {
                    speaker: Speaker::Me,
                    phase: SourcePhase::Running
                },
                RecordEvent::Stopping(StopReason::LimitReached),
                RecordEvent::Phase {
                    speaker: Speaker::Me,
                    phase: SourcePhase::Stopped
                },
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
