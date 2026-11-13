//! Runs the chunker on real (synthetic-voice) speech: the fixture from the
//! engine tests with a long pause put between its two sentences.
//!
//! The tests that only look at where the cuts fall need no model. The ones
//! that check no word was clipped transcribe each chunk with the tiny English
//! model and skip with a `SKIPPED` line when it is absent; see
//! `tests/common/mod.rs`.
//!
//! What the fixture holds, measured in 20 ms windows: "Hello, this is a test
//! of the meeting transcription." from 0 to 2860 ms, 220 ms of near silence,
//! then "The quick brown fox jumps over the lazy dog." from 3080 to 5560 ms,
//! and near silence to the end at 5755 ms.

mod common;

use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker};
use mneme_transcribe::{Chunk, Chunker, ChunkerConfig, Language, Transcriber};

use common::{SPEECH_WAV, engine, read_wav};

const SAMPLES_PER_MS: usize = SAMPLE_RATE as usize / 1000;
/// Frames as the audio crate delivers them: 20 ms.
const FRAME_SAMPLES: usize = 20 * SAMPLES_PER_MS;

/// Where the fixture's first sentence has ended, where the second begins,
/// and where that one has ended, in milliseconds.
const FIRST_SENTENCE_END_MS: u64 = 2_860;
const SECOND_SENTENCE_START_MS: u64 = 3_080;
const SECOND_SENTENCE_END_MS: u64 = 5_560;
/// A point inside the near silence between the sentences.
const SPLIT_MS: u64 = 2_960;

/// Quiet before the speech, the pause put between the sentences, and quiet
/// after the speech.
const LEAD_MS: u64 = 2_000;
const PAUSE_MS: u64 = 3_000;
const TAIL_MS: u64 = 1_500;
/// The meeting time of the first frame: ten minutes in.
const BASE_MS: u64 = 600_000;

const FIRST_SENTENCE: &str = "hello this is a test of the meeting transcription";
const SECOND_SENTENCE: &str = "the quick brown fox jumps over the lazy dog";

fn to_samples(ms: u64) -> usize {
    ms as usize * SAMPLES_PER_MS
}

/// Low white noise (RMS 0.0005, -66 dBFS): a quiet room, not digital zero.
fn room_tone(ms: u64, seed: u32) -> Vec<f32> {
    let mut state = seed;
    (0..to_samples(ms))
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let uniform = (state >> 8) as f32 / 8_388_608.0 - 1.0;
            uniform * 0.0005 * 3.0_f32.sqrt()
        })
        .collect()
}

/// Quiet, the first sentence, a long pause, the second sentence, quiet.
fn speech_with_a_long_pause() -> Vec<f32> {
    let speech = read_wav(SPEECH_WAV);
    let split = to_samples(SPLIT_MS);
    // The split must be in silence, or the pause would cut a word.
    let around = &speech[split - to_samples(60)..split + to_samples(60)];
    assert!(
        around.iter().all(|sample| sample.abs() < 0.002),
        "the fixture is not silent at {SPLIT_MS} ms"
    );

    let mut samples = room_tone(LEAD_MS, 1);
    samples.extend_from_slice(&speech[..split]);
    samples.extend(room_tone(PAUSE_MS, 2));
    samples.extend_from_slice(&speech[split..]);
    samples.extend(room_tone(TAIL_MS, 3));
    samples
}

/// Pushes the audio as 20 ms frames starting at [`BASE_MS`], then flushes.
fn chunk_all(samples: &[f32]) -> (Vec<Chunk>, Chunker) {
    let mut chunker = Chunker::new(Speaker::Them, ChunkerConfig::default());
    let mut chunks = Vec::new();
    for (n, frame) in samples.chunks(FRAME_SAMPLES).enumerate() {
        let frame = AudioFrame {
            speaker: Speaker::Them,
            start_ms: BASE_MS + n as u64 * 20,
            samples: frame.to_vec(),
        };
        chunks.extend(chunker.push(&frame));
    }
    chunks.extend(chunker.flush());
    (chunks, chunker)
}

/// The chunk's samples are the input's, at the place its `start_ms` says.
fn assert_cut_from(chunk: &Chunk, samples: &[f32]) {
    let from = to_samples(chunk.start_ms - BASE_MS);
    let to = from + chunk.samples.len();
    assert!(to <= samples.len());
    assert!(
        chunk.samples == samples[from..to],
        "chunk at {} ms does not match the input there",
        chunk.start_ms
    );
}

/// Lowercase words with punctuation removed.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[test]
fn the_fixture_alone_is_one_chunk() {
    // Its gaps (about 200 ms) are far shorter than the pause.
    let speech = read_wav(SPEECH_WAV);

    let (chunks, chunker) = chunk_all(&speech);

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_ms, BASE_MS);
    assert_eq!(chunks[0].speaker, Speaker::Them);
    // Nothing trimmed: the speech starts at once, and the quiet after it is
    // shorter than the padding.
    assert!(chunks[0].samples == speech);
    assert_eq!(chunker.stats().dropped, 0);
}

