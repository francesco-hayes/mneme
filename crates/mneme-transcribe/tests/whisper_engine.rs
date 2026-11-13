//! Runs the real Whisper engine on the speech fixture.
//!
//! These tests need a model file, which is never committed. They look for
//!
//! 1. the path in the `MNEME_TEST_MODEL` environment variable, else
//! 2. `crates/mneme-transcribe/models/ggml-tiny.en.bin`, which
//!    `crates/mneme-transcribe/scripts/fetch-test-model.sh` downloads.
//!
//! When neither is there, each test prints a `SKIPPED` line and passes.
//!
//! The fixture `tests/data/speech.wav` is 5.75 s of synthetic speech (see the
//! `.gitignore` beside it for how it was made). It says: "Hello, this is a
//! test of the meeting transcription. The quick brown fox jumps over the lazy
//! dog."

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use mneme_core::SAMPLE_RATE;
use mneme_transcribe::{
    Language, TextSpan, TranscribeError, Transcriber, WhisperEngine, WhisperOptions, duration_ms,
};

const DEFAULT_MODEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/models/ggml-tiny.en.bin");
const SPEECH_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/speech.wav");

/// Words the tiny English model must find in the fixture.
const EXPECTED_PHRASES: [&str; 4] = ["hello", "meeting", "quick brown fox", "lazy dog"];

fn model_path() -> Option<PathBuf> {
    let path = match std::env::var_os("MNEME_TEST_MODEL") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(DEFAULT_MODEL),
    };
    path.is_file().then_some(path)
}

/// Loads the test model, or returns `None` (after printing why) when there is
/// no model file.
///
/// Each test loads its own engine and drops it when it returns. An engine
/// must not be kept in a `static`: whisper.cpp's Metal backend aborts the
/// process at exit if a loaded model is still alive then.
fn engine(test: &str) -> Option<WhisperEngine> {
    engine_with(test, &WhisperOptions::default())
}

fn engine_with(test: &str, options: &WhisperOptions) -> Option<WhisperEngine> {
    let Some(path) = model_path() else {
        // Written straight to stderr so it shows without `--nocapture`.
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED {test}: no Whisper model file. Run \
             crates/mneme-transcribe/scripts/fetch-test-model.sh, or set MNEME_TEST_MODEL \
             to a ggml-tiny.en.bin file."
        );
        return None;
    };
    Some(WhisperEngine::load_with(&path, options).expect("the test model should load"))
}

/// Reads a 16 kHz mono 16-bit WAV as `f32` samples.
fn read_wav(path: &str) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("the speech fixture should open");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, SAMPLE_RATE);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);
    assert_eq!(spec.sample_format, hound::SampleFormat::Int);
    reader
        .samples::<i16>()
        .map(|sample| f32::from(sample.expect("a readable sample")) / 32_768.0)
        .collect()
}

fn joined_lowercase(spans: &[TextSpan]) -> String {
    spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

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
