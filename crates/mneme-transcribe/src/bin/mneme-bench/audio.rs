//! Reads the WAV file to benchmark and turns it into what the engine takes:
//! mono `f32` samples at 16 kHz.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use mneme_core::SAMPLE_RATE;
use serde::Serialize;

use crate::resample::resample;

/// What the file held before conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    /// `int` or `float`.
    pub sample_format: &'static str,
    /// Samples per channel.
    pub frames: u64,
}

/// A recording ready for the chunker and the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    /// Mono samples at [`SAMPLE_RATE`].
    pub samples: Vec<f32>,
    pub source: SourceFormat,
    /// The channels were averaged into one.
    pub downmixed: bool,
    /// The sample rate was changed.
    pub resampled: bool,
}

impl Audio {
    /// Length in milliseconds, rounded up.
    pub fn duration_ms(&self) -> u64 {
        mneme_transcribe::duration_ms(self.samples.len())
    }
}

/// Opens and converts the WAV file at `path`.
pub fn read_wav_file(path: &Path) -> Result<Audio, String> {
    let file =
        File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    read_wav(BufReader::new(file)).map_err(|error| format!("{}: {error}", path.display()))
}

/// Converts WAV data: any sample rate, any number of channels, 8, 16, 24 or
/// 32-bit integer samples or 32-bit float samples.
///
/// Channels are averaged, then the result is resampled to 16 kHz. Float
/// samples that are not numbers become silence.
pub fn read_wav<R: Read>(reader: R) -> Result<Audio, String> {
    let reader = hound::WavReader::new(reader).map_err(|error| match error {
        hound::Error::Unsupported => "this kind of WAV file is not supported; use 8, 16, 24 or \
             32-bit integer or 32-bit float samples"
            .to_owned(),
        other => format!("not a readable WAV file ({other})"),
    })?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err("the WAV header names no channels or no sample rate".to_owned());
    }

    let read_error = |error: hound::Error| format!("the WAV data cannot be read ({error})");
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .map(|sample| sample.map(|value| if value.is_finite() { value } else { 0.0 }))
            .collect::<Result<_, _>>()
            .map_err(read_error)?,
        hound::SampleFormat::Int => {
            if !(1..=32).contains(&spec.bits_per_sample) {
                return Err(format!(
                    "{}-bit integer samples are not supported",
                    spec.bits_per_sample
                ));
            }
            // Full scale of a signed sample with this many bits.
            let scale = 2.0_f64.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .into_samples::<i32>()
                .map(|sample| sample.map(|value| (f64::from(value) / scale) as f32))
                .collect::<Result<_, _>>()
                .map_err(read_error)?
        }
    };

    let channels = usize::from(spec.channels);
    // A file cut off inside a frame loses that last, incomplete frame.
    let mono: Vec<f32> = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    if mono.is_empty() {
        return Err("the WAV file holds no audio".to_owned());
    }

    let source = SourceFormat {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bits_per_sample: spec.bits_per_sample,
        sample_format: match spec.sample_format {
            hound::SampleFormat::Float => "float",
            hound::SampleFormat::Int => "int",
        },
        frames: mono.len() as u64,
    };
    let resampled = spec.sample_rate != SAMPLE_RATE;
    let samples = if resampled {
        resample(&mono, spec.sample_rate, SAMPLE_RATE)
    } else {
        mono
    };
    Ok(Audio {
        samples,
        source,
        downmixed: channels > 1,
        resampled,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// A WAV file in memory, written by `hound`.
    fn wav<S: hound::Sample + Copy>(
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
        sample_format: hound::SampleFormat,
        samples: &[S],
    ) -> Cursor<Vec<u8>> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
        };
        let mut bytes = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
        bytes.set_position(0);
        bytes
    }

    const INT: hound::SampleFormat = hound::SampleFormat::Int;
    const FLOAT: hound::SampleFormat = hound::SampleFormat::Float;

    #[test]
    fn sixteen_bit_mono_at_16_khz_is_used_as_it_is() {
        let audio = read_wav(wav(
            16_000,
            1,
            16,
            INT,
            &[0_i16, 16_384, -16_384, 32_767, -32_768],
        ))
        .unwrap();
        assert_eq!(
            audio.samples,
            vec![0.0, 0.5, -0.5, 32_767.0 / 32_768.0, -1.0]
        );
        assert!(!audio.resampled);
        assert!(!audio.downmixed);
        assert_eq!(
            audio.source,
            SourceFormat {
                sample_rate: 16_000,
                channels: 1,
                bits_per_sample: 16,
                sample_format: "int",
                frames: 5,
            }
        );
    }

    #[test]
    fn every_integer_width_is_scaled_to_full_scale_one() {
        let half_8 = read_wav(wav(16_000, 1, 8, INT, &[64_i8, -64, -128])).unwrap();
        assert_eq!(half_8.samples, vec![0.5, -0.5, -1.0]);

        let half_24 = read_wav(wav(16_000, 1, 24, INT, &[4_194_304_i32, -8_388_608])).unwrap();
        assert_eq!(half_24.samples, vec![0.5, -1.0]);
        assert_eq!(half_24.source.bits_per_sample, 24);

        let half_32 = read_wav(wav(16_000, 1, 32, INT, &[1_073_741_824_i32, i32::MIN, 0])).unwrap();
        assert_eq!(half_32.samples, vec![0.5, -1.0, 0.0]);
    }

    #[test]
    fn float_samples_are_taken_as_they_are() {
        let audio = read_wav(wav(16_000, 1, 32, FLOAT, &[0.25_f32, -0.75, 1.5])).unwrap();
        assert_eq!(audio.samples, vec![0.25, -0.75, 1.5]);
        assert_eq!(audio.source.sample_format, "float");
    }

    #[test]
    fn float_samples_that_are_not_numbers_become_silence() {
        let audio = read_wav(wav(
            16_000,
            1,
            32,
            FLOAT,
            &[0.5_f32, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5],
        ))
        .unwrap();
        assert_eq!(audio.samples, vec![0.5, 0.0, 0.0, 0.0, -0.5]);
    }

    #[test]
    fn channels_are_averaged() {
        // Left, right pairs.
        let stereo = read_wav(wav(
            16_000,
            2,
            16,
            INT,
            &[16_384_i16, 16_384, 16_384, -16_384, 32_767, 0, 0, -16_384],
        ))
        .unwrap();
        assert_eq!(stereo.samples, vec![0.5, 0.0, 32_767.0 / 65_536.0, -0.25]);
        assert!(stereo.downmixed);
        assert_eq!(stereo.source.channels, 2);
        assert_eq!(stereo.source.frames, 4);

        let three = read_wav(wav(
            16_000,
            3,
            32,
            FLOAT,
            &[0.3_f32, 0.6, 0.9, -0.3, 0.0, 0.3],
        ))
        .unwrap();
        assert_eq!(three.samples.len(), 2);
        assert!((three.samples[0] - 0.6).abs() < 1e-6);
        assert!(three.samples[1].abs() < 1e-6);
    }

    #[test]
    fn other_sample_rates_are_converted_to_16_khz() {
        // One second of a 440 Hz tone at 48 kHz, in stereo, 24-bit.
        let tone: Vec<i32> = (0..48_000)
            .flat_map(|n| {
                let value = (0.5
                    * (std::f64::consts::TAU * 440.0 * f64::from(n) / 48_000.0).sin()
                    * 8_388_608.0) as i32;
                [value, value]
            })
            .collect();
        let audio = read_wav(wav(48_000, 2, 24, INT, &tone)).unwrap();
        assert!(audio.resampled);
        assert!(audio.downmixed);
        assert_eq!(audio.source.sample_rate, 48_000);
        assert_eq!(audio.source.frames, 48_000);
        assert_eq!(audio.samples.len(), 16_000);
        assert_eq!(audio.duration_ms(), 1_000);
        // Still the same tone: compare the middle with a 440 Hz sine at 16 kHz.
        for (n, sample) in audio.samples.iter().enumerate().take(15_000).skip(1_000) {
            let ideal = 0.5 * (std::f64::consts::TAU * 440.0 * n as f64 / 16_000.0).sin();
            assert!((f64::from(*sample) - ideal).abs() < 1e-3, "sample {n}");
        }
    }

    #[test]
    fn a_file_without_audio_is_refused() {
        let error = read_wav(wav::<i16>(16_000, 1, 16, INT, &[])).unwrap_err();
        assert!(error.contains("no audio"), "{error}");
    }

    #[test]
    fn something_that_is_not_a_wav_file_is_refused_with_a_reason() {
        let error =
            read_wav(Cursor::new(b"ID3 this is not a wave file at all".to_vec())).unwrap_err();
        assert!(error.contains("WAV"), "{error}");
    }

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let error = read_wav_file(Path::new("/nonexistent/mneme/recording.wav")).unwrap_err();
        assert!(
            error.contains("/nonexistent/mneme/recording.wav"),
            "{error}"
        );
    }

    #[test]
    fn the_speech_fixture_reads_as_five_and_three_quarter_seconds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/speech.wav");
        let audio = read_wav_file(Path::new(path)).unwrap();
        assert_eq!(audio.source.sample_rate, 16_000);
        assert_eq!(audio.source.channels, 1);
        assert!((5_700..=5_800).contains(&audio.duration_ms()));
        assert!(audio.samples.iter().all(|s| (-1.0..=1.0).contains(s)));
    }
}
