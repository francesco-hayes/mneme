//! WAV files: a streaming writer for the one format Mneme writes (16 kHz,
//! mono, 16-bit PCM) and a reader that brings any WAV to 16 kHz mono `f32`.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec};
use mneme_core::SAMPLE_RATE;

use crate::AudioError;
use crate::convert::{int_to_f32, to_16k_mono};

/// Converts a sample to 16-bit PCM. Values outside `-1.0..=1.0` are clipped;
/// NaN becomes silence.
///
/// The scale (32 768) is the inverse of the one used when reading, so a
/// sample that came from 16-bit audio is written back unchanged.
#[inline]
pub fn f32_to_i16(sample: f32) -> i16 {
    // A float-to-int `as` cast saturates and maps NaN to 0.
    (sample * 32_768.0).round() as i16
}

/// Writes 16 kHz mono 16-bit PCM from `f32` samples, a piece at a time.
///
/// Samples go to disk through a buffer as they are written; the recording is
/// never held in memory. Call [`finalize`](Self::finalize) at the end to
/// write the final sizes into the header. A writer that is dropped without it
/// still tries, but cannot report a failure.
pub struct WavWriter {
    inner: hound::WavWriter<BufWriter<File>>,
    samples: u64,
}

impl WavWriter {
    /// Creates (or truncates) the file at `path` and writes the header.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let spec = WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        Ok(Self {
            inner: hound::WavWriter::create(path, spec)?,
            samples: 0,
        })
    }

    /// Appends samples at 16 kHz.
    pub fn write(&mut self, samples: &[f32]) -> Result<(), AudioError> {
        // hound's fast path takes the count as u32; stay far below it.
        for block in samples.chunks(1 << 20) {
            let mut writer = self.inner.get_i16_writer(block.len() as u32);
            for sample in block {
                writer.write_sample(f32_to_i16(*sample));
            }
            writer.flush()?;
            self.samples += block.len() as u64;
        }
        Ok(())
    }

    /// Appends `samples` samples of silence, for stretches where a source
    /// delivered nothing.
    pub fn write_silence(&mut self, samples: u64) -> Result<(), AudioError> {
        const ZEROS: [f32; 4096] = [0.0; 4096];
        let mut left = samples;
        while left > 0 {
            let n = left.min(ZEROS.len() as u64) as usize;
            self.write(&ZEROS[..n])?;
            left -= n as u64;
        }
        Ok(())
    }

    /// Number of samples written so far.
    pub fn samples_written(&self) -> u64 {
        self.samples
    }

    /// Length of the audio written so far, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.samples * 1000 / u64::from(SAMPLE_RATE)
    }

    /// Pushes buffered samples to disk and brings the header up to date, so
    /// the file is a valid WAV even if the process dies afterwards.
    pub fn flush(&mut self) -> Result<(), AudioError> {
        Ok(self.inner.flush()?)
    }

    /// Finishes the file: flushes and writes the final header.
    pub fn finalize(self) -> Result<(), AudioError> {
        Ok(self.inner.finalize()?)
    }
}

impl std::fmt::Debug for WavWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WavWriter")
            .field("samples", &self.samples)
            .finish()
    }
}

