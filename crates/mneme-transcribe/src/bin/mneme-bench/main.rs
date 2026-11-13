//! `mneme-bench`: times a Whisper model on a recording.
//!
//! Reads a WAV file, converts it to 16 kHz mono, cuts it with the silence
//! chunker the way a live session does, transcribes every chunk and reports
//! load time, transcription time, the real-time factor, what the chunker did
//! and what the times would have meant in a live meeting. `--help` has the
//! details. Benchmark numbers come from release builds.

mod args;
mod audio;
mod cut;
mod machine;
mod report;
mod resample;
mod simulate;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use mneme_core::{Speaker, format_timestamp};
use mneme_transcribe::{
    ChunkerConfig, Language, ModelError, ModelSpec, TextSpan, Transcriber, WhisperEngine, catalog,
    installed_model_path,
};

use crate::args::{Args, Command, ModelChoice};
use crate::cut::{FIXED_WINDOW_MS, Piece, cut_fixed_windows, cut_with_chunker};
use crate::machine::{Machine, load_average};
use crate::report::{
    AudioReport, ChunkReport, ChunkerSettings, CuttingReport, Definitions, LoadReport, ModelReport,
    Report, RunReport, Spread, Summary, Tool, TranscriptLine, WhatIf,
};
use crate::simulate::{Job, Simulation, simulate};

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            print!("{}", args::HELP);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("mneme-bench {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Run(args)) => match run(&args) {
            Ok(report) => {
                if args.json {
                    println!("{}", report.to_json());
                } else {
                    print!("{}", report.to_text());
                }
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("mneme-bench: {message}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            eprintln!("mneme-bench: {message}");
            eprintln!("Try `mneme-bench --help`.");
            ExitCode::from(2)
        }
    }
}

/// One measured pass over all chunks.
struct Run {
    /// Time the engine took per chunk, in seconds.
    chunk_s: Vec<f64>,
    /// What each chunk gave.
    spans: Vec<Vec<TextSpan>>,
}

