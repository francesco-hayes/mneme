//! Helpers shared by the integration tests: the speech fixture and the
//! optional Whisper test model.
//!
//! Tests that need a model look for
//!
//! 1. the path in the `MNEME_TEST_MODEL` environment variable, else
//! 2. `crates/mneme-transcribe/models/ggml-tiny.en.bin`, which
//!    `crates/mneme-transcribe/scripts/fetch-test-model.sh` downloads.
//!
//! When neither is there, each such test prints a `SKIPPED` line and passes.
//!
//! The fixture `tests/data/speech.wav` is 5.75 s of synthetic speech (see the
//! `.gitignore` beside it for how it was made). It says: "Hello, this is a
//! test of the meeting transcription. The quick brown fox jumps over the lazy
//! dog."

// Each test binary uses its own part of this module.
#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;

use mneme_core::SAMPLE_RATE;
use mneme_transcribe::{TextSpan, WhisperEngine, WhisperOptions};

const DEFAULT_MODEL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/models/ggml-tiny.en.bin");
pub const SPEECH_WAV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/speech.wav");

pub fn model_path() -> Option<PathBuf> {
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
pub fn engine(test: &str) -> Option<WhisperEngine> {
    engine_with(test, &WhisperOptions::default())
}

pub fn engine_with(test: &str, options: &WhisperOptions) -> Option<WhisperEngine> {
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
pub fn read_wav(path: &str) -> Vec<f32> {
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

/// The spans' text as one lowercase string.
pub fn joined_lowercase(spans: &[TextSpan]) -> String {
    spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
