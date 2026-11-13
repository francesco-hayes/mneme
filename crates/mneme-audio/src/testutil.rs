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