fn run(args: &Args) -> Result<Report, String> {
    let (model_path, spec) = resolve_model(&args.model, &args.models_dir)?;
    let model_size = std::fs::metadata(&model_path)
        .map_err(|error| format!("cannot read {}: {error}", model_path.display()))?
        .len();

    let started = Instant::now();
    let audio = audio::read_wav_file(&args.audio)?;
    let read_and_convert_s = started.elapsed().as_secs_f64();
    let audio_s = audio.samples.len() as f64 / f64::from(mneme_core::SAMPLE_RATE);

    if !args.verbose {
        WhisperEngine::silence_log();
    }
    let started_at = machine::local_time();
    let load_average_before = load_average();

    // Load twice: the first load of a process pays for reading the file and
    // setting up the GPU; the second shows what is left once that is warm.
    let started = Instant::now();
    let engine = WhisperEngine::load(&model_path).map_err(|error| error.to_string())?;
    let first_load_s = started.elapsed().as_secs_f64();
    drop(engine);
    let started = Instant::now();
    let engine = WhisperEngine::load(&model_path).map_err(|error| error.to_string())?;
    let second_load_s = started.elapsed().as_secs_f64();
    engine
        .check_language(&args.language)
        .map_err(|error| error.to_string())?;

    let started = Instant::now();
    let (pieces, stats, config) = if args.fixed_windows {
        (
            cut_fixed_windows(&audio.samples, FIXED_WINDOW_MS),
            None,
            None,
        )
    } else {
        let config = ChunkerConfig::default();
        let (pieces, stats) = cut_with_chunker(&audio.samples, config.clone());
        (pieces, Some(stats), Some(config))
    };
    let cutting_s = started.elapsed().as_secs_f64();

    let mut runs = Vec::with_capacity(args.runs);
    for _ in 0..args.runs {
        runs.push(transcribe_all(&engine, &pieces, &args.language)?);
    }
    let english_only = !engine.is_multilingual();
    // The engine must be gone before the process exits (see `WhisperEngine`).
    drop(engine);
    let load_average_after = load_average();

    let stop_at_ms = audio.duration_ms();
    let simulate_run = |run: &Run, slower_by: f64| -> Simulation {
        let jobs: Vec<Job> = pieces
            .iter()
            .zip(&run.chunk_s)
            .map(|(piece, seconds)| Job {
                available_at_ms: piece.available_at_ms,
                audio_ms: piece.duration_ms(),
                processing_ms: seconds * 1000.0 * slower_by,
            })
            .collect();
        simulate(&jobs, stop_at_ms)
    };

    // Exact lengths from the sample counts; `duration_ms` rounds up.
    let chunk_lengths: Vec<f64> = pieces
        .iter()
        .map(|piece| piece.samples.len() as f64 / f64::from(mneme_core::SAMPLE_RATE))
        .collect();
    let audio_sent_s: f64 = chunk_lengths.iter().sum();
    let run_reports: Vec<RunReport> = runs
        .iter()
        .map(|run| {
            let wall_s: f64 = run.chunk_s.iter().sum();
            RunReport {
                wall_s,
                real_time_factor: wall_s / audio_s,
                per_chunk_s: Spread::of(&run.chunk_s),
                chunks_without_text: run.spans.iter().filter(|spans| spans.is_empty()).count(),
                simulation: simulate_run(run, 1.0),
            }
        })
        .collect();

    let spread_over_runs = |pick: &dyn Fn(&RunReport) -> f64| -> Spread {
        Spread::of(&run_reports.iter().map(pick).collect::<Vec<_>>())
            .expect("there is at least one run")
    };
    let all_chunk_times: Vec<f64> = runs.iter().flat_map(|run| run.chunk_s.clone()).collect();
    let summary = Summary {
        runs: runs.len(),
        wall_s: spread_over_runs(&|run| run.wall_s),
        real_time_factor: spread_over_runs(&|run| run.real_time_factor),
        processing_per_audio_sent: (audio_sent_s > 0.0)
            .then(|| spread_over_runs(&|run| run.wall_s / audio_sent_s)),
        per_chunk_s: Spread::of(&all_chunk_times),
        chunks_without_text: run_reports
            .iter()
            .map(|run| run.chunks_without_text)
            .max()
            .unwrap_or(0),
        worst_lag_s: spread_over_runs(&|run| run.simulation.worst_lag_s),
        worst_queued_audio_s: spread_over_runs(&|run| run.simulation.worst_queued_audio_s),
        stop_wait_s: spread_over_runs(&|run| run.simulation.stop_wait_s),
        chunks_that_waited: run_reports
            .iter()
            .map(|run| run.simulation.chunks_that_waited)
            .max()
            .unwrap_or(0),
    };
    let what_if = args.what_if_slower.map(|slower_by| {
        let simulations: Vec<Simulation> = runs
            .iter()
            .map(|run| simulate_run(run, slower_by))
            .collect();
        WhatIf::new(slower_by, &summary, &simulations)
    });

    let first = &runs[0];
    let transcript = transcript_lines(&pieces, &first.spans);
    let transcript_same_in_every_run = runs.iter().all(|run| run.spans == first.spans);
    let chunks = pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| ChunkReport {
            start_s: piece.start_ms as f64 / 1000.0,
            length_s: chunk_lengths[index],
            available_at_s: piece.available_at_ms as f64 / 1000.0,
            processing_s: runs.iter().map(|run| run.chunk_s[index]).collect(),
            text_chars: first.spans[index]
                .iter()
                .map(|span| span.text.chars().count())
                .sum(),
        })
        .collect();
    Ok(Report {
        tool: Tool {
            name: "mneme-bench",
            version: env!("CARGO_PKG_VERSION"),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            whisper_cpp_version: WhisperEngine::whisper_cpp_version(),
            started_at,
        },
        machine: Machine::read(),
        load_average_before,
        load_average_after,
        model: ModelReport {
            path: model_path.display().to_string(),
            file_name: model_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size_bytes: model_size,
            catalog_id: spec.map(|spec| spec.id.clone()),
            name: spec.map(|spec| spec.name.clone()),
            english_only,
        },
        audio: AudioReport {
            path: args.audio.display().to_string(),
            duration_s: audio_s,
            source: audio.source.clone(),
            downmixed: audio.downmixed,
            resampled: audio.resampled,
            read_and_convert_s,
        },
        language: args.language.to_string(),
        load: LoadReport {
            first_s: first_load_s,
            second_s: second_load_s,
        },
        cutting: CuttingReport {
            method: if args.fixed_windows {
                "fixed windows"
            } else {
                "silence chunker"
            },
            chunks: pieces.len(),
            dropped: stats.map(|stats| stats.dropped),
            forced_cuts: stats.map(|stats| stats.forced_cuts),
            gaps: stats.map(|stats| stats.gaps),
            chunker_config: config.as_ref().map(chunker_settings),
            fixed_window_s: args
                .fixed_windows
                .then_some(FIXED_WINDOW_MS as f64 / 1000.0),
            audio_sent_s,
            audio_sent_share: audio_sent_s / audio_s,
            chunk_length_s: Spread::of(&chunk_lengths),
            cutting_s,
        },
        runs: run_reports,
        summary,
        what_if,
        chunks,
        transcript,
        transcript_same_in_every_run,
        definitions: Definitions::default(),
    })
}

