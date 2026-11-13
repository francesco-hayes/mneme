//! Runs the `mneme-bench` and `mneme-models` binaries.
//!
//! The tests that transcribe need a model file; see `tests/common/mod.rs`
//! for where they look for it. Without one each prints a `SKIPPED` line and
//! passes. Everything else runs anywhere, and nothing here uses the network.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

use common::{SPEECH_WAV, model_path, read_wav};

const BENCH: &str = env!("CARGO_BIN_EXE_mneme-bench");
const MODELS: &str = env!("CARGO_BIN_EXE_mneme-models");

/// Words the tiny English model must find in the fixture.
const EXPECTED_PHRASES: [&str; 4] = ["hello", "meeting", "quick brown fox", "lazy dog"];

fn run(program: &str, arguments: &[&str]) -> Output {
    Command::new(program)
        .args(arguments)
        .output()
        .expect("the binary should start")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The test model, or `None` after printing why the test is skipped.
fn model(test: &str) -> Option<String> {
    let path = model_path();
    if path.is_none() {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED {test}: no Whisper model file. Run \
             crates/mneme-transcribe/scripts/fetch-test-model.sh, or set MNEME_TEST_MODEL \
             to a ggml-tiny.en.bin file."
        );
    }
    path.map(|path| path.to_str().expect("a UTF-8 path").to_owned())
}

/// A file that exists but is no model.
fn junk_model(dir: &Path) -> PathBuf {
    let path = dir.join("ggml-junk.bin");
    std::fs::write(&path, b"this is not a whisper model").unwrap();
    path
}

/// The speech fixture as a 48 kHz, stereo, 24-bit file: every sample three
/// times with straight lines between them, the same on both channels.
fn fixture_at_48_khz_stereo(dir: &Path) -> PathBuf {
    let samples = read_wav(SPEECH_WAV);
    let path = dir.join("speech-48k-stereo-24bit.wav");
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for (index, sample) in samples.iter().enumerate() {
        let next = samples.get(index + 1).copied().unwrap_or(*sample);
        for step in 0..3 {
            let value = sample + (next - sample) * step as f32 / 3.0;
            let scaled = (value * 8_388_607.0) as i32;
            writer.write_sample(scaled).unwrap();
            writer.write_sample(scaled).unwrap();
        }
    }
    writer.finalize().unwrap();
    path
}

#[test]
fn help_explains_the_tool_and_exits_cleanly() {
    for flag in ["--help", "-h"] {
        let output = run(BENCH, &[flag]);
        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        for expected in [
            "USAGE",
            "--model <path>",
            "--model-id <id>",
            "--models-dir <dir>",
            "--audio <wav>",
            "--language <code>",
            "--runs <n>",
            "--fixed-windows",
            "--json",
            "--release",
            "Real-time factor = processing time divided by audio time",
        ] {
            assert!(
                text.contains(expected),
                "{expected:?} is missing from the help"
            );
        }
        assert!(stderr(&output).is_empty());
    }
}

