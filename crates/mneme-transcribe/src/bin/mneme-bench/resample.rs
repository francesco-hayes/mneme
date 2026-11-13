//! Sample-rate conversion for the benchmark's input file.
//!
//! A band-limited (windowed-sinc) resampler for a whole file at once. Each
//! output sample is a weighted sum of the input samples around its position;
//! the weights are a sinc low-pass filter under a Kaiser window. The ratio of
//! the two rates is kept as an exact fraction, so output positions never
//! drift, and the weights are computed once per fractional position
//! (a polyphase table).
//!
//! Why this and not something simpler: dropping samples or interpolating
//! linearly folds everything above the new Nyquist frequency back into the
//! speech band (a 48 kHz recording has plenty up there), which Whisper then
//! hears as noise. Why not the `rubato` crate, which the capture crate uses:
//! the benchmark converts one file offline, which takes about a hundred lines
//! and no dependency; `rubato` would add an FFT stack to this crate's build
//! for the benefit of one command-line tool.
//!
//! What the filter does, measured by the tests below: a tone up to 6.8 kHz
//! comes through a conversion to 16 kHz within 0.1 dB, and anything at or
//! above the new Nyquist frequency is at least 70 dB down.

/// The filter reaches this many zero crossings of the sinc to each side.
/// More is sharper and slower.
const ZERO_CROSSINGS: f64 = 32.0;

/// Kaiser window shape for about 80 dB of stop-band attenuation
/// (`0.1102 * (80 - 8.7)`).
const KAISER_BETA: f64 = 7.857;

/// The cut-off as a share of the lower of the two Nyquist frequencies.
///
/// With this window and length the filter needs a band about 15.7% of the
/// cut-off wide to go from passing to blocking, centred on the cut-off.
/// Putting the cut-off at 92.7% makes that band end at the Nyquist frequency,
/// so nothing that would alias gets through. For 16 kHz output: flat to about
/// 6.8 kHz, blocked from 8 kHz.
const ROLLOFF: f64 = 0.927;

/// Converts `input` from `from_rate` to `to_rate` samples per second.
///
/// The result has `ceil(input.len() * to_rate / from_rate)` samples, and
/// output sample `n` sits at the time of input position `n * from_rate /
/// to_rate`: nothing is delayed. Audio before the start and after the end of
/// `input` counts as silence. Equal rates return the input unchanged.
///
/// # Panics
///
/// If either rate is zero.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    assert!(
        from_rate > 0 && to_rate > 0,
        "sample rates must be positive"
    );
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    // to_rate / from_rate = up / down, in lowest terms.
    let divisor = gcd(u64::from(from_rate), u64::from(to_rate));
    let up = u64::from(to_rate) / divisor;
    let down = u64::from(from_rate) / divisor;

    // The cut-off, as a share of the input's Nyquist frequency. Going down in
    // rate it must sit under the output's Nyquist frequency instead.
    let cutoff = ROLLOFF * (up as f64 / down as f64).min(1.0);
    // How far the filter reaches to each side, in input samples.
    let reach = ZERO_CROSSINGS / cutoff;
    let half = reach.ceil() as usize;
    let taps = 2 * half;

    // weights[phase * taps + k] is the weight of input sample
    // `base - half + 1 + k` for an output sample that sits `phase / up` of
    // the way from input sample `base` to the next one.
    let phases = up as usize;
    let mut weights = vec![0.0_f32; phases * taps];
    for phase in 0..phases {
        let fraction = phase as f64 / up as f64;
        let row = &mut weights[phase * taps..(phase + 1) * taps];
        let mut sum = 0.0;
        let mut exact = vec![0.0_f64; taps];
        for (k, weight) in exact.iter_mut().enumerate() {
            let distance = k as f64 - (half as f64 - 1.0) - fraction;
            *weight = cutoff * sinc(cutoff * distance) * kaiser(distance / reach);
            sum += *weight;
        }
        // Each row sums to exactly one, so a constant level comes through
        // unchanged whatever the fractional position.
        for (weight, exact) in row.iter_mut().zip(&exact) {
            *weight = (exact / sum) as f32;
        }
    }

    let output_len = (input.len() as u64 * up).div_ceil(down) as usize;
    let mut output = Vec::with_capacity(output_len);
    for n in 0..output_len as u64 {
        let position = n * down;
        let base = (position / up) as i64;
        let phase = (position % up) as usize;
        let row = &weights[phase * taps..(phase + 1) * taps];

        let first = base - half as i64 + 1;
        // The part of the row whose input samples exist.
        let skip = (-first).clamp(0, taps as i64) as usize;
        let start = (first.max(0) as usize).min(input.len());
        let end = ((first + taps as i64).max(0) as usize).min(input.len());
        let mut sum = 0.0_f64;
        for (sample, weight) in input[start..end].iter().zip(&row[skip..]) {
            sum += f64::from(*sample) * f64::from(*weight);
        }
        output.push(sum as f32);
    }
    output
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// `sin(pi x) / (pi x)`.
fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let t = std::f64::consts::PI * x;
        t.sin() / t
    }
}