#[test]
fn a_long_pause_splits_the_speech_in_two() {
    let samples = speech_with_a_long_pause();
    let (chunks, chunker) = chunk_all(&samples);
    let padding = u64::from(chunker.config().padding_ms);

    assert_eq!(chunks.len(), 2, "stats: {:?}", chunker.stats());
    assert_eq!(chunker.stats().dropped, 0);
    assert_eq!(chunker.stats().forced_cuts, 0);
    for chunk in &chunks {
        assert_cut_from(chunk, &samples);
    }

    // Where each sentence sits in the input, on the meeting clock.
    let first_start = BASE_MS + LEAD_MS;
    let first_end = first_start + FIRST_SENTENCE_END_MS;
    let second_start = first_start + PAUSE_MS + SECOND_SENTENCE_START_MS;
    let second_end = first_start + PAUSE_MS + SECOND_SENTENCE_END_MS;

    // Each chunk starts one padding before its sentence, to within a 20 ms
    // window: the leading quiet and the pause are not in the chunks, so
    // `start_ms` says when the speech began.
    let near = |actual: u64, expected: u64| actual.abs_diff(expected) <= 20;
    assert!(
        near(chunks[0].start_ms, first_start - padding),
        "first chunk starts at {}, speech at {first_start}",
        chunks[0].start_ms
    );
    assert!(
        near(chunks[1].start_ms, second_start - padding),
        "second chunk starts at {}, speech at {second_start}",
        chunks[1].start_ms
    );
    // Each chunk runs past the end of its sentence, by no more than the
    // padding (and the fixture's own soft word endings, about 40 ms).
    for (chunk, end) in [(&chunks[0], first_end), (&chunks[1], second_end)] {
        assert!(
            chunk.end_ms() >= end && chunk.end_ms() <= end + padding + 20,
            "chunk ends at {}, speech at {end}",
            chunk.end_ms()
        );
    }
}

#[test]
fn room_noise_neither_hides_the_speech_nor_passes_for_it() {
    // The same audio in a room with a steady hum at -50 dBFS, six times the
    // level of the quiet used elsewhere and above the minimum speech level.
    let mut samples = speech_with_a_long_pause();
    for (sample, hum) in samples.iter_mut().zip(room_tone(60_000, 4)) {
        *sample += hum * 6.0;
    }

    let (chunks, chunker) = chunk_all(&samples);

    assert_eq!(chunks.len(), 2, "stats: {:?}", chunker.stats());
    assert_eq!(chunker.stats().dropped, 0);
    let padding = u64::from(chunker.config().padding_ms);
    let first_start = BASE_MS + LEAD_MS;
    let second_start = first_start + PAUSE_MS + SECOND_SENTENCE_START_MS;
    // The hum covers the softest 20 ms at the start of each sentence, so
    // allow one window more than in a quiet room.
    for (chunk, speech_start) in [(&chunks[0], first_start), (&chunks[1], second_start)] {
        assert!(
            chunk.start_ms.abs_diff(speech_start - padding) <= 40,
            "chunk starts at {}, speech at {speech_start}",
            chunk.start_ms
        );
    }
}

#[test]
fn no_words_are_clipped_at_the_cuts() {
    let Some(engine) = engine("no_words_are_clipped_at_the_cuts") else {
        return;
    };
    let samples = speech_with_a_long_pause();
    let (chunks, _) = chunk_all(&samples);
    assert_eq!(chunks.len(), 2);

    let mut lines = Vec::new();
    for chunk in &chunks {
        let spans = engine
            .transcribe(&chunk.samples, &Language::english())
            .unwrap();
        assert!(
            !spans.is_empty(),
            "no text for the chunk at {}",
            chunk.start_ms
        );
        let segments: Vec<_> = spans
            .into_iter()
            .map(|span| span.into_segment(chunk.speaker, chunk.start_ms))
            .collect();
        lines.push(segments);
    }
    let text = |segments: &[mneme_core::Segment]| {
        segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Every word of each sentence, first and last included, and nothing
    // from the other sentence.
    assert_eq!(words(&text(&lines[0])), words(FIRST_SENTENCE));
    assert_eq!(words(&text(&lines[1])), words(SECOND_SENTENCE));

    // Timestamps as the live session builds them: chunk start plus the
    // offset inside the chunk. Each line starts within a quarter of a second
    // of its sentence. Without the trimming the first would be two seconds
    // early.
    let first_start = BASE_MS + LEAD_MS;
    let second_start = first_start + PAUSE_MS + SECOND_SENTENCE_START_MS;
    for (segments, speech_start) in [(&lines[0], first_start), (&lines[1], second_start)] {
        let line_start = segments[0].start_ms;
        assert!(
            line_start.abs_diff(speech_start) <= 250,
            "line at {line_start} ms, speech at {speech_start} ms"
        );
        assert_eq!(segments[0].speaker, Speaker::Them);
    }
}

#[test]
fn chunks_of_the_whole_fixture_keep_every_word() {
    // With a pause as short as the fixture's own gaps the chunker cuts
    // after "Hello," and between the sentences: three chunks. Even then
    // every word must reach the transcriber, in order.
    let Some(engine) = engine("chunks_of_the_whole_fixture_keep_every_word") else {
        return;
    };
    let speech = read_wav(SPEECH_WAV);
    let config = ChunkerConfig {
        pause_ms: 160,
        ..ChunkerConfig::default()
    };
    let mut chunker = Chunker::new(Speaker::Me, config);
    let mut chunks = Vec::new();
    for (n, frame) in speech.chunks(FRAME_SAMPLES).enumerate() {
        chunks.extend(chunker.push_samples(n as u64 * 20, frame));
    }
    chunks.extend(chunker.flush());
    let spans: Vec<_> = chunks.iter().map(|c| (c.start_ms, c.end_ms())).collect();
    assert_eq!(chunks.len(), 3, "{spans:?}");

    let mut heard = Vec::new();
    for chunk in &chunks {
        let spans = engine
            .transcribe(&chunk.samples, &Language::english())
            .unwrap();
        for span in spans {
            heard.extend(words(&span.text));
        }
    }

    let mut expected = words(FIRST_SENTENCE);
    expected.extend(words(SECOND_SENTENCE));
    assert_eq!(heard, expected);
}
