//! Command-line arguments of `mneme-bench`.

use std::path::PathBuf;

use mneme_transcribe::Language;

/// The models folder `--model-id` looks in when `--models-dir` is not given:
/// this crate's own `models/` folder, where the development scripts put
/// model files. Never the app's data folder.
pub const DEFAULT_MODELS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/models");

pub const DEFAULT_RUNS: usize = 3;

pub const HELP: &str = "\
mneme-bench: how fast does a Whisper model transcribe a recording on this Mac?

Reads a WAV file, cuts it into chunks at pauses the way a live recording is
cut, transcribes every chunk and reports the times.

Benchmark numbers come from release builds:

  cargo run -p mneme-transcribe --release --bin mneme-bench -- \\
      --model-id tiny.en --audio recording.wav

USAGE
  mneme-bench (--model <path> | --model-id <id>) --audio <wav> [options]

MODEL
  --model <path>        A whisper.cpp model file (ggml-*.bin).
  --model-id <id>       A model of the catalog, such as tiny.en, base or
                        large-v3-turbo, looked up in the models folder.
  --models-dir <dir>    The models folder for --model-id. Only read, never
                        changed. Default: this crate's models/ folder
                        (the development folder, not the app's).

AUDIO
  --audio <wav>         The recording. Any sample rate and channel count,
                        8, 16, 24 or 32-bit integer or 32-bit float samples.
                        It is mixed down to mono and converted to 16 kHz.
  --language <code>     auto, or a language code such as en or de.
                        Default: auto. English-only models take auto and en.

MEASUREMENT
  --runs <n>            Transcribe the recording n times with the model
                        loaded once, and report the spread. Default: 3.
  --fixed-windows       Baseline without the silence chunker: cut the
                        recording into windows of 28 s, silence included.
  --what-if-slower <x>  Also show how a live session would cope if every
                        chunk took x times as long as measured. A projection,
                        not a measurement.

OUTPUT
  --json                Print the report as JSON instead of text.
  --verbose             Let whisper.cpp write its own log to stderr.
  -h, --help            Show this help.
  -V, --version         Show the version.

WHAT IS REPORTED
  Audio length, model load time (the first load in this process and a second
  one right after it), transcription time, the real-time factor, the chunks
  and what the chunker did, the time per chunk, what the times would have
  meant in a live meeting, and the transcript.

  Real-time factor = processing time divided by audio time. Lower is faster;
  0.1 means one minute of audio takes six seconds; above 1 the model is
  slower than the meeting.

  Live simulation: a recording session has one worker that transcribes
  chunks in order. The report plays the measured times through such a worker
  and says how far it would have fallen behind and how long stopping the
  recording would have waited for the transcript.
";

/// Where the model comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChoice {
    Path(PathBuf),
    Id(String),
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub model: ModelChoice,
    pub models_dir: PathBuf,
    pub audio: PathBuf,
    pub language: Language,
    pub runs: usize,
    pub fixed_windows: bool,
    pub what_if_slower: Option<f64>,
    pub json: bool,
    pub verbose: bool,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    Version,
    Run(Box<Args>),
}