/// Transcribes every piece in order, as the live worker does, and times each
/// call.
fn transcribe_all(
    engine: &WhisperEngine,
    pieces: &[Piece],
    language: &Language,
) -> Result<Run, String> {
    let mut chunk_s = Vec::with_capacity(pieces.len());
    let mut spans = Vec::with_capacity(pieces.len());
    for piece in pieces {
        let started = Instant::now();
        let result = engine
            .transcribe(&piece.samples, language)
            .map_err(|error| error.to_string())?;
        chunk_s.push(started.elapsed().as_secs_f64());
        spans.push(result);
    }
    Ok(Run { chunk_s, spans })
}

/// The transcript as the session would write it: each span placed on the
/// meeting clock by its chunk's start, one `[hh:mm:ss] text` line per span.
fn transcript_lines(pieces: &[Piece], spans: &[Vec<TextSpan>]) -> Vec<TranscriptLine> {
    pieces
        .iter()
        .zip(spans)
        .flat_map(|(piece, spans)| {
            spans.iter().cloned().map(|span| {
                let segment = span.into_segment(Speaker::Me, piece.start_ms);
                TranscriptLine {
                    line: format!("[{}] {}", format_timestamp(segment.start_ms), segment.text),
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                    text: segment.text,
                }
            })
        })
        .collect()
}

fn chunker_settings(config: &ChunkerConfig) -> ChunkerSettings {
    ChunkerSettings {
        pause_ms: config.pause_ms,
        max_chunk_ms: config.max_chunk_ms,
        cut_search_ms: config.cut_search_ms,
        padding_ms: config.padding_ms,
        min_speech_ms: config.min_speech_ms,
        speech_ratio: config.speech_ratio,
        min_speech_level: config.min_speech_level,
        noise_doubling_ms: config.noise_doubling_ms,
        max_gap_ms: config.max_gap_ms,
    }
}

