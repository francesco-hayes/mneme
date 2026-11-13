//! Conversion from whatever a device delivers to 16 kHz mono `f32`.
//!
//! Three steps, each usable on its own and none needing a device:
//!
//! 1. **Sample format**: [`ToF32`] maps the integer and float sample types
//!    devices return onto `f32` in `-1.0..=1.0`.
//! 2. **Downmix**: [`downmix_interleaved`] and [`downmix_planar`] average all
//!    channels of a frame into one.
//! 3. **Resample**: [`StreamResampler`] converts a mono stream from any rate
//!    to [`SAMPLE_RATE`] with a band-limited resampler (the `rubato` crate),
//!    in pieces of any size. Its start-up delay is removed, so output sample
//!    `n` lines up with input time `n / 16000` s to within one sample, and
//!    the output length is the input length scaled by the rate ratio.
//!
//! [`to_16k_mono`] does all three in one call for a buffer already in memory.
//!
//! Downmixing averages every channel. A multi-channel interface with a
//! microphone on one input and silence on the others therefore comes out
//! quieter (by the channel count), but never clipped.

use mneme_core::SAMPLE_RATE;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, SincInterpolationParameters, WindowFunction,
};

use crate::AudioError;

/// A sample type that converts to `f32` in `-1.0..=1.0`.
///
/// Signed integers divide by their most negative value, so the full negative
/// range maps to `-1.0` and the full positive range to just under `1.0`.
/// Unsigned integers are centred on their midpoint first.
pub trait ToF32: Copy {
    /// Converts one sample.
    fn to_f32(self) -> f32;
}

impl ToF32 for f32 {
    #[inline]
    fn to_f32(self) -> f32 {
        self
    }
}

impl ToF32 for f64 {
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32
    }
}

macro_rules! impl_to_f32_signed {
    ($($t:ty),*) => {$(
        impl ToF32 for $t {
            #[inline]
            fn to_f32(self) -> f32 {
                // -MIN overflows the type itself, so compute the scale in f64.
                (self as f64 / -(<$t>::MIN as f64)) as f32
            }
        }
    )*};
}

macro_rules! impl_to_f32_unsigned {
    ($($t:ty),*) => {$(
        impl ToF32 for $t {
            #[inline]
            fn to_f32(self) -> f32 {
                let half = (<$t>::MAX / 2 + 1) as f64;
                ((self as f64 - half) / half) as f32
            }
        }
    )*};
}

impl_to_f32_signed!(i8, i16, i32, i64);
impl_to_f32_unsigned!(u8, u16, u32, u64);

/// Converts a signed 24-bit sample stored in an `i32` (range
/// `-8_388_608..=8_388_607`), as 24-bit devices and WAV files deliver it.
#[inline]
pub fn i24_to_f32(sample: i32) -> f32 {
    sample as f32 / 8_388_608.0
}

/// Converts a signed integer sample of the given bit depth stored in an
/// `i32`, as WAV readers deliver it. `bits` is clamped to `1..=32`.
#[inline]
pub fn int_to_f32(sample: i32, bits: u16) -> f32 {
    let bits = bits.clamp(1, 32);
    (sample as f64 / (1u64 << (bits - 1)) as f64) as f32
}

/// Downmixes interleaved frames to mono by averaging the channels, converting
/// the sample format on the way, and appends the result to `out`.
///
/// `input` holds `channels` samples per frame. A trailing partial frame is
/// ignored. With `channels == 0` nothing is appended.
pub fn downmix_interleaved<T: ToF32>(input: &[T], channels: usize, out: &mut Vec<f32>) {
    match channels {
        0 => {}
        1 => out.extend(input.iter().map(|s| s.to_f32())),
        _ => {
            let scale = 1.0 / channels as f32;
            out.reserve(input.len() / channels);
            out.extend(
                input
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().map(|s| s.to_f32()).sum::<f32>() * scale),
            );
        }
    }
}

