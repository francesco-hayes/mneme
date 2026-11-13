//! Runs the real Whisper engine on the speech fixture.
//!
//! These tests need a model file, which is never committed; see
//! `tests/common/mod.rs` for where they look for it and for the fixture.
//! Without a model each test prints a `SKIPPED` line and passes.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::thread;

use mneme_core::SAMPLE_RATE;
use mneme_transcribe::{
    Language, TextSpan, TranscribeError, Transcriber, WhisperEngine, WhisperOptions, duration_ms,
};

use common::{SPEECH_WAV, engine, engine_with, joined_lowercase, read_wav};

/// Words the tiny English model must find in the fixture.
const EXPECTED_PHRASES: [&str; 4] = ["hello", "meeting", "quick brown fox", "lazy dog"];

fn assert_expected_words(spans: &[TextSpan]) {
    let text = joined_lowercase(spans);
    for phrase in EXPECTED_PHRASES {
        assert!(text.contains(phrase), "{phrase:?} is missing from {text:?}");
    }
}

/// Spans are trimmed, in order, and inside the slice.
fn assert_well_formed(spans: &[TextSpan], sample_count: usize) {
    let slice_ms = duration_ms(sample_count);
    let mut previous_start = 0;
    for span in spans {
        assert!(!span.text.is_empty());
        assert_eq!(span.text, span.text.trim());
        assert!(span.start_ms <= span.end_ms, "{span:?}");
        assert!(span.end_ms <= slice_ms, "{span:?} ends after {slice_ms} ms");
        assert!(span.start_ms >= previous_start, "{span:?} is out of order");
        previous_start = span.start_ms;
    }
}

#[test]
fn speech_fixture_is_what_the_tests_expect() {
    // Runs with or without a model.
    let samples = read_wav(SPEECH_WAV);
    let seconds = samples.len() as f64 / f64::from(SAMPLE_RATE);
    assert!((5.0..7.0).contains(&seconds), "fixture is {seconds} s long");
    let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    assert!(peak > 0.1 && peak <= 1.0, "fixture peak is {peak}");
}

#[test]
fn transcribes_the_speech_fixture() {
    let Some(engine) = engine("transcribes_the_speech_fixture") else {
        return;
    };
    let samples = read_wav(SPEECH_WAV);

    let spans = engine.transcribe(&samples, &Language::english()).unwrap();

    assert_expected_words(&spans);
    assert_well_formed(&spans, samples.len());
    let first = spans.first().unwrap();
    let last = spans.last().unwrap();
    assert!(first.start_ms < 1_000, "speech starts at once: {first:?}");
    assert!(last.end_ms > 4_000, "speech runs to the end: {last:?}");
}

#[test]
fn english_only_model_takes_auto_as_english() {
    let Some(engine) = engine("english_only_model_takes_auto_as_english") else {
        return;
    };
    if engine.is_multilingual() {
        return; // MNEME_TEST_MODEL points at a multilingual model.
    }
    let samples = read_wav(SPEECH_WAV);

    assert_eq!(engine.check_language(&Language::auto()), Ok(()));
    let spans = engine.transcribe(&samples, &Language::auto()).unwrap();

    assert_expected_words(&spans);
}

#[test]
fn english_only_model_rejects_another_language() {
    let Some(engine) = engine("english_only_model_rejects_another_language") else {
        return;
    };
    if engine.is_multilingual() {
        return; // MNEME_TEST_MODEL points at a multilingual model.
    }
    let german: Language = "de".parse().unwrap();
    let expected = TranscribeError::EnglishOnlyModel {
        requested: "de".to_owned(),
    };

    assert_eq!(engine.check_language(&german), Err(expected.clone()));
    let samples = read_wav(SPEECH_WAV);
    assert_eq!(engine.transcribe(&samples, &german), Err(expected));
    // The refusal leaves the engine usable.
    let spans = engine.transcribe(&samples, &Language::english()).unwrap();
    assert_expected_words(&spans);
}

#[test]
fn one_engine_serves_many_calls_and_threads() {
    let Some(engine) = engine("one_engine_serves_many_calls_and_threads") else {
        return;
    };
    let samples = Arc::new(read_wav(SPEECH_WAV));
    let shared: Arc<dyn Transcriber> = Arc::new(engine);

    let first = shared.transcribe(&samples, &Language::english()).unwrap();
    assert_expected_words(&first);

    let handles: Vec<_> = (0..3)
        .map(|_| {
            let transcriber = Arc::clone(&shared);
            let samples = Arc::clone(&samples);
            thread::spawn(move || transcriber.transcribe(&samples, &Language::english()))
        })
        .collect();
    for handle in handles {
        // Same audio, same model, no carried-over context: same result.
        assert_eq!(handle.join().unwrap().unwrap(), first);
    }
}

#[test]
fn cpu_only_engine_transcribes_too() {
    let options = WhisperOptions {
        use_gpu: false,
        ..WhisperOptions::default()
    };
    let Some(engine) = engine_with("cpu_only_engine_transcribes_too", &options) else {
        return;
    };
    let samples = read_wav(SPEECH_WAV);

    let spans = engine.transcribe(&samples, &Language::english()).unwrap();

    assert_expected_words(&spans);
    assert_well_formed(&spans, samples.len());
}

#[test]
fn silence_gives_no_text() {
    let Some(engine) = engine("silence_gives_no_text") else {
        return;
    };
    let silence = vec![0.0_f32; 3 * SAMPLE_RATE as usize];

    let spans = engine.transcribe(&silence, &Language::english()).unwrap();

    // Whisper answers digital silence with a blank-audio marker, which the
    // engine drops.
    assert_eq!(spans, []);
}

#[test]
fn empty_and_very_short_slices_give_no_text() {
    let Some(engine) = engine("empty_and_very_short_slices_give_no_text") else {
        return;
    };
    let language = Language::english();

    assert_eq!(engine.transcribe(&[], &language), Ok(Vec::new()));
    // whisper.cpp does not process less than 100 ms of audio.
    let fifty_ms = vec![0.0_f32; SAMPLE_RATE as usize / 20];
    assert_eq!(engine.transcribe(&fifty_ms, &language), Ok(Vec::new()));
}

#[test]
fn leading_silence_does_not_lose_the_speech() {
    let Some(engine) = engine("leading_silence_does_not_lose_the_speech") else {
        return;
    };
    // Two seconds of silence, then the fixture.
    let speech = read_wav(SPEECH_WAV);
    let mut samples = vec![0.0_f32; 2 * SAMPLE_RATE as usize];
    samples.extend_from_slice(&speech);

    let spans = engine.transcribe(&samples, &Language::english()).unwrap();

    assert_expected_words(&spans);
    assert_well_formed(&spans, samples.len());
    // Offsets are measured from the start of the slice handed in, so the
    // speech ends later than it does in the fixture alone. Whisper's segment
    // start is coarse, though: it puts the first span at 0, inside the
    // silence, so nothing is asserted about where the speech starts.
    let last = spans.last().unwrap();
    assert!(last.end_ms > 6_000, "speech runs to the end: {last:?}");
}

#[test]
fn a_file_that_is_not_a_model_is_a_load_error() {
    // Runs with or without a model.
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("not-a-model.bin");
    std::fs::write(&path, b"this is not a whisper model").unwrap();

    let error = WhisperEngine::load(&path).unwrap_err();

    assert!(
        matches!(&error, TranscribeError::ModelLoad { path: p, .. } if *p == path),
        "{error:?}"
    );
    assert!(error.to_string().contains("not-a-model.bin"), "{error}");
    std::fs::remove_file(&path).unwrap();
}