/// The Kaiser window at `x`, where the window spans `-1.0..=1.0`.
fn kaiser(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        0.0
    } else {
        bessel_i0(KAISER_BETA * (1.0 - x * x).sqrt()) / bessel_i0(KAISER_BETA)
    }
}

/// The modified Bessel function of the first kind, order zero, by its power
/// series.
fn bessel_i0(x: f64) -> f64 {
    let quarter = x * x / 4.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..200 {
        term *= quarter / (k * k) as f64;
        sum += term;
        if term < sum * 1e-16 {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;

    /// A sine of `frequency` Hz, `seconds` long, at `rate` samples a second.
    fn tone(frequency: f64, amplitude: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let count = (seconds * f64::from(rate)).round() as usize;
        (0..count)
            .map(|n| (amplitude * (TAU * frequency * n as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    /// The samples away from the two ends, where the filter overlaps the
    /// silence outside the recording.
    fn interior(samples: &[f32], rate: u32) -> &[f32] {
        let margin = rate as usize / 20;
        &samples[margin..samples.len() - margin]
    }

    fn rms(samples: &[f32]) -> f64 {
        let sum: f64 = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
        (sum / samples.len() as f64).sqrt()
    }

    fn decibels(ratio: f64) -> f64 {
        20.0 * ratio.log10()
    }

    /// Largest difference between `output` and the same tone generated at the
    /// output rate, away from the ends.
    fn worst_error(output: &[f32], frequency: f64, amplitude: f64, rate: u32) -> f64 {
        let margin = rate as usize / 20;
        (margin..output.len() - margin)
            .map(|n| {
                let ideal = amplitude * (TAU * frequency * n as f64 / f64::from(rate)).sin();
                (f64::from(output[n]) - ideal).abs()
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn equal_rates_return_the_input_untouched() {
        let input = tone(440.0, 0.5, 16_000, 0.25);
        assert_eq!(resample(&input, 16_000, 16_000), input);
        assert_eq!(resample(&input, 48_000, 48_000), input);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn output_length_follows_the_rate_ratio_rounded_up() {
        for (from, input_len, expected) in [
            (48_000, 48_000, 16_000),
            (48_000, 48_001, 16_001),
            (44_100, 44_100, 16_000),
            (44_100, 441, 160),
            (44_100, 442, 161),
            (22_050, 22_050, 16_000),
            (8_000, 8_000, 16_000),
            (96_000, 96_000 * 3, 48_000),
            (16_001, 16_001, 16_000),
        ] {
            let input = vec![0.0; input_len];
            assert_eq!(
                resample(&input, from, 16_000).len(),
                expected,
                "{input_len} samples at {from} Hz"
            );
        }
    }

    #[test]
    fn a_tone_keeps_its_frequency_level_and_timing_going_down() {
        // Compared sample by sample with the same tone made at 16 kHz, so a
        // wrong ratio, a delay or a gain error all show up.
        for (from, frequency) in [
            (48_000, 1_000.0),
            (44_100, 3_000.0),
            (22_050, 440.0),
            (32_000, 5_000.0),
            (96_000, 2_500.0),
            (24_000, 150.0),
        ] {
            let output = resample(&tone(frequency, 0.5, from, 1.0), from, 16_000);
            let error = worst_error(&output, frequency, 0.5, 16_000);
            assert!(
                error < 5e-4,
                "{frequency} Hz from {from} Hz: off by up to {error}"
            );
        }
    }

    #[test]
    fn a_tone_keeps_its_frequency_level_and_timing_going_up() {
        for (from, frequency) in [(8_000, 1_000.0), (11_025, 2_000.0), (12_000, 300.0)] {
            let output = resample(&tone(frequency, 0.5, from, 1.0), from, 16_000);
            let error = worst_error(&output, frequency, 0.5, 16_000);
            assert!(
                error < 5e-4,
                "{frequency} Hz from {from} Hz: off by up to {error}"
            );
        }
    }

    #[test]
    fn the_speech_band_comes_through_at_full_level() {
        // Up to 6.8 kHz the level changes by less than 0.1 dB.
        for frequency in [100.0, 1_000.0, 4_000.0, 6_000.0, 6_800.0] {
            for from in [48_000, 44_100] {
                let output = resample(&tone(frequency, 0.5, from, 1.0), from, 16_000);
                let change = decibels(rms(interior(&output, 16_000)) / (0.5 / 2.0_f64.sqrt()));
                assert!(
                    change.abs() < 0.1,
                    "{frequency} Hz from {from} Hz changed by {change:.3} dB"
                );
            }
        }
    }

    #[test]
    fn sound_above_the_new_nyquist_frequency_is_removed_not_folded_back() {
        // Without the low-pass filter a 10 kHz tone at 48 kHz turns into a
        // 6 kHz tone at 16 kHz at full level. Here it must be 70 dB down.
        for (from, frequency) in [
            (48_000, 8_000.0),
            (48_000, 10_000.0),
            (48_000, 15_000.0),
            (48_000, 23_000.0),
            (44_100, 9_000.0),
            (44_100, 20_000.0),
            (96_000, 30_000.0),
            (22_050, 9_500.0),
        ] {
            let output = resample(&tone(frequency, 0.5, from, 1.0), from, 16_000);
            let level = decibels(rms(interior(&output, 16_000)) / (0.5 / 2.0_f64.sqrt()));
            assert!(
                level < -70.0,
                "{frequency} Hz from {from} Hz is only {level:.1} dB down"
            );
        }
    }

    #[test]
    fn a_constant_level_stays_constant() {
        for from in [48_000, 44_100, 8_000, 22_050] {
            let output = resample(&vec![0.25; from as usize], from, 16_000);
            for sample in interior(&output, 16_000) {
                assert!(
                    (sample - 0.25).abs() < 1e-6,
                    "from {from} Hz: {sample} is not 0.25"
                );
            }
        }
    }

    #[test]
    fn a_click_stays_at_its_time() {
        // One second in, whatever the input rate.
        for from in [48_000_u32, 44_100, 8_000, 22_050] {
            let mut input = vec![0.0; 2 * from as usize];
            input[from as usize] = 1.0;
            let output = resample(&input, from, 16_000);
            let loudest = output
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(index, _)| index)
                .unwrap();
            assert_eq!(loudest, 16_000, "from {from} Hz");
        }
    }

    #[test]
    fn silence_stays_silent_and_the_ends_do_not_ring_loudly() {
        assert!(
            resample(&vec![0.0; 4_800], 48_000, 16_000)
                .iter()
                .all(|s| *s == 0.0)
        );
        // A tone that starts and stops abruptly must not overshoot much.
        let output = resample(&tone(1_000.0, 0.5, 48_000, 0.5), 48_000, 16_000);
        let peak = output.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!(peak < 0.6, "peak {peak}");
    }

    #[test]
    fn speech_survives_a_trip_to_48_khz_and_back() {
        // The speech fixture, up to 48 kHz and down again, against itself.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/speech.wav");
        let original: Vec<f32> = hound::WavReader::open(path)
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect();

        let up = resample(&original, 16_000, 48_000);
        assert_eq!(up.len(), original.len() * 3);
        let back = resample(&up, 48_000, 16_000);
        assert_eq!(back.len(), original.len());

        let difference: Vec<f32> = original.iter().zip(&back).map(|(a, b)| a - b).collect();
        // What differs is the top of the band (above 6.8 kHz), which the
        // filter gives up to keep aliasing out.
        let lost = decibels(rms(&difference) / rms(&original));
        assert!(lost < -30.0, "the round trip differs by {lost:.1} dB");
    }
}