/// Downmixes planar (one slice per channel) audio to mono by averaging the
/// channels, and appends the result to `out`.
///
/// Core Audio delivers non-interleaved buffers this way. If the channel
/// slices differ in length, the shortest decides how many frames are used.
pub fn downmix_planar<T: ToF32>(channels: &[&[T]], out: &mut Vec<f32>) {
    let Some(frames) = channels.iter().map(|c| c.len()).min() else {
        return;
    };
    if let [only] = channels {
        out.extend(only.iter().map(|s| s.to_f32()));
        return;
    }
    let scale = 1.0 / channels.len() as f32;
    out.reserve(frames);
    out.extend((0..frames).map(|i| channels.iter().map(|c| c[i].to_f32()).sum::<f32>() * scale));
}

/// Downmixes interleaved frames to a new mono buffer. See
/// [`downmix_interleaved`].
pub fn downmix_to_mono<T: ToF32>(input: &[T], channels: usize) -> Vec<f32> {
    let mut out = Vec::new();
    downmix_interleaved(input, channels, &mut out);
    out
}

/// Resamples a whole mono buffer from `input_rate` to [`SAMPLE_RATE`].
///
/// The output has `ceil(len * 16000 / input_rate)` samples and is time-aligned
/// with the input to within one sample.
pub fn resample_to_16k(mono: &[f32], input_rate: u32) -> Result<Vec<f32>, AudioError> {
    let mut resampler = StreamResampler::new(input_rate)?;
    let mut out = Vec::with_capacity(resampler.expected_output_len(mono.len() as u64) as usize);
    resampler.process(mono, &mut out)?;
    resampler.finish(&mut out)?;
    Ok(out)
}

/// Converts a whole interleaved buffer in any supported sample format, channel
/// count and rate to 16 kHz mono `f32`.
pub fn to_16k_mono<T: ToF32>(
    interleaved: &[T],
    channels: usize,
    input_rate: u32,
) -> Result<Vec<f32>, AudioError> {
    if channels == 0 {
        return Err(AudioError::UnsupportedFormat("zero channels".to_owned()));
    }
    resample_to_16k(&downmix_to_mono(interleaved, channels), input_rate)
}

/// Input frames handed to the resampler per call. About 21 ms at 48 kHz,
/// which bounds the latency the resampler adds to live capture.
const CHUNK_FRAMES: usize = 1024;

/// Largest FFT block (in input frames) the fixed-ratio resampler may need
/// before the sinc resampler is used instead. Every standard device rate
/// stays far below this; only rates that share almost no factor with 16 kHz
/// (for example 44 101 Hz) exceed it.
const MAX_FFT_BLOCK: u32 = 4096;

/// Resamples a mono `f32` stream from a fixed input rate to [`SAMPLE_RATE`],
/// piece by piece.
///
/// Feed input of any length with [`process`](Self::process); output is
/// appended to the buffer you pass as it becomes available. Call
/// [`finish`](Self::finish) at the end of the stream to flush what is still
/// inside. Across one stream, the total output is exactly
/// `ceil(total_input * 16000 / input_rate)` samples, with the resampler's
/// start-up delay removed (in whole samples; under one sample period may
/// remain), so sample positions can be used as a clock.
///
/// At an input rate of 16 kHz the samples pass through untouched.
pub struct StreamResampler {
    input_rate: u32,
    /// `None` when the input is already at 16 kHz.
    inner: Option<Box<dyn Resampler<f32>>>,
    /// Input waiting for a full chunk.
    pending: Vec<f32>,
    /// Scratch space for one output chunk.
    scratch: Vec<f32>,
    /// Start-up delay still to drop from the output, in output samples.
    to_trim: usize,
    /// Input samples accepted since the stream began.
    total_in: u64,
    /// Output samples produced since the stream began, after trimming.
    total_out: u64,
}

impl StreamResampler {
    /// Creates a resampler from `input_rate` Hz to 16 kHz.
    pub fn new(input_rate: u32) -> Result<Self, AudioError> {
        if input_rate == 0 {
            return Err(AudioError::UnsupportedFormat(
                "sample rate of 0 Hz".to_owned(),
            ));
        }
        let inner = if input_rate == SAMPLE_RATE {
            None
        } else {
            Some(build_resampler(input_rate)?)
        };
        let (scratch_len, to_trim) = inner
            .as_ref()
            .map_or((0, 0), |r| (r.output_frames_max(), r.output_delay()));
        Ok(Self {
            input_rate,
            inner,
            pending: Vec::with_capacity(2 * CHUNK_FRAMES),
            scratch: vec![0.0; scratch_len],
            to_trim,
            total_in: 0,
            total_out: 0,
        })
    }

