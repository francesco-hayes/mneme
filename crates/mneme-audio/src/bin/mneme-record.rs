//! `mneme-record`: records the microphone and system audio to `mic.wav` and
//! `system.wav` (PRD 01, build step 1). A development tool.
//!
//! The work is in the library: [`mneme_audio::record_to_wav`] is the
//! recording loop, tested there with fake sources. This file parses the
//! arguments, turns Ctrl-C into a stop flag, and prints what happened.
//!
//! How to run it and which permissions it needs: `crates/mneme-audio/README.md`.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use mneme_audio::{
    AudioSource, FakeSource, MicSource, Pacing, RecordEvent, RecordOptions, Recording, SourcePhase,
    SourceStatus, StopReason, SystemAudioSource, TrackSummary, microphone_permission_status,
    record_to_wav, system_audio_permission_status,
};
use mneme_core::{SAMPLE_RATE, Speaker};

const USAGE: &str = "Usage: mneme-record --out <dir> [--seconds <n>] [--fake]";

const HELP: &str = "\
mneme-record: record the microphone and system audio to two WAV files.

Usage: mneme-record --out <dir> [--seconds <n>] [--fake]

Options:
  --out <dir>     Folder for mic.wav and system.wav. Created if missing;
                  files of those names in it are replaced.
  --seconds <n>   Stop after n seconds (decimals allowed). Without it the
                  recording runs until Ctrl-C.
  --fake          Record two synthetic tones instead of real audio: 440 Hz
                  as the microphone, 880 Hz as system audio starting 0.5 s
                  late. Opens no device and needs no permission; for
                  checking the tool itself. Needs --seconds.
  -h, --help      Show this text.

Output:
  mic.wav      the microphone (you)
  system.wav   what the Mac is playing (the other side of a call)
  Both are 16 kHz mono 16-bit PCM, have the same length and line up in
  time: wherever a channel has no audio (it started late, the device
  changed, it failed) its file has silence instead.

Stopping:
  Ctrl-C (or SIGTERM) stops the recording and finishes both files. A second
  Ctrl-C quits at once: the files stay playable but can be up to a second
  short and unequal in length.

Permissions:
  macOS asks for Microphone and for System Audio Recording, on behalf of
  the app that started this program (the terminal), or of the bundle when
  run through crates/mneme-audio/scripts/record.sh. Without the second one
  system.wav is silent and no error is reported. Use headphones: with
  speakers, the other side is picked up by the microphone as well.
  See crates/mneme-audio/README.md.

Environment:
  MNEME_SKIP_TAP_ACCESS_CHECK=1   start system audio without checking that
                                  macOS allows it (a diagnostic; see README)

Exit status:
  0  recorded; both sources ran until the recording was stopped
  1  a source failed, or the files could not be written
  2  wrong usage
";

/// Longest `--fake` recording, to bound the memory the tones take.
const MAX_FAKE_SECONDS: f64 = 600.0;

#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Record(Args),
}

#[derive(Debug, PartialEq)]
struct Args {
    out: PathBuf,
    limit: Option<Duration>,
    fake: bool,
}

/// Parses the arguments after the program name.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut out: Option<PathBuf> = None;
    let mut limit: Option<Duration> = None;
    let mut fake = false;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        // `--name=value` and `--name value` are both accepted.
        let (name, mut inline) = match arg.to_str() {
            Some(text) => match text.split_once('=') {
                Some((name, value)) if name.starts_with("--") => {
                    (name.to_owned(), Some(OsString::from(value)))
                }
                _ => (text.to_owned(), None),
            },
            // Not UTF-8: cannot be an option name.
            None => return Err(format!("unexpected argument {arg:?}")),
        };
        let mut value = |name: &str| {
            inline
                .take()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--out" => {
                let dir = value("--out")?;
                if dir.is_empty() {
                    return Err("--out needs a folder".to_owned());
                }
                if out.replace(PathBuf::from(dir)).is_some() {
                    return Err("--out was given twice".to_owned());
                }
            }
            "--seconds" => {
                let text = value("--seconds")?;
                let seconds = text
                    .to_str()
                    .and_then(|t| t.parse::<f64>().ok())
                    .filter(|s| s.is_finite() && *s > 0.0 && *s <= 86_400.0)
                    .ok_or_else(|| {
                        format!(
                            "--seconds needs a number of seconds above 0 (at most 86400), not {text:?}"
                        )
                    })?;
                if limit.replace(Duration::from_secs_f64(seconds)).is_some() {
                    return Err("--seconds was given twice".to_owned());
                }
            }
            "--fake" => {
                if inline.is_some() {
                    return Err("--fake takes no value".to_owned());
                }
                fake = true;
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }

    let out = out.ok_or("--out <dir> is required")?;
    if fake {
        match limit {
            None => return Err("--fake needs --seconds".to_owned()),
            Some(limit) if limit.as_secs_f64() > MAX_FAKE_SECONDS => {
                return Err(format!("--fake records at most {MAX_FAKE_SECONDS} seconds"));
            }
            Some(_) => {}
        }
    }
    Ok(Command::Record(Args { out, limit, fake }))
}

