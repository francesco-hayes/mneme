//! Cuts the recording into the pieces handed to the engine, and notes when
//! each piece would have become available in a live meeting.

use mneme_core::{SAMPLE_RATE, Speaker};
use mneme_transcribe::{Chunker, ChunkerConfig, ChunkerStats, duration_ms};

/// The live sources deliver audio in frames of this length (`FRAME_MS` in
/// the capture crate), so the chunker is fed the same way here.
pub const FRAME_MS: u64 = 20;
const FRAME_SAMPLES: usize = (SAMPLE_RATE as u64 * FRAME_MS / 1000) as usize;

/// Length of the windows of the `--fixed-windows` baseline: the chunker's
/// default maximum chunk length.
pub const FIXED_WINDOW_MS: u64 = 28_000;

/// One piece of audio for the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// Meeting time of the first sample.
    pub start_ms: u64,
    /// Meeting time at which a live session would have had this piece in its
    /// queue: when the audio that completed it had been captured.
    pub available_at_ms: u64,
    pub samples: Vec<f32>,
}

impl Piece {
    pub fn duration_ms(&self) -> u64 {
        duration_ms(self.samples.len())
    }
}

/// Runs the recording through the silence chunker the way a live session
/// does: 20 ms frames in order, then a flush at the end.
///
/// A chunk the chunker returns for a frame was available once that frame had
/// been captured, so its `available_at_ms` is the end of that frame. The
/// chunk still open at the end comes out of the flush, at the end of the
/// recording.
pub fn cut_with_chunker(samples: &[f32], config: ChunkerConfig) -> (Vec<Piece>, ChunkerStats) {
    let total_ms = duration_ms(samples.len());
    let mut chunker = Chunker::new(Speaker::Me, config);
    let mut pieces = Vec::new();

    for (index, frame) in samples.chunks(FRAME_SAMPLES).enumerate() {
        let start_ms = index as u64 * FRAME_MS;
        let frame_end_ms = (start_ms + FRAME_MS).min(total_ms);
        for chunk in chunker.push_samples(start_ms, frame) {
            pieces.push(Piece {
                start_ms: chunk.start_ms,
                available_at_ms: frame_end_ms,
                samples: chunk.samples,
            });
        }
    }
    if let Some(chunk) = chunker.flush() {
        pieces.push(Piece {
            start_ms: chunk.start_ms,
            available_at_ms: total_ms,
            samples: chunk.samples,
        });
    }
    (pieces, chunker.stats())
}