/// Reads a whole WAV file and converts it to 16 kHz mono `f32`.
///
/// Accepts integer PCM of 8 to 32 bits and 32-bit float, any channel count
/// and any sample rate.
pub fn read_wav_16k_mono(path: impl AsRef<Path>) -> Result<Vec<f32>, AudioError> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        SampleFormat::Int => reader
            .samples::<i32>()
            .map(|s| s.map(|s| int_to_f32(s, spec.bits_per_sample)))
            .collect::<Result<_, _>>()?,
    };
    to_16k_mono(&interleaved, usize::from(spec.channels), spec.sample_rate)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::testutil::{sine, zero_crossing_frequency};

    /// A file path under the build's temp directory, unique per test.
    fn temp_wav(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mneme-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn samples_convert_to_16_bit_with_clipping() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(0.5), 16_384);
        assert_eq!(f32_to_i16(-0.5), -16_384);
        assert_eq!(f32_to_i16(-1.0), i16::MIN);
        assert_eq!(f32_to_i16(1.0), i16::MAX);
        assert_eq!(f32_to_i16(7.5), i16::MAX);
        assert_eq!(f32_to_i16(-7.5), i16::MIN);
        assert_eq!(f32_to_i16(f32::NAN), 0);
        assert_eq!(f32_to_i16(f32::INFINITY), i16::MAX);
    }

    #[test]
    fn wav_round_trip_preserves_format_length_and_samples() {
        let path = temp_wav("round-trip.wav");
        // 1.5 s of a half-scale tone, written in uneven pieces.
        let samples: Vec<f32> = sine(440.0, 16_000, 24_000)
            .iter()
            .map(|s| s * 0.5)
            .collect();
        let mut writer = WavWriter::create(&path).unwrap();
        for piece in samples.chunks(777) {
            writer.write(piece).unwrap();
        }
        assert_eq!(writer.samples_written(), 24_000);
        assert_eq!(writer.duration_ms(), 1500);
        writer.finalize().unwrap();

        // 44-byte header plus two bytes per sample.
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 2 * 24_000);
        let reader = WavReader::open(&path).unwrap();
        assert_eq!(
            reader.spec(),
            WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            }
        );
        assert_eq!(reader.duration(), 24_000);

        let read = read_wav_16k_mono(&path).unwrap();
        assert_eq!(read.len(), samples.len());
        let max_error = read
            .iter()
            .zip(&samples)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // Half of one 16-bit step.
        assert!(max_error <= 0.5 / 32_768.0 + 1e-7, "max error {max_error}");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn rewriting_16_bit_audio_is_lossless() {
        let first = temp_wav("lossless-a.wav");
        let second = temp_wav("lossless-b.wav");
        let mut writer = WavWriter::create(&first).unwrap();
        writer.write(&sine(1_000.0, 16_000, 4_000)).unwrap();
        writer.finalize().unwrap();

        let mut writer = WavWriter::create(&second).unwrap();
        writer.write(&read_wav_16k_mono(&first).unwrap()).unwrap();
        writer.finalize().unwrap();

        assert_eq!(
            std::fs::read(&first).unwrap(),
            std::fs::read(&second).unwrap()
        );
        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();
    }

    #[test]
    fn silence_pads_by_sample_count() {
        let path = temp_wav("silence.wav");
        let mut writer = WavWriter::create(&path).unwrap();
        writer.write_silence(10_000).unwrap();
        writer.write(&[0.25; 100]).unwrap();
        writer.write_silence(0).unwrap();
        assert_eq!(writer.samples_written(), 10_100);
        writer.finalize().unwrap();

        let read = read_wav_16k_mono(&path).unwrap();
        assert_eq!(read.len(), 10_100);
        assert!(read[..10_000].iter().all(|s| *s == 0.0));
        assert!(read[10_000..].iter().all(|s| *s == 0.25));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn flush_leaves_a_valid_file_mid_recording() {
        let path = temp_wav("flushed.wav");
        let mut writer = WavWriter::create(&path).unwrap();
        writer.write(&[0.1; 1_600]).unwrap();
        writer.flush().unwrap();
        // Read while the writer is still open, as after a crash.
        assert_eq!(read_wav_16k_mono(&path).unwrap().len(), 1_600);
        writer.write(&[0.1; 1_600]).unwrap();
        writer.finalize().unwrap();
        assert_eq!(read_wav_16k_mono(&path).unwrap().len(), 3_200);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn reading_converts_other_rates_channels_and_formats() {
        // 48 kHz stereo 24-bit integer.
        let path = temp_wav("stereo-48k-24bit.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for s in sine(1_000.0, 48_000, 48_000) {
            let v = (s * 8_388_607.0) as i32;
            writer.write_sample(v).unwrap();
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();
        let read = read_wav_16k_mono(&path).unwrap();
        assert_eq!(read.len(), 16_000);
        let body = &read[800..15_200];
        assert!((zero_crossing_frequency(body, 16_000) - 1_000.0).abs() < 1.0);
        let peak = body.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 1.0).abs() < 0.01, "peak {peak}");
        std::fs::remove_file(&path).unwrap();

        // 44.1 kHz mono 32-bit float.
        let path = temp_wav("mono-44k-float.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for s in sine(1_000.0, 44_100, 44_100) {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        let read = read_wav_16k_mono(&path).unwrap();
        assert_eq!(read.len(), 16_000);
        assert!((zero_crossing_frequency(&read[800..15_200], 16_000) - 1_000.0).abs() < 1.0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_missing_file_is_a_wav_error() {
        let err = read_wav_16k_mono(temp_wav("does-not-exist.wav")).unwrap_err();
        assert!(matches!(err, AudioError::Wav(_)));
        let err = WavWriter::create("/nonexistent-dir/x.wav").unwrap_err();
        assert!(matches!(err, AudioError::Wav(_)));
    }
}