/// Makes SIGINT (Ctrl-C), SIGTERM and SIGHUP set the returned flag instead
/// of killing the process, so that the recording loop can stop the sources
/// and finish both WAV files. A second signal ends the process at once, in
/// case stopping a device hangs.
///
/// This is the pattern from `signal-hook`'s documentation for a graceful
/// shutdown with an impatient second Ctrl-C: for each signal, first the
/// conditional shutdown (does nothing while the flag is unset), then the
/// action that sets the flag. The order matters.
fn stop_on_signals() -> std::io::Result<Arc<AtomicBool>> {
    use signal_hook::consts::signal::{SIGINT, SIGTERM};
    use signal_hook::flag;

    let stop = Arc::new(AtomicBool::new(false));
    #[allow(unused_mut)]
    let mut signals = vec![SIGINT, SIGTERM];
    #[cfg(unix)]
    signals.push(signal_hook::consts::signal::SIGHUP);
    for signal in signals {
        flag::register_conditional_shutdown(signal, FORCED_EXIT, Arc::clone(&stop))?;
        flag::register(signal, Arc::clone(&stop))?;
    }
    Ok(stop)
}

/// Exit status when a second signal cuts the shutdown short (the shell
/// convention for "ended by SIGINT").
const FORCED_EXIT: i32 = 130;

fn real_sources() -> Vec<Box<dyn AudioSource>> {
    let mut system = SystemAudioSource::new();
    if std::env::var_os("MNEME_SKIP_TAP_ACCESS_CHECK").is_some_and(|v| v == "1") {
        eprintln!("MNEME_SKIP_TAP_ACCESS_CHECK=1: not checking access to system audio");
        system = system.without_access_check();
    }
    // The microphone first: see `Recorder` on the order of starting.
    vec![Box::new(MicSource::new()), Box::new(system)]
}

/// Two tones delivered in real time, a little longer than the recording.
fn fake_sources(limit: Duration) -> Vec<Box<dyn AudioSource>> {
    let len = ((limit.as_secs_f64() + 2.0) * f64::from(SAMPLE_RATE)) as usize;
    let tone = |freq: f64| -> Vec<f32> {
        (0..len)
            .map(|i| {
                let phase = std::f64::consts::TAU * freq * i as f64 / f64::from(SAMPLE_RATE);
                (0.25 * phase.sin()) as f32
            })
            .collect()
    };
    let mic = FakeSource::from_samples(Speaker::Me, tone(440.0)).with_pacing(Pacing::RealTime);
    let system = FakeSource::from_samples(Speaker::Them, tone(880.0))
        .with_pacing(Pacing::RealTime)
        .starting_at(Duration::from_millis(500));
    vec![Box::new(mic), Box::new(system)]
}

fn source_name(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::Me => "microphone",
        Speaker::Them => "system audio",
    }
}

fn seconds(duration: Duration) -> String {
    format!("{:.2} s", duration.as_secs_f64())
}

fn ms(ms: u64) -> String {
    seconds(Duration::from_millis(ms))
}

fn reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Interrupted => "interrupted (Ctrl-C or a signal)",
        StopReason::LimitReached => "the time limit was reached",
        StopReason::SourcesEnded => "every source had ended or failed",
    }
}