#[test]
fn version_prints_the_crate_version() {
    let output = run(BENCH, &["--version"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim(),
        format!("mneme-bench {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn bad_arguments_exit_with_status_2_and_point_to_the_help() {
    for arguments in [
        &[][..],
        &["--audio", "a.wav"],
        &["--model", "m.bin"],
        &["--model", "m.bin", "--audio", "a.wav", "--runs", "0"],
        &["--model", "m.bin", "--audio", "a.wav", "--no-such-flag"],
    ] {
        let output = run(BENCH, arguments);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(stdout(&output).is_empty(), "{arguments:?}");
        let message = stderr(&output);
        assert!(message.starts_with("mneme-bench: "), "{message}");
        assert!(message.contains("--help"), "{message}");
    }
}

#[test]
fn a_missing_model_or_recording_is_an_error_with_status_1() {
    let dir = TempDir::new().unwrap();

    let output = run(
        BENCH,
        &[
            "--model",
            "/nonexistent/ggml-none.bin",
            "--audio",
            SPEECH_WAV,
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("model file not found: /nonexistent/ggml-none.bin"));

    let models_dir = dir.path().join("models");
    let output = run(
        BENCH,
        &[
            "--model-id",
            "medium",
            "--models-dir",
            models_dir.to_str().unwrap(),
            "--audio",
            SPEECH_WAV,
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("Medium (`medium`) is not in"), "{message}");
    assert!(
        message.contains("mneme-models -- fetch medium"),
        "{message}"
    );
    assert!(
        !models_dir.exists(),
        "the models folder must not be created"
    );

    let junk = junk_model(dir.path());
    let output = run(
        BENCH,
        &[
            "--model",
            junk.to_str().unwrap(),
            "--audio",
            "/nonexistent/recording.wav",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot open /nonexistent/recording.wav"));
    assert!(stdout(&output).is_empty());
}

#[test]
fn a_file_that_is_no_model_is_reported_and_nothing_is_printed_as_a_result() {
    let dir = TempDir::new().unwrap();
    let junk = junk_model(dir.path());
    let output = run(
        BENCH,
        &["--model", junk.to_str().unwrap(), "--audio", SPEECH_WAV],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    let message = stderr(&output);
    assert!(message.contains("could not load model"), "{message}");
    assert!(message.contains("ggml-junk.bin"), "{message}");
}

#[test]
fn the_text_report_on_the_fixture_has_every_field() {
    let Some(model) = model("the_text_report_on_the_fixture_has_every_field") else {
        return;
    };
    let output = run(
        BENCH,
        &["--model", &model, "--audio", SPEECH_WAV, "--runs", "2"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);

    for expected in [
        "whisper.cpp ",
        "Machine     ",
        "Model       ",
        "ggml-tiny.en.bin",
        "Audio       ",
        "5.75 s; 16000 Hz, 1 channel, 16-bit int; used as it is",
        "Language    auto",
        "MODEL LOAD",
        "First load in this process",
        "Second load right after",
        "CHUNKS (silence chunker",
        "Chunks sent to the model       1",
        "Dropped for too little speech  0",
        "Cuts forced by the maximum     0",
        "Audio sent to the model",
        "Chunk length",
        "Chunks that gave no text       0",
        "TRANSCRIPTION (2 runs, model loaded once)",
        "Run 1",
        "Run 2",
        "Wall time",
        "Real-time factor               min 0.",
        "= processing time divided by audio time; lower is faster",
        "Time per chunk",
        "LIVE SIMULATION",
        "Furthest behind",
        "Most audio waiting             min 0.000 s · median 0.000 s · max 0.000 s",
        "Wait at stop",
        "Chunks that had to wait        0 of 1",
        "TRANSCRIPT (run 1",
        "[00:00:00] Hello",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
    // The transcript lines, without their timestamps, joined.
    let transcript = text
        .lines()
        .filter_map(|line| line.strip_prefix('[')?.split_once("] "))
        .map(|(_, words)| words)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for phrase in EXPECTED_PHRASES {
        assert!(
            transcript.contains(phrase),
            "{phrase:?} is missing from:\n{text}"
        );
    }
    // whisper.cpp's own log stays out unless --verbose asks for it.
    assert!(!stderr(&output).contains("whisper_"), "{}", stderr(&output));
}

#[test]
fn the_json_report_on_a_48_khz_stereo_file_is_complete_and_converted() {
    let Some(model) = model("the_json_report_on_a_48_khz_stereo_file_is_complete_and_converted")
    else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let wav = fixture_at_48_khz_stereo(dir.path());
    let output = run(
        BENCH,
        &[
            "--model",
            &model,
            "--audio",
            wav.to_str().unwrap(),
            "--language",
            "en",
            "--runs",
            "2",
            "--what-if-slower",
            "100",
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    // The whole of stdout is one JSON document.
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("stdout is JSON");

    assert_eq!(json["tool"]["name"], "mneme-bench");
    assert!(json["machine"]["chip"].is_string());
    assert!(json["load_average_before"][0].is_number());
    assert_eq!(json["model"]["file_name"], "ggml-tiny.en.bin");
    assert_eq!(json["model"]["english_only"], true);
    assert_eq!(json["language"], "en");

    // Converted: 48 kHz stereo 24-bit in, 5.75 s of 16 kHz mono out.
    let audio = &json["audio"];
    assert_eq!(audio["source"]["sample_rate"], 48_000);
    assert_eq!(audio["source"]["channels"], 2);
    assert_eq!(audio["source"]["bits_per_sample"], 24);
    assert_eq!(audio["resampled"], true);
    assert_eq!(audio["downmixed"], true);
    let duration = audio["duration_s"].as_f64().unwrap();
    assert!((5.7..5.8).contains(&duration), "{duration}");

    assert!(json["load"]["first_s"].as_f64().unwrap() > 0.0);
    assert!(json["load"]["second_s"].as_f64().unwrap() > 0.0);

    let cutting = &json["cutting"];
    assert_eq!(cutting["method"], "silence chunker");
    assert_eq!(cutting["chunks"], 1);
    assert_eq!(cutting["dropped"], 0);
    assert_eq!(cutting["forced_cuts"], 0);
    assert_eq!(cutting["chunker_config"]["pause_ms"], 700);

    let runs = json["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    for run in runs {
        let wall = run["wall_s"].as_f64().unwrap();
        let factor = run["real_time_factor"].as_f64().unwrap();
        assert!(wall > 0.0);
        assert!((factor - wall / duration).abs() < 1e-9);
        assert_eq!(run["chunks_without_text"], 0);
    }
    let summary = &json["summary"];
    assert_eq!(summary["runs"], 2);
    let median = summary["real_time_factor"]["median"].as_f64().unwrap();
    assert!(median > 0.0 && median < 1.0, "tiny model, factor {median}");
    assert!(summary["per_chunk_s"]["max"].as_f64().unwrap() > 0.0);
    // One chunk, flushed at the end: stop waits exactly as long as it takes.
    assert_eq!(summary["stop_wait_s"]["max"], summary["worst_lag_s"]["max"]);
    assert_eq!(summary["worst_queued_audio_s"]["max"], 0.0);

    let what_if = &json["what_if"];
    assert_eq!(what_if["slower_by"], 100.0);
    let projected = what_if["stop_wait_s"]["max"].as_f64().unwrap();
    let measured = summary["stop_wait_s"]["max"].as_f64().unwrap();
    assert!((projected - measured * 100.0).abs() < 1e-6);

    let chunks = json["chunks"].as_array().unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0]["processing_s"].as_array().unwrap().len(), 2);
    assert!(chunks[0]["text_chars"].as_u64().unwrap() > 50);

    // The converted audio still says what the fixture says.
    let transcript = json["transcript"].as_array().unwrap();
    assert!(!transcript.is_empty());
    assert!(
        transcript[0]["line"]
            .as_str()
            .unwrap()
            .starts_with("[00:00:00] ")
    );
    let text = transcript
        .iter()
        .map(|line| line["text"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    for phrase in EXPECTED_PHRASES {
        assert!(text.contains(phrase), "{phrase:?} is missing from {text:?}");
    }
    assert!(
        json["definitions"]["real_time_factor"]
            .as_str()
            .unwrap()
            .contains("processing time divided by audio time")
    );
}

#[test]
fn fixed_windows_skip_the_chunker() {
    let Some(model) = model("fixed_windows_skip_the_chunker") else {
        return;
    };
    let output = run(
        BENCH,
        &[
            "--model",
            &model,
            "--audio",
            SPEECH_WAV,
            "--runs",
            "1",
            "--fixed-windows",
            "--json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let cutting = &json["cutting"];
    assert_eq!(cutting["method"], "fixed windows");
    assert_eq!(cutting["fixed_window_s"], 28.0);
    assert_eq!(cutting["chunks"], 1);
    assert!(cutting["dropped"].is_null());
    assert!(cutting["chunker_config"].is_null());
    assert_eq!(cutting["audio_sent_share"], 1.0);
    assert_eq!(json["chunks"][0]["start_s"], 0.0);
}

#[test]
fn an_english_only_model_refuses_another_language_before_transcribing() {
    let Some(model) = model("an_english_only_model_refuses_another_language_before_transcribing")
    else {
        return;
    };
    let output = run(
        BENCH,
        &["--model", &model, "--audio", SPEECH_WAV, "--language", "de"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("only transcribes English"));
}

#[test]
fn verbose_lets_whisper_cpp_log_to_stderr_and_keeps_stdout_clean() {
    let Some(model) = model("verbose_lets_whisper_cpp_log_to_stderr_and_keeps_stdout_clean") else {
        return;
    };
    let output = run(
        BENCH,
        &[
            "--model",
            &model,
            "--audio",
            SPEECH_WAV,
            "--runs",
            "1",
            "--json",
            "--verbose",
        ],
    );
    assert!(output.status.success());
    assert!(stderr(&output).contains("whisper_"), "{}", stderr(&output));
    serde_json::from_str::<serde_json::Value>(&stdout(&output)).expect("stdout is still JSON");
}

#[test]
fn mneme_models_lists_the_catalog_without_touching_the_folder() {
    let dir = TempDir::new().unwrap();
    let models_dir = dir.path().join("models");
    std::fs::create_dir(&models_dir).unwrap();
    let part = models_dir.join("ggml-small.bin.part");
    std::fs::write(&part, b"half a download").unwrap();
    // A sparse file of the right size counts as installed.
    let tiny = mneme_transcribe::find_model("tiny").unwrap();
    std::fs::File::create(models_dir.join(&tiny.file_name))
        .unwrap()
        .set_len(tiny.size_bytes)
        .unwrap();

    let output = run(
        MODELS,
        &["list", "--models-dir", models_dir.to_str().unwrap()],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for spec in mneme_transcribe::catalog() {
        let line = text
            .lines()
            .find(|line| line.split_whitespace().next() == Some(spec.id.as_str()))
            .unwrap_or_else(|| panic!("{} is not listed in:\n{text}", spec.id));
        assert!(line.contains(&spec.size_bytes.to_string()), "{line}");
        let expected = if spec.id == "tiny" { "yes" } else { "no" };
        assert_eq!(line.split_whitespace().nth(2), Some(expected), "{line}");
    }
    assert!(part.exists(), "listing must not delete a .part file");
}

#[test]
fn mneme_models_fetch_does_nothing_for_an_installed_model_and_refuses_unknown_ids() {
    let dir = TempDir::new().unwrap();
    let models_dir = dir.path().to_str().unwrap();
    let base = mneme_transcribe::find_model("base").unwrap();
    std::fs::File::create(dir.path().join(&base.file_name))
        .unwrap()
        .set_len(base.size_bytes)
        .unwrap();

    // Installed already: no download is started.
    let output = run(MODELS, &["fetch", "base", "--models-dir", models_dir]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("Already installed: "));

    // An unknown id fails before any connection is made.
    let output = run(MODELS, &["fetch", "gigantic", "--models-dir", models_dir]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown model `gigantic`"));

    let output = run(MODELS, &["fetch"]);
    assert_eq!(output.status.code(), Some(2));
    let output = run(MODELS, &["--help"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("mneme-models fetch <id>"));
}