/// Parses the arguments after the program name. Options take their value as
/// the next argument or after `=`.
pub fn parse<I, S>(arguments: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut model_path: Option<PathBuf> = None;
    let mut model_id: Option<String> = None;
    let mut models_dir: Option<PathBuf> = None;
    let mut audio: Option<PathBuf> = None;
    let mut language = Language::auto();
    let mut runs = DEFAULT_RUNS;
    let mut fixed_windows = false;
    let mut what_if_slower = None;
    let mut json = false;
    let mut verbose = false;

    let mut arguments = arguments.into_iter().map(Into::into);
    while let Some(argument) = arguments.next() {
        let (name, attached) = match argument.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (argument.clone(), None),
        };
        // The value of an option that takes one.
        let mut value = |attached: Option<String>| -> Result<String, String> {
            attached
                .or_else(|| arguments.next())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let no_value = |attached: &Option<String>| -> Result<(), String> {
            match attached {
                Some(_) => Err(format!("{name} takes no value")),
                None => Ok(()),
            }
        };

        match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--model" => model_path = Some(PathBuf::from(value(attached)?)),
            "--model-id" => model_id = Some(value(attached)?),
            "--models-dir" => models_dir = Some(PathBuf::from(value(attached)?)),
            "--audio" => audio = Some(PathBuf::from(value(attached)?)),
            "--language" => {
                let text = value(attached)?;
                language = text
                    .parse()
                    .map_err(|error| format!("--language: {error}"))?;
            }
            "--runs" => {
                let text = value(attached)?;
                runs = match text.parse::<usize>() {
                    Ok(count) if (1..=1000).contains(&count) => count,
                    _ => return Err(format!("--runs: `{text}` is not a number from 1 to 1000")),
                };
            }
            "--what-if-slower" => {
                let text = value(attached)?;
                what_if_slower = match text.parse::<f64>() {
                    Ok(factor) if factor.is_finite() && factor > 0.0 => Some(factor),
                    _ => {
                        return Err(format!(
                            "--what-if-slower: `{text}` is not a number above zero"
                        ));
                    }
                };
            }
            "--fixed-windows" => {
                no_value(&attached)?;
                fixed_windows = true;
            }
            "--json" => {
                no_value(&attached)?;
                json = true;
            }
            "--verbose" => {
                no_value(&attached)?;
                verbose = true;
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    let model = match (model_path, model_id) {
        (Some(path), None) => ModelChoice::Path(path),
        (None, Some(id)) => ModelChoice::Id(id),
        (Some(_), Some(_)) => return Err("give --model or --model-id, not both".to_owned()),
        (None, None) => {
            return Err("a model is needed: --model <path> or --model-id <id>".to_owned());
        }
    };
    if matches!(model, ModelChoice::Path(_)) && models_dir.is_some() {
        return Err("--models-dir only goes with --model-id".to_owned());
    }
    let audio = audio.ok_or("a recording is needed: --audio <wav>")?;

    Ok(Command::Run(Box::new(Args {
        model,
        models_dir: models_dir.unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR)),
        audio,
        language,
        runs,
        fixed_windows,
        what_if_slower,
        json,
        verbose,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arguments: &[&str]) -> Args {
        match parse(arguments.iter().copied()) {
            Ok(Command::Run(args)) => *args,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    fn error(arguments: &[&str]) -> String {
        parse(arguments.iter().copied()).expect_err("expected an error")
    }

    #[test]
    fn the_two_required_arguments_and_the_defaults() {
        let args = run(&["--model", "models/ggml-base.bin", "--audio", "call.wav"]);
        assert_eq!(
            args,
            Args {
                model: ModelChoice::Path(PathBuf::from("models/ggml-base.bin")),
                models_dir: PathBuf::from(DEFAULT_MODELS_DIR),
                audio: PathBuf::from("call.wav"),
                language: Language::auto(),
                runs: 3,
                fixed_windows: false,
                what_if_slower: None,
                json: false,
                verbose: false,
            }
        );
    }

    #[test]
    fn the_default_models_folder_is_the_crate_s_own() {
        assert!(DEFAULT_MODELS_DIR.ends_with("crates/mneme-transcribe/models"));
    }

    #[test]
    fn every_option_in_both_spellings() {
        let args = run(&[
            "--model-id=small.en",
            "--models-dir",
            "/tmp/models",
            "--audio=/tmp/a b.wav",
            "--language",
            "EN",
            "--runs=5",
            "--fixed-windows",
            "--what-if-slower",
            "12.5",
            "--json",
            "--verbose",
        ]);
        assert_eq!(args.model, ModelChoice::Id("small.en".to_owned()));
        assert_eq!(args.models_dir, PathBuf::from("/tmp/models"));
        assert_eq!(args.audio, PathBuf::from("/tmp/a b.wav"));
        assert_eq!(args.language, Language::english());
        assert_eq!(args.runs, 5);
        assert!(args.fixed_windows);
        assert_eq!(args.what_if_slower, Some(12.5));
        assert!(args.json);
        assert!(args.verbose);
    }

    #[test]
    fn help_and_version_win_over_everything_else() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
        assert_eq!(parse(["--model", "m.bin", "--help"]), Ok(Command::Help));
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
    }

    #[test]
    fn the_help_text_names_every_option_and_defines_the_real_time_factor() {
        for option in [
            "--model <path>",
            "--model-id <id>",
            "--models-dir <dir>",
            "--audio <wav>",
            "--language <code>",
            "--runs <n>",
            "--fixed-windows",
            "--what-if-slower <x>",
            "--json",
            "--verbose",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(option), "{option} is missing from the help");
        }
        assert!(HELP.contains("--release"));
        assert!(HELP.contains("processing time divided by audio time"));
    }

    #[test]
    fn a_model_and_a_recording_are_required() {
        assert!(error(&["--audio", "a.wav"]).contains("a model is needed"));
        assert!(error(&["--model", "m.bin"]).contains("a recording is needed"));
        assert!(error(&[]).contains("a model is needed"));
    }

    #[test]
    fn model_path_and_model_id_exclude_each_other() {
        let message = error(&["--model", "m.bin", "--model-id", "base", "--audio", "a.wav"]);
        assert!(message.contains("not both"), "{message}");
        let message = error(&["--model", "m.bin", "--models-dir", "d", "--audio", "a.wav"]);
        assert!(
            message.contains("--models-dir only goes with --model-id"),
            "{message}"
        );
    }

    #[test]
    fn bad_values_are_named() {
        let base = ["--model", "m.bin", "--audio", "a.wav"];
        let with = |extra: &[&str]| error(&[&base[..], extra].concat());

        assert!(with(&["--runs", "0"]).contains("--runs"));
        assert!(with(&["--runs", "many"]).contains("`many`"));
        assert!(with(&["--runs"]).contains("--runs needs a value"));
        assert!(with(&["--language", "english"]).contains("--language"));
        assert!(with(&["--what-if-slower", "0"]).contains("--what-if-slower"));
        assert!(with(&["--what-if-slower", "-2"]).contains("--what-if-slower"));
        assert!(with(&["--what-if-slower", "nan"]).contains("--what-if-slower"));
        assert!(with(&["--json=yes"]).contains("--json takes no value"));
        assert!(with(&["--frobnicate"]).contains("unknown argument `--frobnicate`"));
        assert!(with(&["stray.wav"]).contains("unknown argument `stray.wav`"));
        assert!(error(&["--model=", "--audio", "a.wav"]).contains("--model needs a value"));
    }
}