/// One line for something that happened while recording.
fn event_line(at: Duration, event: &RecordEvent) -> String {
    let what = match event {
        RecordEvent::Phase { speaker, phase } => {
            let name = source_name(*speaker);
            match phase {
                SourcePhase::Starting => format!("{name}: starting"),
                SourcePhase::Running => format!("{name}: capturing"),
                SourcePhase::FailedToStart => format!("{name}: FAILED TO START"),
                SourcePhase::Ended => format!("{name}: ENDED by itself"),
                SourcePhase::Stopped => format!("{name}: stopped"),
                SourcePhase::Abandoned => format!("{name}: given up on, it never started"),
            }
        }
        RecordEvent::Error { speaker, error } => {
            format!("{}: error: {error}", source_name(*speaker))
        }
        RecordEvent::Stopping(reason) => format!("stopping: {}", reason_text(*reason)),
    };
    format!("[{:>8}] {what}", seconds(at))
}

/// What became of a source, for the summary.
fn phase_text(status: &SourceStatus) -> String {
    match status.phase {
        SourcePhase::Stopped => "captured until the recording stopped".to_owned(),
        SourcePhase::Ended => match status.end_ms {
            Some(end) => format!("ENDED BY ITSELF, last audio at {}", ms(end)),
            None => "ENDED BY ITSELF without delivering any audio".to_owned(),
        },
        SourcePhase::FailedToStart => "FAILED TO START".to_owned(),
        SourcePhase::Abandoned => "NEVER STARTED: its start had not returned when the \
             recording stopped (an unanswered permission prompt?)"
            .to_owned(),
        // Not final states; a summary should never show them.
        SourcePhase::Starting => "still starting".to_owned(),
        SourcePhase::Running => "still running".to_owned(),
    }
}

fn peak_text(track: &TrackSummary) -> String {
    if track.captured_samples == 0 {
        "no audio".to_owned()
    } else if track.is_silent() {
        "peak: none, all of it is digital silence".to_owned()
    } else {
        format!("peak {:.1} dBFS", 20.0 * f64::from(track.peak).log10())
    }
}

/// Things the numbers do not say by themselves.
fn notes(recording: &Recording) -> Vec<String> {
    let mut notes = Vec::new();
    for track in &recording.tracks {
        if !track.is_silent() {
            continue;
        }
        notes.push(match track.speaker {
            Speaker::Them => "system.wav holds only digital silence, and that is not a success: \
                 either nothing was playing, or macOS is withholding system audio \
                 (without the System Audio Recording permission it delivers silence, \
                 not an error). This tool cannot tell the two apart. Play something \
                 and record again; if it stays silent, look at System Settings > \
                 Privacy & Security > Screen & System Audio Recording."
                .to_owned(),
            Speaker::Me => "mic.wav holds only digital silence: the input may be muted, or \
                 macOS may be withholding the microphone (System Settings > Privacy \
                 & Security > Microphone). This tool cannot tell which."
                .to_owned(),
        });
    }
    for track in &recording.tracks {
        if track.gaps > 0 {
            notes.push(format!(
                "{} has {} gap(s) filled with silence: the source re-opened its device \
                 or stalled (a device change does this once).",
                file_name(track),
                track.gaps
            ));
        }
        if track.dropped_samples > 0 && recording.reason != StopReason::LimitReached {
            notes.push(format!(
                "{}: {} samples were left out because their timestamps overlapped \
                 audio already written. That should not happen; please report it.",
                file_name(track),
                track.dropped_samples
            ));
        }
    }
    notes
}

fn file_name(track: &TrackSummary) -> String {
    track
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The report printed when the recording is over.
fn summary(recording: &Recording) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Recording ended: {}.", reason_text(recording.reason));
    let _ = writeln!(
        text,
        "Duration: {} (both files)",
        seconds(recording.duration)
    );

    let mut error_count = 0;
    for track in &recording.tracks {
        let _ = writeln!(text);
        let _ = writeln!(
            text,
            "{}: {} captured, {} silence padded, {}",
            file_name(track),
            seconds(track.captured()),
            seconds(track.padded()),
            peak_text(track)
        );
        if let (Some(first), Some(last)) = (track.first_audio_ms, track.last_audio_ms) {
            let _ = writeln!(text, "  audio from {} to {}", ms(first), ms(last));
        }
        let name = source_name(track.speaker);
        let sources: Vec<&SourceStatus> = recording
            .sources
            .iter()
            .filter(|s| s.speaker == track.speaker)
            .collect();
        if sources.is_empty() {
            let _ = writeln!(text, "  {name}: no such source was recorded");
        }
        for source in sources {
            let _ = writeln!(text, "  {name}: {}", phase_text(source));
            if let Some(started) = source.started_ms.filter(|ms| *ms >= 1_000) {
                let _ = writeln!(text, "  {name}: took {} to start", ms(started));
            }
            for error in &source.errors {
                error_count += 1;
                let _ = writeln!(text, "  {name}: error: {error}");
            }
        }
    }

    let _ = writeln!(text);
    let _ = match error_count {
        0 => writeln!(text, "Source errors: none"),
        n => writeln!(text, "Source errors: {n} (listed above)"),
    };
    for note in notes(recording) {
        let _ = writeln!(text, "Note: {note}");
    }
    let _ = writeln!(text);
    for track in &recording.tracks {
        let _ = writeln!(text, "{}", track.path.display());
    }
    text
}

