//! The benchmark's result: one structure, printed as text or as JSON.

use std::fmt::Write as _;

use serde::Serialize;

use crate::audio::SourceFormat;
use crate::machine::Machine;
use crate::simulate::Simulation;

/// What "real-time factor" means in this report. Printed with the number.
pub const REAL_TIME_FACTOR_MEANING: &str = "processing time divided by audio time; lower is \
     faster, and above 1 the model is slower than the meeting";

/// Smallest, middle and largest of a set of measurements.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Spread {
    pub min: f64,
    pub median: f64,
    pub max: f64,
}

impl Spread {
    /// `None` for no values. The median of an even number of values is the
    /// mean of the middle two.
    pub fn of(values: &[f64]) -> Option<Spread> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let middle = sorted.len() / 2;
        let median = if sorted.len() % 2 == 1 {
            sorted[middle]
        } else {
            (sorted[middle - 1] + sorted[middle]) / 2.0
        };
        Some(Spread {
            min: sorted[0],
            median,
            max: sorted[sorted.len() - 1],
        })
    }

    fn scaled(self, factor: f64) -> Spread {
        Spread {
            min: self.min * factor,
            median: self.median * factor,
            max: self.max * factor,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub tool: Tool,
    pub machine: Machine,
    /// System load averages (1, 5, 15 minutes) before the model was loaded
    /// and after the last run.
    pub load_average_before: Option<[f64; 3]>,
    pub load_average_after: Option<[f64; 3]>,
    pub model: ModelReport,
    pub audio: AudioReport,
    pub language: String,
    pub load: LoadReport,
    pub cutting: CuttingReport,
    pub runs: Vec<RunReport>,
    pub summary: Summary,
    /// Present with `--what-if-slower`. A projection, not a measurement.
    pub what_if: Option<WhatIf>,
    pub chunks: Vec<ChunkReport>,
    /// The transcript of the first run.
    pub transcript: Vec<TranscriptLine>,
    pub transcript_same_in_every_run: bool,
    pub definitions: Definitions,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub version: &'static str,
    /// `release` or `debug`. Only release builds give benchmark numbers.
    pub build_profile: &'static str,
    pub whisper_cpp_version: &'static str,
    /// Local date and time at the start, if `date` could be run.
    pub started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelReport {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    /// The catalog entry whose file name and size this file has, if any.
    pub catalog_id: Option<String>,
    pub name: Option<String>,
    pub english_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioReport {
    pub path: String,
    /// Length after conversion, in seconds.
    pub duration_s: f64,
    pub source: SourceFormat,
    pub downmixed: bool,
    pub resampled: bool,
    /// Time to read and convert the file.
    pub read_and_convert_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadReport {
    /// The first load of the model in this process.
    pub first_s: f64,
    /// A second load right after dropping the first.
    pub second_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CuttingReport {
    /// `silence chunker` or `fixed windows`.
    pub method: &'static str,
    /// Chunks handed to the model.
    pub chunks: usize,
    /// The chunker's own counts; absent for fixed windows.
    pub dropped: Option<u64>,
    pub forced_cuts: Option<u64>,
    pub gaps: Option<u64>,
    pub chunker_config: Option<ChunkerSettings>,
    pub fixed_window_s: Option<f64>,
    /// Audio handed to the model, in seconds, and as a share of the
    /// recording.
    pub audio_sent_s: f64,
    pub audio_sent_share: f64,
    pub chunk_length_s: Option<Spread>,
    /// Time the cutting itself took.
    pub cutting_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkerSettings {
    pub pause_ms: u32,
    pub max_chunk_ms: u32,
    pub cut_search_ms: u32,
    pub padding_ms: u32,
    pub min_speech_ms: u32,
    pub speech_ratio: f32,
    pub min_speech_level: f32,
    pub noise_doubling_ms: u32,
    pub max_gap_ms: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    /// Time spent transcribing all chunks, one after another.
    pub wall_s: f64,
    pub real_time_factor: f64,
    pub per_chunk_s: Option<Spread>,
    pub chunks_without_text: usize,
    pub simulation: Simulation,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub runs: usize,
    pub wall_s: Spread,
    pub real_time_factor: Spread,
    /// Processing time divided by the audio handed to the model, which
    /// leaves out the silence the chunker dropped.
    pub processing_per_audio_sent: Option<Spread>,
    /// Over every chunk of every run.
    pub per_chunk_s: Option<Spread>,
    /// Chunks that gave no text, in the run where there were most.
    pub chunks_without_text: usize,
    pub worst_lag_s: Spread,
    pub worst_queued_audio_s: Spread,
    pub stop_wait_s: Spread,
    pub chunks_that_waited: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhatIf {
    /// Every measured chunk time multiplied by this.
    pub slower_by: f64,
    pub real_time_factor: Spread,
    pub worst_lag_s: Spread,
    pub worst_queued_audio_s: Spread,
    pub stop_wait_s: Spread,
    pub chunks_that_waited: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkReport {
    pub start_s: f64,
    pub length_s: f64,
    /// Meeting time at which a live session would have queued the chunk.
    pub available_at_s: f64,
    /// Time the engine took, one value per run.
    pub processing_s: Vec<f64>,
    /// Characters of text in the first run. Zero: the chunk gave no text.
    pub text_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// The line as the text report prints it: `[hh:mm:ss] text`.
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Definitions {
    pub real_time_factor: &'static str,
    pub worst_lag_s: &'static str,
    pub worst_queued_audio_s: &'static str,
    pub stop_wait_s: &'static str,
}

impl Default for Definitions {
    fn default() -> Self {
        Definitions {
            real_time_factor: REAL_TIME_FACTOR_MEANING,
            worst_lag_s: "one worker, chunks in order: the longest any chunk went from joining \
                          the queue to having its text",
            worst_queued_audio_s: "the most audio waiting in the queue at any moment, not \
                                   counting the chunk being transcribed",
            stop_wait_s: "how long after the end of the recording the last chunk's text was \
                          ready",
        }
    }
}

impl WhatIf {
    /// The projection for chunk times `slower_by` times the measured ones.
    /// `simulations` are the simulations of the scaled runs.
    pub fn new(slower_by: f64, measured: &Summary, simulations: &[Simulation]) -> WhatIf {
        let spread = |pick: fn(&Simulation) -> f64| {
            Spread::of(&simulations.iter().map(pick).collect::<Vec<_>>()).unwrap_or(Spread {
                min: 0.0,
                median: 0.0,
                max: 0.0,
            })
        };
        WhatIf {
            slower_by,
            real_time_factor: measured.real_time_factor.scaled(slower_by),
            worst_lag_s: spread(|s| s.worst_lag_s),
            worst_queued_audio_s: spread(|s| s.worst_queued_audio_s),
            stop_wait_s: spread(|s| s.stop_wait_s),
            chunks_that_waited: simulations
                .iter()
                .map(|s| s.chunks_that_waited)
                .max()
                .unwrap_or(0),
        }
    }
}

impl Report {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("the report is plain data")
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out)
            .expect("writing to a string cannot fail");
        out
    }

    fn write_text(&self, out: &mut String) -> std::fmt::Result {
        let tool = &self.tool;
        writeln!(
            out,
            "{} {} · {} build · whisper.cpp {}",
            tool.name, tool.version, tool.build_profile, tool.whisper_cpp_version
        )?;
        if tool.build_profile != "release" {
            writeln!(
                out,
                "NOT A BENCHMARK: this is a {} build. Build with --release for numbers to keep.",
                tool.build_profile
            )?;
        }
        if let Some(started_at) = &tool.started_at {
            writeln!(out, "{started_at}")?;
        }
        writeln!(out)?;

        writeln!(out, "{:<12}{}", "Machine", machine_line(&self.machine))?;
        if let (Some(before), Some(after)) = (self.load_average_before, self.load_average_after) {
            writeln!(
                out,
                "{:<12}{:.2} before, {:.2} after (1-minute load average: threads running or \
                 waiting for a core)",
                "Load", before[0], after[0]
            )?;
        }
        writeln!(out, "{:<12}{}", "Model", model_line(&self.model))?;
        writeln!(out, "{:<12}{}", "", self.model.path)?;
        writeln!(out, "{:<12}{}", "Audio", self.audio.path)?;
        writeln!(out, "{:<12}{}", "", audio_line(&self.audio))?;
        writeln!(out, "{:<12}{}", "Language", self.language)?;
        writeln!(out)?;

        writeln!(out, "MODEL LOAD")?;
        row(
            out,
            "First load in this process",
            seconds(self.load.first_s),
        )?;
        row(out, "Second load right after", seconds(self.load.second_s))?;
        writeln!(out)?;

        let cutting = &self.cutting;
        match (&cutting.chunker_config, cutting.fixed_window_s) {
            (Some(config), _) => writeln!(
                out,
                "CHUNKS (silence chunker: a pause of {} ms ends a chunk, none longer than {} s)",
                config.pause_ms,
                f64::from(config.max_chunk_ms) / 1000.0
            )?,
            (None, Some(window_s)) => writeln!(
                out,
                "CHUNKS (fixed windows of {window_s} s, no silence chunker)"
            )?,
            (None, None) => writeln!(out, "CHUNKS")?,
        }
        row(out, "Chunks sent to the model", cutting.chunks.to_string())?;
        if let Some(dropped) = cutting.dropped {
            row(out, "Dropped for too little speech", dropped.to_string())?;
        }
        if let Some(forced_cuts) = cutting.forced_cuts {
            row(out, "Cuts forced by the maximum", forced_cuts.to_string())?;
        }
        if let Some(gaps) = cutting.gaps {
            row(out, "Breaks in the timestamps", gaps.to_string())?;
        }
        row(
            out,
            "Audio sent to the model",
            format!(
                "{} of {} ({:.1}%)",
                seconds(cutting.audio_sent_s),
                seconds(self.audio.duration_s),
                cutting.audio_sent_share * 100.0
            ),
        )?;
        if let Some(length) = cutting.chunk_length_s {
            row(out, "Chunk length", spread_seconds(length))?;
        }
        row(
            out,
            "Chunks that gave no text",
            self.summary.chunks_without_text.to_string(),
        )?;
        writeln!(out)?;

        let summary = &self.summary;
        writeln!(
            out,
            "TRANSCRIPTION ({} {}, model loaded once)",
            summary.runs,
            if summary.runs == 1 { "run" } else { "runs" }
        )?;
        for (index, run) in self.runs.iter().enumerate() {
            row(
                out,
                &format!("Run {}", index + 1),
                format!(
                    "{}   real-time factor {}",
                    seconds(run.wall_s),
                    factor(run.real_time_factor)
                ),
            )?;
        }
        row(out, "Wall time", spread_seconds(summary.wall_s))?;
        row(
            out,
            "Real-time factor",
            spread_factor(summary.real_time_factor),
        )?;
        writeln!(out, "    = {REAL_TIME_FACTOR_MEANING}")?;
        if let Some(per_audio_sent) = summary.processing_per_audio_sent {
            row(
                out,
                "Per second of audio sent",
                format!("{} s (median)", factor(per_audio_sent.median)),
            )?;
        }
        if let Some(per_chunk) = summary.per_chunk_s {
            row(out, "Time per chunk", spread_seconds(per_chunk))?;
        }
        writeln!(out)?;

        writeln!(
            out,
            "LIVE SIMULATION (one worker; each chunk queued when a live recording would have cut it)"
        )?;
        simulation_rows(
            out,
            summary.worst_lag_s,
            summary.worst_queued_audio_s,
            summary.stop_wait_s,
            summary.chunks_that_waited,
            cutting.chunks,
        )?;
        if summary.runs > 1 {
            writeln!(out, "    each as min · median · max over the runs")?;
        }
        if let Some(what_if) = &self.what_if {
            writeln!(out)?;
            writeln!(
                out,
                "WHAT IF every chunk took {} times as long (a projection, not a measurement)",
                what_if.slower_by
            )?;
            row(
                out,
                "Real-time factor",
                spread_factor(what_if.real_time_factor),
            )?;
            simulation_rows(
                out,
                what_if.worst_lag_s,
                what_if.worst_queued_audio_s,
                what_if.stop_wait_s,
                what_if.chunks_that_waited,
                cutting.chunks,
            )?;
        }
        writeln!(out)?;

        writeln!(
            out,
            "TRANSCRIPT (run 1{})",
            match (summary.runs, self.transcript_same_in_every_run) {
                (1, _) => "",
                (_, true) => "; the same text in every run",
                (_, false) => "; NOT the same text in every run",
            }
        )?;
        if self.transcript.is_empty() {
            writeln!(out, "(no text)")?;
        }
        for line in &self.transcript {
            writeln!(out, "{}", line.line)?;
        }
        Ok(())
    }
}

fn simulation_rows(
    out: &mut String,
    worst_lag_s: Spread,
    worst_queued_audio_s: Spread,
    stop_wait_s: Spread,
    chunks_that_waited: usize,
    chunks: usize,
) -> std::fmt::Result {
    row(out, "Furthest behind", spread_seconds(worst_lag_s))?;
    writeln!(
        out,
        "    = the longest any chunk went from being cut to having its text"
    )?;
    row(
        out,
        "Most audio waiting",
        spread_seconds(worst_queued_audio_s),
    )?;
    writeln!(
        out,
        "    = the most audio queued behind the chunk being transcribed"
    )?;
    row(out, "Wait at stop", spread_seconds(stop_wait_s))?;
    writeln!(
        out,
        "    = from the end of the recording to the complete transcript"
    )?;
    row(
        out,
        "Chunks that had to wait",
        format!("{chunks_that_waited} of {chunks}"),
    )
}

fn row(out: &mut String, label: &str, value: String) -> std::fmt::Result {
    writeln!(out, "  {label:<31}{value}")
}

fn machine_line(machine: &Machine) -> String {
    let mut parts = Vec::new();
    if let Some(chip) = &machine.chip {
        parts.push(chip.clone());
    }
    if let Some(cores) = machine.cores {
        parts.push(
            match (machine.performance_cores, machine.efficiency_cores) {
                (Some(performance), Some(efficiency)) => {
                    format!("{cores} cores ({performance} performance, {efficiency} efficiency)")
                }
                _ => format!("{cores} cores"),
            },
        );
    }
    if let Some(bytes) = machine.memory_bytes {
        parts.push(format!("{} GB memory", bytes / (1 << 30)));
    }
    if let Some(os) = &machine.os_version {
        parts.push(os.clone());
    }
    if parts.is_empty() {
        "unknown".to_owned()
    } else {
        parts.join(", ")
    }
}

fn model_line(model: &ModelReport) -> String {
    let mut line = match (&model.catalog_id, &model.name) {
        (Some(id), Some(name)) => format!("{id} ({name}), {}", model.file_name),
        _ => format!("{} (not a catalog file)", model.file_name),
    };
    let _ = write!(
        line,
        ", {} bytes ({:.0} MB)",
        grouped(model.size_bytes),
        model.size_bytes as f64 / 1e6
    );
    if model.english_only {
        line.push_str(", English only");
    }
    line
}

fn audio_line(audio: &AudioReport) -> String {
    let source = &audio.source;
    let conversion = match (audio.downmixed, audio.resampled) {
        (false, false) => "used as it is".to_owned(),
        (true, false) => "mixed down to mono".to_owned(),
        (false, true) => "converted to 16000 Hz".to_owned(),
        (true, true) => "mixed down to mono and converted to 16000 Hz".to_owned(),
    };
    format!(
        "{}; {} Hz, {} {}, {}-bit {}; {conversion}",
        length(audio.duration_s),
        source.sample_rate,
        source.channels,
        if source.channels == 1 {
            "channel"
        } else {
            "channels"
        },
        source.bits_per_sample,
        source.sample_format,
    )
}

/// Seconds with enough digits to compare short times.
fn seconds(value: f64) -> String {
    if value < 100.0 {
        format!("{value:.3} s")
    } else {
        format!("{value:.1} s")
    }
}

/// A recording's length: seconds, and minutes when it is long enough.
fn length(seconds_total: f64) -> String {
    if seconds_total < 60.0 {
        format!("{seconds_total:.2} s")
    } else {
        let whole = seconds_total.round() as u64;
        format!(
            "{seconds_total:.2} s ({} min {:02} s)",
            whole / 60,
            whole % 60
        )
    }
}

fn factor(value: f64) -> String {
    format!("{value:.4}")
}

fn spread_seconds(spread: Spread) -> String {
    format!(
        "min {} · median {} · max {}",
        seconds(spread.min),
        seconds(spread.median),
        seconds(spread.max)
    )
}

fn spread_factor(spread: Spread) -> String {
    format!(
        "min {} · median {} · max {}",
        factor(spread.min),
        factor(spread.median),
        factor(spread.max)
    )
}

/// `77704715` as `77,704,715`.
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_of_nothing_is_nothing() {
        assert_eq!(Spread::of(&[]), None);
    }

    #[test]
    fn spread_gives_smallest_middle_and_largest() {
        assert_eq!(
            Spread::of(&[4.0]),
            Some(Spread {
                min: 4.0,
                median: 4.0,
                max: 4.0
            })
        );
        assert_eq!(
            Spread::of(&[9.0, 1.0, 5.0]),
            Some(Spread {
                min: 1.0,
                median: 5.0,
                max: 9.0
            })
        );
        // An even count: the mean of the middle two.
        assert_eq!(
            Spread::of(&[8.0, 2.0, 4.0, 100.0]),
            Some(Spread {
                min: 2.0,
                median: 6.0,
                max: 100.0
            })
        );
    }

    #[test]
    fn numbers_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(77_704_715), "77,704,715");
        assert_eq!(grouped(3_095_033_483), "3,095,033,483");
    }

    #[test]
    fn lengths_show_minutes_from_one_minute_up() {
        assert_eq!(length(5.75), "5.75 s");
        assert_eq!(length(291.4), "291.40 s (4 min 51 s)");
        assert_eq!(length(3_600.0), "3600.00 s (60 min 00 s)");
    }

    fn sample_report() -> Report {
        let spread = |value: f64| Spread {
            min: value,
            median: value,
            max: value,
        };
        let simulation = Simulation {
            worst_lag_s: 0.045,
            worst_queued_audio_s: 0.0,
            stop_wait_s: 0.045,
            chunks_that_waited: 0,
        };
        let summary = Summary {
            runs: 2,
            wall_s: Spread {
                min: 0.08,
                median: 0.0825,
                max: 0.085,
            },
            real_time_factor: Spread {
                min: 0.0139,
                median: 0.0143,
                max: 0.0148,
            },
            processing_per_audio_sent: Some(spread(0.0155)),
            per_chunk_s: Some(Spread {
                min: 0.037,
                median: 0.04,
                max: 0.045,
            }),
            chunks_without_text: 0,
            worst_lag_s: spread(0.045),
            worst_queued_audio_s: spread(0.0),
            stop_wait_s: spread(0.045),
            chunks_that_waited: 0,
        };
        Report {
            tool: Tool {
                name: "mneme-bench",
                version: "0.1.0",
                build_profile: "release",
                whisper_cpp_version: "1.8.3",
                started_at: Some("2026-11-12 21:00:00 +0100".to_owned()),
            },
            machine: Machine {
                chip: Some("Apple M2 Pro".to_owned()),
                cores: Some(12),
                performance_cores: Some(8),
                efficiency_cores: Some(4),
                memory_bytes: Some(34_359_738_368),
                os_version: Some("macOS 27.0.1".to_owned()),
            },
            load_average_before: Some([1.5, 2.0, 2.5]),
            load_average_after: Some([1.75, 2.0, 2.5]),
            model: ModelReport {
                path: "/models/ggml-tiny.en.bin".to_owned(),
                file_name: "ggml-tiny.en.bin".to_owned(),
                size_bytes: 77_704_715,
                catalog_id: Some("tiny.en".to_owned()),
                name: Some("Tiny (English)".to_owned()),
                english_only: true,
            },
            audio: AudioReport {
                path: "speech.wav".to_owned(),
                duration_s: 5.75,
                source: SourceFormat {
                    sample_rate: 48_000,
                    channels: 2,
                    bits_per_sample: 24,
                    sample_format: "int",
                    frames: 276_000,
                },
                downmixed: true,
                resampled: true,
                read_and_convert_s: 0.01,
            },
            language: "auto".to_owned(),
            load: LoadReport {
                first_s: 0.213,
                second_s: 0.052,
            },
            cutting: CuttingReport {
                method: "silence chunker",
                chunks: 2,
                dropped: Some(1),
                forced_cuts: Some(0),
                gaps: Some(0),
                chunker_config: Some(ChunkerSettings {
                    pause_ms: 700,
                    max_chunk_ms: 28_000,
                    cut_search_ms: 8_000,
                    padding_ms: 200,
                    min_speech_ms: 160,
                    speech_ratio: 3.0,
                    min_speech_level: 0.002,
                    noise_doubling_ms: 2_000,
                    max_gap_ms: 100,
                }),
                fixed_window_s: None,
                audio_sent_s: 5.32,
                audio_sent_share: 0.925,
                chunk_length_s: Some(Spread {
                    min: 2.1,
                    median: 2.66,
                    max: 3.22,
                }),
                cutting_s: 0.001,
            },
            runs: vec![
                RunReport {
                    wall_s: 0.085,
                    real_time_factor: 0.0148,
                    per_chunk_s: Some(spread(0.04)),
                    chunks_without_text: 0,
                    simulation,
                },
                RunReport {
                    wall_s: 0.08,
                    real_time_factor: 0.0139,
                    per_chunk_s: Some(spread(0.04)),
                    chunks_without_text: 0,
                    simulation,
                },
            ],
            what_if: Some(WhatIf::new(50.0, &summary, &[simulation, simulation])),
            summary,
            chunks: vec![ChunkReport {
                start_s: 0.0,
                length_s: 3.22,
                available_at_s: 3.92,
                processing_s: vec![0.045, 0.04],
                text_chars: 51,
            }],
            transcript: vec![TranscriptLine {
                start_ms: 3_725_000,
                end_ms: 3_728_000,
                text: "Hello, this is a test.".to_owned(),
                line: "[01:02:05] Hello, this is a test.".to_owned(),
            }],
            transcript_same_in_every_run: true,
            definitions: Definitions::default(),
        }
    }

    #[test]
    fn the_text_report_has_every_field() {
        let text = sample_report().to_text();
        for expected in [
            "mneme-bench 0.1.0 · release build · whisper.cpp 1.8.3",
            "Apple M2 Pro, 12 cores (8 performance, 4 efficiency), 32 GB memory, macOS 27.0.1",
            "1.50 before, 1.75 after",
            "tiny.en (Tiny (English)), ggml-tiny.en.bin, 77,704,715 bytes (78 MB), English only",
            "5.75 s; 48000 Hz, 2 channels, 24-bit int; mixed down to mono and converted to 16000 Hz",
            "Language    auto",
            "First load in this process     0.213 s",
            "Second load right after        0.052 s",
            "a pause of 700 ms ends a chunk, none longer than 28 s",
            "Chunks sent to the model       2",
            "Dropped for too little speech  1",
            "Cuts forced by the maximum     0",
            "Audio sent to the model        5.320 s of 5.750 s (92.5%)",
            "Chunk length                   min 2.100 s · median 2.660 s · max 3.220 s",
            "Chunks that gave no text       0",
            "TRANSCRIPTION (2 runs, model loaded once)",
            "Run 1                          0.085 s   real-time factor 0.0148",
            "Run 2                          0.080 s   real-time factor 0.0139",
            "Wall time                      min 0.080 s · median 0.083 s · max 0.085 s",
            "Real-time factor               min 0.0139 · median 0.0143 · max 0.0148",
            "= processing time divided by audio time; lower is faster",
            "Time per chunk                 min 0.037 s · median 0.040 s · max 0.045 s",
            "LIVE SIMULATION",
            "Furthest behind                min 0.045 s",
            "Most audio waiting             min 0.000 s",
            "Wait at stop                   min 0.045 s",
            "Chunks that had to wait        0 of 2",
            "WHAT IF every chunk took 50 times as long (a projection, not a measurement)",
            "TRANSCRIPT (run 1; the same text in every run)",
            "[01:02:05] Hello, this is a test.",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains("NOT A BENCHMARK"));
    }

    #[test]
    fn a_debug_build_says_its_numbers_are_not_a_benchmark() {
        let mut report = sample_report();
        report.tool.build_profile = "debug";
        let text = report.to_text();
        assert!(text.contains("debug build"));
        assert!(text.contains("NOT A BENCHMARK"), "{text}");
    }

    #[test]
    fn a_changing_transcript_is_called_out() {
        let mut report = sample_report();
        report.transcript_same_in_every_run = false;
        assert!(report.to_text().contains("NOT the same text in every run"));
    }

    #[test]
    fn the_what_if_projection_scales_the_real_time_factor() {
        let report = sample_report();
        let what_if = report.what_if.unwrap();
        assert_eq!(what_if.slower_by, 50.0);
        assert!((what_if.real_time_factor.median - 0.0143 * 50.0).abs() < 1e-9);
        assert!((what_if.real_time_factor.max - 0.0148 * 50.0).abs() < 1e-9);
    }

    #[test]
    fn the_json_report_carries_the_numbers_for_the_table() {
        let json: serde_json::Value = serde_json::from_str(&sample_report().to_json()).unwrap();
        assert_eq!(json["tool"]["build_profile"], "release");
        assert_eq!(json["machine"]["chip"], "Apple M2 Pro");
        assert_eq!(json["model"]["catalog_id"], "tiny.en");
        assert_eq!(json["model"]["size_bytes"], 77_704_715);
        assert_eq!(json["audio"]["duration_s"], 5.75);
        assert_eq!(json["audio"]["source"]["sample_rate"], 48_000);
        assert_eq!(json["load"]["first_s"], 0.213);
        assert_eq!(json["load"]["second_s"], 0.052);
        assert_eq!(json["cutting"]["chunks"], 2);
        assert_eq!(json["cutting"]["dropped"], 1);
        assert_eq!(json["cutting"]["forced_cuts"], 0);
        assert_eq!(json["cutting"]["chunker_config"]["pause_ms"], 700);
        assert_eq!(json["runs"].as_array().unwrap().len(), 2);
        assert_eq!(json["runs"][0]["wall_s"], 0.085);
        assert_eq!(json["summary"]["real_time_factor"]["median"], 0.0143);
        assert_eq!(json["summary"]["per_chunk_s"]["max"], 0.045);
        assert_eq!(json["summary"]["worst_lag_s"]["max"], 0.045);
        assert_eq!(json["summary"]["stop_wait_s"]["max"], 0.045);
        assert_eq!(json["what_if"]["slower_by"], 50.0);
        assert_eq!(json["chunks"][0]["processing_s"][1], 0.04);
        assert_eq!(
            json["transcript"][0]["line"],
            "[01:02:05] Hello, this is a test."
        );
        assert_eq!(json["transcript_same_in_every_run"], true);
        assert!(
            json["definitions"]["real_time_factor"]
                .as_str()
                .unwrap()
                .contains("processing time divided by audio time")
        );
    }
}