    /// The input rate this resampler was built for, in Hz.
    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    /// How many output samples a stream of `input_len` input samples yields
    /// in total: `ceil(input_len * 16000 / input_rate)`.
    pub fn expected_output_len(&self, input_len: u64) -> u64 {
        (input_len * u64::from(SAMPLE_RATE)).div_ceil(u64::from(self.input_rate))
    }

    /// Feeds input samples and appends any output that is ready to `out`.
    ///
    /// Output lags the input by up to one internal chunk plus the filter
    /// delay; [`finish`](Self::finish) delivers the rest.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), AudioError> {
        self.total_in += input.len() as u64;
        if self.inner.is_none() {
            out.extend_from_slice(input);
            self.total_out += input.len() as u64;
            return Ok(());
        }
        self.pending.extend_from_slice(input);
        let mut consumed = 0;
        loop {
            let needed = self.input_frames_next();
            if self.pending.len() - consumed < needed {
                break;
            }
            let chunk_end = consumed + needed;
            self.run_chunk(consumed..chunk_end, out)?;
            consumed = chunk_end;
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    /// Ends the stream: flushes buffered input and the filter tail to `out`,
    /// bringing the stream's total output to
    /// [`expected_output_len`](Self::expected_output_len), then resets the
    /// resampler so it can start a new stream.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        if self.inner.is_none() {
            self.total_in = 0;
            self.total_out = 0;
            return Ok(());
        }
        let target = self.expected_output_len(self.total_in);
        let start_len = out.len();
        let already_out = self.total_out;
        // Pad with silence: first to complete the partial chunk, then whole
        // chunks until the filter has pushed out everything that belongs to
        // real input.
        while self.total_out < target {
            let needed = self.input_frames_next();
            self.pending.resize(needed, 0.0);
            self.run_chunk(0..needed, out)?;
            self.pending.clear();
        }
        out.truncate(start_len + target.saturating_sub(already_out) as usize);

        if let Some(inner) = self.inner.as_mut() {
            inner.reset();
            self.to_trim = inner.output_delay();
        }
        self.pending.clear();
        self.total_in = 0;
        self.total_out = 0;
        Ok(())
    }

    fn input_frames_next(&self) -> usize {
        self.inner.as_ref().map_or(0, |r| r.input_frames_next())
    }

    /// Runs one chunk of `self.pending` through the resampler and appends the
    /// output, minus any start-up delay still owed, to `out`.
    fn run_chunk(
        &mut self,
        range: std::ops::Range<usize>,
        out: &mut Vec<f32>,
    ) -> Result<(), AudioError> {
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        let frames_in = range.len();
        let needed_out = inner.output_frames_next();
        if self.scratch.len() < needed_out {
            self.scratch.resize(needed_out, 0.0);
        }
        let scratch_frames = self.scratch.len();
        let input = InterleavedSlice::new(&self.pending[range], 1, frames_in)
            .map_err(|e| AudioError::Resample(e.to_string()))?;
        let mut output = InterleavedSlice::new_mut(&mut self.scratch, 1, scratch_frames)
            .map_err(|e| AudioError::Resample(e.to_string()))?;
        let (_, written) = inner
            .process_into_buffer(&input, &mut output, None)
            .map_err(|e| AudioError::Resample(e.to_string()))?;

        let skip = self.to_trim.min(written);
        self.to_trim -= skip;
        out.extend_from_slice(&self.scratch[skip..written]);
        self.total_out += (written - skip) as u64;
        Ok(())
    }
}