fn run(args: Args) -> ExitCode {
    let stop = match stop_on_signals() {
        Ok(stop) => stop,
        Err(err) => {
            eprintln!("error: could not install the signal handlers: {err}");
            return ExitCode::from(1);
        }
    };

    let sources = match (args.fake, args.limit) {
        (true, Some(limit)) => {
            eprintln!("--fake: recording two synthetic tones; no device is opened");
            fake_sources(limit)
        }
        _ => {
            eprintln!(
                "Microphone permission: {:?}. System Audio Recording permission: {}.",
                microphone_permission_status(),
                match system_audio_permission_status() {
                    Some(status) => format!("{status:?}"),
                    None => "unknown (macOS has no public way to ask)".to_owned(),
                }
            );
            eprintln!(
                "If macOS shows a permission prompt, allow it: each channel starts \
                 once its prompt is answered."
            );
            real_sources()
        }
    };

    match args.limit {
        Some(limit) => eprintln!(
            "Recording to {} for {}. Ctrl-C stops earlier.",
            args.out.display(),
            seconds(limit)
        ),
        None => eprintln!("Recording to {}. Ctrl-C to stop.", args.out.display()),
    }

    let options = RecordOptions {
        limit: args.limit,
        ..RecordOptions::default()
    };
    let begun = Instant::now();
    let mut print_event = |event: RecordEvent| eprintln!("{}", event_line(begun.elapsed(), &event));
    match record_to_wav(sources, &args.out, &options, &stop, &mut print_event) {
        Ok(recording) => {
            eprintln!();
            print!("{}", summary(&recording));
            if recording.any_source_failed() {
                eprintln!();
                eprintln!("error: not every source ran to the end; see above");
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    match parse_args(std::env::args_os().skip(1)) {
        Ok(Command::Help) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Ok(Command::Record(args)) => run(args),
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("{USAGE}");
            eprintln!("Try: mneme-record --help");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use mneme_audio::AudioError;

    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(OsString::from))
    }

    fn record(out: &str, limit: Option<Duration>, fake: bool) -> Result<Command, String> {
        Ok(Command::Record(Args {
            out: PathBuf::from(out),
            limit,
            fake,
        }))
    }

    #[test]
    fn the_documented_forms_parse() {
        assert_eq!(parse(&["--out", "rec"]), record("rec", None, false));
        assert_eq!(
            parse(&["--out", "/tmp/a b", "--seconds", "30"]),
            record("/tmp/a b", Some(Duration::from_secs(30)), false)
        );
        assert_eq!(
            parse(&["--seconds=1.5", "--out=rec"]),
            record("rec", Some(Duration::from_millis(1500)), false)
        );
        assert_eq!(
            parse(&["--fake", "--seconds", "2", "--out", "rec"]),
            record("rec", Some(Duration::from_secs(2)), true)
        );
        // A value may itself contain `=`.
        assert_eq!(parse(&["--out=a=b"]), record("a=b", None, false));
    }

    #[test]
    fn help_wins_wherever_it_stands() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--out", "rec", "--help"]), Ok(Command::Help));
    }

    #[test]
    fn wrong_arguments_are_refused_with_a_reason() {
        let error = |args: &[&str]| parse(args).unwrap_err();
        assert_eq!(error(&[]), "--out <dir> is required");
        assert_eq!(error(&["--seconds", "5"]), "--out <dir> is required");
        assert_eq!(error(&["--out"]), "--out needs a value");
        assert_eq!(error(&["--out="]), "--out needs a folder");
        assert_eq!(
            error(&["--out", "a", "--out", "b"]),
            "--out was given twice"
        );
        assert_eq!(
            error(&["--out", "a", "--seconds"]),
            "--seconds needs a value"
        );
        for bad in ["0", "-3", "abc", "nan", "inf", "1e9", ""] {
            let message = error(&["--out", "a", "--seconds", bad]);
            assert!(message.starts_with("--seconds needs a number"), "{message}");
        }
        assert_eq!(
            error(&["--out", "a", "--seconds", "1", "--seconds", "2"]),
            "--seconds was given twice"
        );
        assert_eq!(error(&["--out", "a", "rec"]), "unexpected argument \"rec\"");
        assert_eq!(
            error(&["--out", "a", "--loud"]),
            "unexpected argument \"--loud\""
        );
        assert_eq!(error(&["--out", "a", "--fake"]), "--fake needs --seconds");
        assert_eq!(
            error(&["--out", "a", "--fake=yes", "--seconds", "1"]),
            "--fake takes no value"
        );
        assert_eq!(
            error(&["--out", "a", "--fake", "--seconds", "601"]),
            "--fake records at most 600 seconds"
        );
    }

    #[test]
    fn the_help_text_covers_every_option() {
        for needle in [
            "--out",
            "--seconds",
            "--fake",
            "--help",
            "Ctrl-C",
            "mic.wav",
            "system.wav",
        ] {
            assert!(HELP.contains(needle), "help does not mention {needle}");
        }
        assert!(HELP.contains(USAGE));
    }

    #[test]
    fn a_signal_sets_the_stop_flag_instead_of_killing_the_process() {
        let stop = stop_on_signals().unwrap();
        assert!(!stop.load(Ordering::SeqCst));
        // Without the handler this would end the test process.
        signal_hook::low_level::raise(signal_hook::consts::signal::SIGINT).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !stop.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the flag was never set");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn track(speaker: Speaker, captured: u64, padded: u64, peak: f32) -> TrackSummary {
        TrackSummary {
            speaker,
            path: PathBuf::from(match speaker {
                Speaker::Me => "/rec/mic.wav",
                Speaker::Them => "/rec/system.wav",
            }),
            samples: captured + padded,
            captured_samples: captured,
            padded_samples: padded,
            dropped_samples: 0,
            gaps: 0,
            peak,
            first_audio_ms: (captured > 0).then_some(0),
            last_audio_ms: (captured > 0).then_some(captured / 16),
        }
    }

    fn source(speaker: Speaker, phase: SourcePhase, errors: Vec<AudioError>) -> SourceStatus {
        SourceStatus {
            speaker,
            phase,
            errors,
            started_ms: Some(12),
            frames: 0,
            samples: 0,
            first_frame_ms: None,
            end_ms: Some(4_000),
        }
    }

    fn recording(tracks: [TrackSummary; 2], sources: Vec<SourceStatus>) -> Recording {
        Recording {
            reason: StopReason::Interrupted,
            duration: tracks[0].duration(),
            tracks,
            sources,
        }
    }

    #[test]
    fn the_summary_gives_duration_capture_padding_and_errors_per_source() {
        let died = AudioError::Stream("device unplugged".to_owned());
        let reopened = AudioError::NoInputDevice;
        let text = summary(&recording(
            [
                track(Speaker::Me, 160_000, 0, 0.5),
                track(Speaker::Them, 64_000, 96_000, 0.25),
            ],
            vec![
                source(Speaker::Me, SourcePhase::Stopped, vec![reopened]),
                source(Speaker::Them, SourcePhase::Ended, vec![died]),
            ],
        ));
        println!("{text}");
        assert!(text.contains("Recording ended: interrupted"));
        assert!(text.contains("Duration: 10.00 s (both files)"));
        assert!(text.contains("mic.wav: 10.00 s captured, 0.00 s silence padded, peak -6.0 dBFS"));
        assert!(
            text.contains("system.wav: 4.00 s captured, 6.00 s silence padded, peak -12.0 dBFS")
        );
        assert!(text.contains("microphone: captured until the recording stopped"));
        assert!(text.contains("system audio: ENDED BY ITSELF, last audio at 4.00 s"));
        // Every error, each under its source: one that ended the source and
        // one the source recovered from.
        assert!(text.contains("microphone: error: no default input device"));
        assert!(text.contains("system audio: error: audio stream error: device unplugged"));
        assert!(text.contains("Source errors: 2"));
        assert!(text.contains("/rec/mic.wav\n/rec/system.wav\n"));
        assert!(!text.contains("Note:"));
    }

    #[test]
    fn a_silent_system_file_is_called_out_not_passed_off_as_success() {
        // Everything "worked": both sources ran, no errors, but the tap gave
        // nothing but zeros.
        let text = summary(&recording(
            [
                track(Speaker::Me, 160_000, 0, 0.5),
                track(Speaker::Them, 160_000, 0, 0.0),
            ],
            vec![
                source(Speaker::Me, SourcePhase::Stopped, vec![]),
                source(Speaker::Them, SourcePhase::Stopped, vec![]),
            ],
        ));
        println!("{text}");
        assert!(text.contains("system.wav: 10.00 s captured"));
        assert!(text.contains("all of it is digital silence"));
        assert!(text.contains("Source errors: none"));
        assert!(text.contains("Note: system.wav holds only digital silence"));
        assert!(text.contains("nothing was playing"));
        assert!(text.contains("System Audio Recording permission"));
        assert!(text.contains("cannot tell the two apart"));
        // The microphone is fine and is not mentioned in a note.
        assert!(!text.contains("Note: mic.wav"));

        // A silent microphone gets its own note.
        let text = summary(&recording(
            [
                track(Speaker::Me, 160_000, 0, 0.0),
                track(Speaker::Them, 160_000, 0, 0.3),
            ],
            vec![
                source(Speaker::Me, SourcePhase::Stopped, vec![]),
                source(Speaker::Them, SourcePhase::Stopped, vec![]),
            ],
        ));
        assert!(text.contains("Note: mic.wav holds only digital silence"));
        assert!(!text.contains("Note: system.wav"));
    }

    #[test]
    fn sources_that_never_ran_are_spelled_out() {
        let denied = AudioError::PermissionDenied("system audio".to_owned());
        let mut slow = source(Speaker::Me, SourcePhase::Abandoned, vec![]);
        slow.started_ms = None;
        let mut failed = source(Speaker::Them, SourcePhase::FailedToStart, vec![denied]);
        failed.started_ms = Some(61_500);
        let text = summary(&recording(
            [
                track(Speaker::Me, 0, 16_000, 0.0),
                track(Speaker::Them, 0, 16_000, 0.0),
            ],
            vec![slow, failed],
        ));
        println!("{text}");
        assert!(text.contains("mic.wav: 0.00 s captured, 1.00 s silence padded, no audio"));
        assert!(text.contains("microphone: NEVER STARTED"));
        assert!(text.contains("system audio: FAILED TO START"));
        assert!(text.contains("system audio: took 61.50 s to start"));
        assert!(text.contains("system audio: error: audio permission denied: system audio"));
        // No audio at all is not "captured silence": no silence note.
        assert!(!text.contains("Note:"));
    }

    #[test]
    fn gaps_are_explained() {
        let mut system = track(Speaker::Them, 100_000, 60_000, 0.3);
        system.gaps = 2;
        let text = summary(&recording(
            [track(Speaker::Me, 160_000, 0, 0.5), system],
            vec![
                source(Speaker::Me, SourcePhase::Stopped, vec![]),
                source(Speaker::Them, SourcePhase::Stopped, vec![]),
            ],
        ));
        assert!(text.contains("Note: system.wav has 2 gap(s) filled with silence"));
    }

    #[test]
    fn events_read_as_one_line_each() {
        let line = event_line(
            Duration::from_millis(1_234),
            &RecordEvent::Phase {
                speaker: Speaker::Them,
                phase: SourcePhase::Running,
            },
        );
        assert_eq!(line, "[  1.23 s] system audio: capturing");
        let line = event_line(
            Duration::from_secs(70),
            &RecordEvent::Error {
                speaker: Speaker::Me,
                error: AudioError::NoInputDevice,
            },
        );
        assert_eq!(
            line,
            "[ 70.00 s] microphone: error: no default input device"
        );
        let line = event_line(
            Duration::ZERO,
            &RecordEvent::Stopping(StopReason::LimitReached),
        );
        assert_eq!(line, "[  0.00 s] stopping: the time limit was reached");
    }

    #[test]
    fn fake_sources_are_a_late_system_channel_next_to_the_microphone() {
        let sources = fake_sources(Duration::from_secs(1));
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].speaker(), Speaker::Me);
        assert_eq!(sources[1].speaker(), Speaker::Them);
    }
}