/// Finds the model file, and the catalog entry it is, if any.
///
/// A model given by id is looked up in `models_dir` without opening a model
/// store there: opening one deletes unfinished downloads in the folder.
fn resolve_model(
    choice: &ModelChoice,
    models_dir: &Path,
) -> Result<(PathBuf, Option<&'static ModelSpec>), String> {
    match choice {
        ModelChoice::Path(path) => {
            let metadata = std::fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .ok_or_else(|| format!("model file not found: {}", path.display()))?;
            let file_name = path.file_name().map(|name| name.to_string_lossy());
            let spec = catalog().iter().find(|spec| {
                Some(spec.file_name.as_str()) == file_name.as_deref()
                    && spec.size_bytes == metadata.len()
            });
            Ok((path.clone(), spec))
        }
        ModelChoice::Id(id) => match installed_model_path(models_dir, id) {
            Ok(path) => Ok((path, mneme_transcribe::find_model(id))),
            Err(ModelError::UnknownModel { .. }) => Err(format!(
                "unknown model `{id}`. The catalog has: {}",
                catalog()
                    .iter()
                    .map(|spec| spec.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Err(ModelError::NotInstalled { name, .. }) => Err(format!(
                "{name} (`{id}`) is not in {}. Download it there with:\n  \
                 cargo run -p mneme-transcribe --release --bin mneme-models -- fetch {id}\n\
                 or point --models-dir at the folder that has it, or pass the file with --model.",
                models_dir.display()
            )),
            Err(other) => Err(other.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(start_ms: u64) -> Piece {
        Piece {
            start_ms,
            available_at_ms: start_ms + 1_000,
            samples: vec![0.0; 16_000],
        }
    }

    fn span(start_ms: u64, end_ms: u64, text: &str) -> TextSpan {
        TextSpan {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn transcript_lines_sit_on_the_meeting_clock() {
        let pieces = [piece(800), piece(3_723_400), piece(4_000_000)];
        let spans = [
            vec![span(0, 900, "Good morning.")],
            vec![
                span(0, 2_000, "Let's start."),
                span(2_700, 5_000, "First point."),
            ],
            // A chunk that gave no text adds no line.
            vec![],
        ];
        let lines = transcript_lines(&pieces, &spans);
        let text: Vec<&str> = lines.iter().map(|line| line.line.as_str()).collect();
        assert_eq!(
            text,
            vec![
                "[00:00:00] Good morning.",
                "[01:02:03] Let's start.",
                "[01:02:06] First point.",
            ]
        );
        assert_eq!(lines[2].start_ms, 3_726_100);
        assert_eq!(lines[2].end_ms, 3_728_400);
        assert_eq!(lines[2].text, "First point.");
    }

    #[test]
    fn a_model_path_that_is_not_a_file_is_refused() {
        let choice = ModelChoice::Path(PathBuf::from("/nonexistent/mneme/ggml-base.bin"));
        let error = resolve_model(&choice, Path::new("/unused")).unwrap_err();
        assert!(error.contains("model file not found"), "{error}");
        assert!(
            error.contains("/nonexistent/mneme/ggml-base.bin"),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_model_id_lists_the_catalog() {
        let choice = ModelChoice::Id("huge".to_owned());
        let error = resolve_model(&choice, Path::new("/unused")).unwrap_err();
        assert!(error.contains("unknown model `huge`"), "{error}");
        assert!(error.contains("tiny.en"), "{error}");
        assert!(error.contains("large-v3-turbo"), "{error}");
    }

    #[test]
    fn a_model_that_is_not_installed_says_how_to_get_it_and_touches_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let missing = folder.path().join("not-created");
        let choice = ModelChoice::Id("base.en".to_owned());
        let error = resolve_model(&choice, &missing).unwrap_err();
        assert!(error.contains("Base (English)"), "{error}");
        assert!(error.contains("mneme-models -- fetch base.en"), "{error}");
        assert!(!missing.exists(), "looking must not create the folder");

        // An unfinished download in the folder is left alone.
        let part = folder.path().join("ggml-base.en.bin.part");
        std::fs::write(&part, b"partial").unwrap();
        assert!(resolve_model(&choice, folder.path()).is_err());
        assert!(part.exists(), "looking must not delete a .part file");
    }

    #[test]
    fn a_model_file_outside_the_catalog_still_resolves() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ggml-custom.bin");
        std::fs::write(&path, b"not really a model").unwrap();
        let (resolved, spec) =
            resolve_model(&ModelChoice::Path(path.clone()), Path::new("/unused")).unwrap();
        assert_eq!(resolved, path);
        assert!(spec.is_none());
    }
}