impl std::fmt::Debug for StreamResampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamResampler")
            .field("input_rate", &self.input_rate)
            .field("passthrough", &self.inner.is_none())
            .field("total_in", &self.total_in)
            .field("total_out", &self.total_out)
            .finish()
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Picks the resampler for a rate. Device rates are fixed, so the synchronous
/// FFT resampler fits and is both the fastest and the highest quality choice.
/// It needs a block size that is a multiple of `input_rate / gcd`, which is
/// tiny for every standard rate; for odd rates where it would be huge, the
/// sinc resampler handles any ratio.
fn build_resampler(input_rate: u32) -> Result<Box<dyn Resampler<f32>>, AudioError> {
    let construction = |e: rubato::ResamplerConstructionError| AudioError::Resample(e.to_string());
    if input_rate / gcd(input_rate, SAMPLE_RATE) <= MAX_FFT_BLOCK {
        // One sub-chunk: the largest FFT block the chunk allows, which gives
        // the sharpest anti-aliasing filter. The extra delay is trimmed.
        let fft = Fft::<f32>::new_custom(
            input_rate as usize,
            SAMPLE_RATE as usize,
            CHUNK_FRAMES,
            1,
            1,
            WindowFunction::BlackmanHarris2,
            FixedSync::Input,
        )
        .map_err(construction)?;
        Ok(Box::new(fft))
    } else {
        let sinc = Async::<f32>::new_sinc(
            f64::from(SAMPLE_RATE) / f64::from(input_rate),
            1.0,
            &SincInterpolationParameters::default(),
            CHUNK_FRAMES,
            1,
            FixedAsync::Input,
        )
        .map_err(construction)?;
        Ok(Box::new(sinc))
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;
    use crate::testutil::{fit_sine, sine, zero_crossing_frequency};

    #[test]
    fn float_samples_pass_through() {
        assert_eq!(0.25f32.to_f32(), 0.25);
        assert_eq!((-0.5f64).to_f32(), -0.5);
    }

    #[test]
    fn signed_integers_scale_to_unit_range() {
        assert_eq!(0i16.to_f32(), 0.0);
        assert_eq!(i16::MIN.to_f32(), -1.0);
        assert_eq!(16_384i16.to_f32(), 0.5);
        assert!((i16::MAX.to_f32() - 1.0).abs() < 1e-4);
        assert_eq!(i8::MIN.to_f32(), -1.0);
        assert_eq!(64i8.to_f32(), 0.5);
        assert_eq!(i32::MIN.to_f32(), -1.0);
        assert_eq!((1i32 << 30).to_f32(), 0.5);
        assert_eq!(i64::MIN.to_f32(), -1.0);
    }

    #[test]
    fn unsigned_integers_are_centred() {
        assert_eq!(128u8.to_f32(), 0.0);
        assert_eq!(0u8.to_f32(), -1.0);
        assert_eq!(192u8.to_f32(), 0.5);
        assert_eq!(32_768u16.to_f32(), 0.0);
        assert_eq!(0u16.to_f32(), -1.0);
        assert!((u16::MAX.to_f32() - 1.0).abs() < 1e-4);
        assert_eq!((1u32 << 31).to_f32(), 0.0);
        assert_eq!((1u64 << 63).to_f32(), 0.0);
    }

    #[test]
    fn packed_integers_scale_by_bit_depth() {
        assert_eq!(i24_to_f32(-8_388_608), -1.0);
        assert_eq!(i24_to_f32(4_194_304), 0.5);
        assert_eq!(int_to_f32(16_384, 16), 0.5);
        assert_eq!(int_to_f32(-8_388_608, 24), -1.0);
        assert_eq!(int_to_f32(i32::MIN, 32), -1.0);
        assert_eq!(int_to_f32(-128, 8), -1.0);
    }

    #[test]
    fn mono_downmix_is_a_copy() {
        assert_eq!(downmix_to_mono(&[0.1f32, -0.2, 0.3], 1), [0.1, -0.2, 0.3]);
    }

    #[test]
    fn stereo_downmix_averages_each_frame() {
        let stereo = [1.0f32, 0.0, 0.5, 0.5, -1.0, 1.0, 0.25, -0.75];
        assert_eq!(downmix_to_mono(&stereo, 2), [0.5, 0.5, 0.0, -0.25]);
    }

    #[test]
    fn downmix_handles_many_channels_and_integer_formats() {
        // Two frames of four i16 channels.
        let frames: [i16; 8] = [16_384, 16_384, 0, 0, -32_768, -32_768, -32_768, -32_768];
        assert_eq!(downmix_to_mono(&frames, 4), [0.25, -1.0]);
    }

    #[test]
    fn downmix_ignores_a_trailing_partial_frame_and_zero_channels() {
        assert_eq!(downmix_to_mono(&[1.0f32, 1.0, 0.5], 2), [1.0]);
        assert!(downmix_to_mono(&[1.0f32, 1.0], 0).is_empty());
        assert!(downmix_to_mono::<f32>(&[], 2).is_empty());
    }

    #[test]
    fn downmix_appends_to_the_output_buffer() {
        let mut out = vec![9.0];
        downmix_interleaved(&[1.0f32, 0.0], 2, &mut out);
        assert_eq!(out, [9.0, 0.5]);
    }

    #[test]
    fn planar_downmix_matches_interleaved() {
        let left = [1.0f32, 0.5, -1.0];
        let right = [0.0f32, 0.5, 1.0];
        let mut out = Vec::new();
        downmix_planar(&[&left[..], &right[..]], &mut out);
        assert_eq!(out, [0.5, 0.5, 0.0]);

        let mut mono = Vec::new();
        downmix_planar(&[&left[..]], &mut mono);
        assert_eq!(mono, left);

        let mut uneven = Vec::new();
        downmix_planar(&[&left[..], &right[..2]], &mut uneven);
        assert_eq!(uneven, [0.5, 0.5]);

        let mut none = Vec::new();
        downmix_planar::<f32>(&[], &mut none);
        assert!(none.is_empty());
    }

    /// Resamples a tone and checks length, frequency, level, purity and time
    /// alignment of the result.
    fn check_tone(input_rate: u32, freq: f64, seconds: f64) {
        let input = sine(freq, input_rate, (f64::from(input_rate) * seconds) as usize);
        let out = resample_to_16k(&input, input_rate).unwrap();

        let expected_len = (input.len() as u64 * 16_000).div_ceil(u64::from(input_rate)) as usize;
        assert_eq!(out.len(), expected_len, "{input_rate} Hz: output length");

        // Skip 50 ms at each end, where the filter sees the tone start and stop.
        let edge = 800;
        let body = &out[edge..out.len() - edge];

        let measured = zero_crossing_frequency(body, 16_000);
        assert!(
            (measured - freq).abs() < 1.0,
            "{input_rate} Hz: tone at {freq} Hz came out at {measured} Hz"
        );

        let fit = fit_sine(body, freq, 16_000, edge);
        assert!(
            (fit.amplitude - 1.0).abs() < 0.01,
            "{input_rate} Hz: amplitude {}",
            fit.amplitude
        );
        assert!(
            fit.residual_rms < 0.005,
            "{input_rate} Hz: residual {}",
            fit.residual_rms
        );
        // Phase is measured against the input's time origin. The resampler
        // delay is removed in whole output samples, so what remains must be
        // less than one sample period at 16 kHz.
        let one_sample = TAU * freq / 16_000.0;
        assert!(
            fit.phase.abs() < one_sample,
            "{input_rate} Hz: phase offset {} rad, one sample is {one_sample} rad",
            fit.phase
        );
    }

    #[test]
    fn resampling_44100_keeps_tone_frequency_and_length() {
        check_tone(44_100, 1_000.0, 2.0);
        check_tone(44_100, 440.0, 1.0);
    }

    #[test]
    fn resampling_48000_keeps_tone_frequency_and_length() {
        check_tone(48_000, 1_000.0, 2.0);
        check_tone(48_000, 3_000.0, 1.0);
    }

    #[test]
    fn resampling_other_rates_keeps_tone_frequency_and_length() {
        for rate in [8_000, 22_050, 24_000, 32_000, 88_200, 96_000, 192_000] {
            check_tone(rate, 1_000.0, 1.0);
        }
    }

    #[test]
    fn odd_rates_use_the_sinc_fallback() {
        // 44 101 is prime, so the FFT resampler would need a huge block.
        check_tone(44_101, 1_000.0, 1.0);
    }

    #[test]
    fn sixteen_khz_passes_through_bit_exact() {
        let input = sine(700.0, 16_000, 5_000);
        assert_eq!(resample_to_16k(&input, 16_000).unwrap(), input);
    }

    #[test]
    fn frequencies_above_the_new_nyquist_are_removed() {
        // 12 kHz cannot exist at 16 kHz; without a real anti-aliasing filter
        // it would fold down to 4 kHz at full level.
        let input = sine(12_000.0, 48_000, 48_000);
        let out = resample_to_16k(&input, 48_000).unwrap();
        let body = &out[800..out.len() - 800];
        let rms =
            (body.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / body.len() as f64).sqrt();
        assert!(rms < 0.001, "alias level {rms}");
    }

    #[test]
    fn streaming_in_uneven_pieces_matches_one_shot() {
        for rate in [44_100u32, 48_000] {
            let input = sine(1_000.0, rate, rate as usize * 2 + 123);
            let one_shot = resample_to_16k(&input, rate).unwrap();

            let mut resampler = StreamResampler::new(rate).unwrap();
            let mut streamed = Vec::new();
            let sizes = [480usize, 1, 441, 512, 2_048, 7, 1_024, 333];
            let mut pos = 0;
            let mut i = 0;
            while pos < input.len() {
                let end = (pos + sizes[i % sizes.len()]).min(input.len());
                resampler.process(&input[pos..end], &mut streamed).unwrap();
                pos = end;
                i += 1;
            }
            resampler.finish(&mut streamed).unwrap();

            assert_eq!(streamed.len(), one_shot.len());
            let max_diff = streamed
                .iter()
                .zip(&one_shot)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(max_diff < 1e-6, "{rate} Hz: streams differ by {max_diff}");
        }
    }

    #[test]
    fn finish_resets_for_a_new_stream() {
        let input = sine(1_000.0, 48_000, 30_000);
        let mut resampler = StreamResampler::new(48_000).unwrap();
        let mut first = Vec::new();
        resampler.process(&input, &mut first).unwrap();
        resampler.finish(&mut first).unwrap();
        let mut second = Vec::new();
        resampler.process(&input, &mut second).unwrap();
        resampler.finish(&mut second).unwrap();
        assert_eq!(first.len(), 10_000);
        assert_eq!(first, second);
    }

    #[test]
    fn short_and_empty_streams_have_the_expected_length() {
        for (rate, len) in [
            (48_000u32, 0usize),
            (48_000, 1),
            (48_000, 100),
            (44_100, 441),
        ] {
            let out = resample_to_16k(&vec![0.5; len], rate).unwrap();
            let expected = (len as u64 * 16_000).div_ceil(u64::from(rate)) as usize;
            assert_eq!(out.len(), expected, "{len} samples at {rate} Hz");
        }
    }

    #[test]
    fn zero_rate_and_zero_channels_are_rejected() {
        assert!(matches!(
            StreamResampler::new(0),
            Err(AudioError::UnsupportedFormat(_))
        ));
        assert!(matches!(
            to_16k_mono(&[0.0f32; 4], 0, 48_000),
            Err(AudioError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn one_call_converts_stereo_i16_at_48k() {
        // The same 1 kHz tone on both channels, as 16-bit integers.
        let mono = sine(1_000.0, 48_000, 48_000);
        let stereo: Vec<i16> = mono
            .iter()
            .flat_map(|s| {
                let v = (s * 32_767.0) as i16;
                [v, v]
            })
            .collect();
        let out = to_16k_mono(&stereo, 2, 48_000).unwrap();
        assert_eq!(out.len(), 16_000);
        let body = &out[800..out.len() - 800];
        let fit = fit_sine(body, 1_000.0, 16_000, 800);
        assert!((fit.amplitude - 1.0).abs() < 0.01);
        assert!((zero_crossing_frequency(body, 16_000) - 1_000.0).abs() < 1.0);
    }
}