/// The baseline without the chunker: consecutive windows of `window_ms`,
/// silence included, each available when its last sample has been captured.
pub fn cut_fixed_windows(samples: &[f32], window_ms: u64) -> Vec<Piece> {
    let window_samples = (u64::from(SAMPLE_RATE) * window_ms / 1000).max(1) as usize;
    let total_ms = duration_ms(samples.len());
    samples
        .chunks(window_samples)
        .enumerate()
        .map(|(index, window)| {
            let start_ms = index as u64 * window_ms;
            Piece {
                start_ms,
                available_at_ms: (start_ms + duration_ms(window.len())).min(total_ms),
                samples: window.to_vec(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording made of stretches of near silence and stretches of
    /// "speech": a 220 Hz tone in syllables of 240 ms with 60 ms dips, which
    /// the chunker takes for unbroken speech.
    fn recording(parts: &[(u64, bool)]) -> Vec<f32> {
        let per_ms = (SAMPLE_RATE / 1000) as usize;
        let mut samples = Vec::new();
        for (length_ms, speech) in parts {
            for i in 0..*length_ms as usize * per_ms {
                let n = samples.len() as f32;
                // A deterministic hiss far below the speech threshold.
                let hiss = 0.0003 * ((n * 12.9898).sin() * 43_758.547).fract();
                let in_syllable = (i / per_ms) % 300 < 240;
                let voice = if *speech && in_syllable {
                    0.3 * (n * std::f32::consts::TAU * 220.0 / SAMPLE_RATE as f32).sin()
                } else {
                    0.0
                };
                samples.push(hiss + voice);
            }
        }
        samples
    }

    #[test]
    fn a_chunk_is_available_when_its_pause_has_passed() {
        // Speech from 1.0 to 2.0 s and from 5.0 to 6.0 s; the recording ends
        // at 6.2 s, before the second pause is over.
        let samples = recording(&[
            (1_000, false),
            (1_000, true),
            (3_000, false),
            (1_000, true),
            (200, false),
        ]);
        let (pieces, stats) = cut_with_chunker(&samples, ChunkerConfig::default());

        assert_eq!(pieces.len(), 2);
        assert_eq!(stats.chunks, 2);

        // Default settings: 200 ms of padding, 700 ms of pause.
        assert_eq!(pieces[0].start_ms, 800);
        assert_eq!(pieces[0].duration_ms(), 1_400);
        // Known only once 700 ms without speech have gone by after 2.0 s.
        assert_eq!(pieces[0].available_at_ms, 2_700);

        // The second chunk is still open when the recording stops, so it
        // comes from the flush, at the end of the recording.
        assert_eq!(pieces[1].start_ms, 4_800);
        assert_eq!(pieces[1].available_at_ms, 6_200);
    }

    #[test]
    fn no_chunk_is_available_before_its_last_sample() {
        let samples = recording(&[
            (300, false),
            (2_500, true),
            (900, false),
            (40_000, true),
            (1_500, false),
            (700, true),
            (10, false),
        ]);
        let total_ms = duration_ms(samples.len());
        let (pieces, stats) = cut_with_chunker(&samples, ChunkerConfig::default());

        assert!(stats.forced_cuts >= 1, "40 s of speech needs a forced cut");
        assert_eq!(pieces.len() as u64, stats.chunks);
        let mut previous = 0;
        for piece in &pieces {
            assert!(piece.available_at_ms >= piece.start_ms + piece.duration_ms());
            assert!(piece.available_at_ms <= total_ms);
            assert!(piece.available_at_ms >= previous, "pieces come in order");
            previous = piece.available_at_ms;
        }
        assert_eq!(pieces.last().unwrap().available_at_ms, total_ms);
    }

    #[test]
    fn the_pieces_are_the_chunks_the_chunker_gives_for_the_whole_recording() {
        let samples = recording(&[
            (500, false),
            (3_000, true),
            (2_000, false),
            (31_000, true),
            (1_000, false),
        ]);
        let (pieces, stats) = cut_with_chunker(&samples, ChunkerConfig::default());

        let mut chunker = Chunker::new(Speaker::Me, ChunkerConfig::default());
        let mut expected = chunker.push_samples(0, &samples);
        expected.extend(chunker.flush());

        assert_eq!(stats, chunker.stats());
        assert_eq!(pieces.len(), expected.len());
        for (piece, chunk) in pieces.iter().zip(&expected) {
            assert_eq!(piece.start_ms, chunk.start_ms);
            assert_eq!(piece.samples, chunk.samples);
        }
    }

    #[test]
    fn fixed_windows_cover_the_recording_once_and_in_order() {
        // 70 s in 28 s windows: 28, 28 and 14 s.
        let samples: Vec<f32> = (0..70 * SAMPLE_RATE).map(|n| n as f32).collect();
        let pieces = cut_fixed_windows(&samples, FIXED_WINDOW_MS);

        let lengths: Vec<u64> = pieces.iter().map(Piece::duration_ms).collect();
        assert_eq!(lengths, vec![28_000, 28_000, 14_000]);
        let starts: Vec<u64> = pieces.iter().map(|piece| piece.start_ms).collect();
        assert_eq!(starts, vec![0, 28_000, 56_000]);
        let available: Vec<u64> = pieces.iter().map(|piece| piece.available_at_ms).collect();
        assert_eq!(available, vec![28_000, 56_000, 70_000]);

        let joined: Vec<f32> = pieces.iter().flat_map(|p| p.samples.clone()).collect();
        assert_eq!(joined, samples);
    }

    #[test]
    fn fixed_windows_of_nothing_are_nothing() {
        assert!(cut_fixed_windows(&[], FIXED_WINDOW_MS).is_empty());
    }
}
