//! Audio retention: nothing is written by default; `MNEME_KEEP_AUDIO=1`
//! writes `mic.wav` and `system.wav` from the frames.
//!
//! The switch is an environment variable, and a process's environment is
//! shared by all its threads, so these tests do not change their own. Each
//! one runs this test binary again as a child process with the variable
//! set, or removed, for that child alone. The child runs
//! [`child_records_a_short_meeting`], which is a test like any other and
//! does nothing when the suite runs it directly.

mod common;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use mneme_audio::read_wav_16k_mono;
use mneme_core::Speaker;
use mneme_pipeline::{KEEP_AUDIO_ENV, Session, SessionConfig, keep_audio_enabled};
use mneme_transcribe::FakeTranscriber;

use common::{RecordingSink, Tape, config, sources, watchdog};

/// Tells the child where the session may keep audio.
const AUDIO_DIR_ENV: &str = "MNEME_PIPELINE_TEST_AUDIO_DIR";
/// Tells the child where to write what it did.
const REPORT_ENV: &str = "MNEME_PIPELINE_TEST_REPORT";

/// Two seconds, one burst.
fn mic_tape() -> Tape {
    Tape::new().burst_at(400, 300).until(2_000)
}

/// One second with one burst, played from 0.5 s into the meeting.
fn system_tape() -> Tape {
    Tape::new().burst_at(200, 300).until(1_000)
}

const SYSTEM_STARTS_AT_MS: u64 = 500;

#[test]
fn child_records_a_short_meeting() {
    let (Some(audio_dir), Some(report)) = (env::var_os(AUDIO_DIR_ENV), env::var_os(REPORT_ENV))
    else {
        // Not a child of one of the tests below.
        return;
    };
    watchdog(move || {
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                mic_tape().source(Speaker::Me),
                system_tape()
                    .source(Speaker::Them)
                    .starting_at(Duration::from_millis(SYSTEM_STARTS_AT_MS)),
            ]),
            Arc::new(FakeTranscriber::new()),
            sink.for_session(),
            // What the app does: always name the meeting folder.
            SessionConfig {
                audio_dir: Some(PathBuf::from(audio_dir)),
                ..config()
            },
        )
        .unwrap();
        let summary = session.wait();
        let audio = match &summary.audio {
            None => "none".to_owned(),
            Some(kept) => format!(
                "{} files, error {:?}, in {}",
                kept.files.len(),
                kept.error,
                kept.dir.display()
            ),
        };
        let text = format!(
            "enabled={}\nlines={}\naudio={audio}\naudio_error={:?}\n",
            keep_audio_enabled(),
            summary.lines,
            session.status().audio_error,
        );
        fs::write(report, text).unwrap();
    });
}

struct Child {
    /// Keeps the temp folder alive.
    _tmp: tempfile::TempDir,
    root: PathBuf,
    meeting: PathBuf,
    report: String,
}

/// Runs the child with `MNEME_KEEP_AUDIO` set to `keep`, or unset. The
/// meeting folder exists beforehand, with a `notes.md` in it, unless
/// `folder_exists` is false.
fn run_child(keep: Option<&str>, folder_exists: bool) -> Child {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let meeting = root.join("2026-11-12 1000 Meeting");
    if folder_exists {
        fs::create_dir(&meeting).unwrap();
        fs::write(meeting.join("notes.md"), "notes\n").unwrap();
    }
    let report = root.join("report.txt");

    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "child_records_a_short_meeting",
            "--exact",
            "--test-threads=1",
        ])
        .env(AUDIO_DIR_ENV, &meeting)
        .env(REPORT_ENV, &report);
    match keep {
        Some(value) => command.env(KEEP_AUDIO_ENV, value),
        None => command.env_remove(KEEP_AUDIO_ENV),
    };
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "the child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(&report).expect("the child ran the session and reported");
    Child {
        _tmp: tmp,
        root,
        meeting,
        report,
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn without_the_variable_no_audio_is_written() {
    let child = run_child(None, true);
    assert_eq!(
        child.report,
        "enabled=false\nlines=2\naudio=none\naudio_error=None\n"
    );
    // The meeting folder holds what it held before, and nothing appeared
    // next to it.
    assert_eq!(names_in(&child.meeting), ["notes.md"]);
    assert_eq!(
        names_in(&child.root),
        ["2026-11-12 1000 Meeting", "report.txt"]
    );
}

#[test]
fn without_the_variable_the_folder_is_not_even_created() {
    let child = run_child(None, false);
    assert!(
        child
            .report
            .starts_with("enabled=false\nlines=2\naudio=none\n")
    );
    assert!(!child.meeting.exists());
    assert_eq!(names_in(&child.root), ["report.txt"]);
}

#[test]
fn any_value_but_1_writes_no_audio() {
    for value in ["0", "", "true"] {
        let child = run_child(Some(value), true);
        assert!(
            child
                .report
                .starts_with("enabled=false\nlines=2\naudio=none\n"),
            "{value:?}: {}",
            child.report
        );
        assert_eq!(names_in(&child.meeting), ["notes.md"], "{value:?}");
    }
}

#[test]
fn with_the_variable_both_channels_are_written_whole_and_lined_up() {
    let child = run_child(Some("1"), true);
    assert_eq!(
        child.report,
        format!(
            "enabled=true\nlines=2\naudio=2 files, error None, in {}\naudio_error=None\n",
            child.meeting.display()
        )
    );
    assert_eq!(
        names_in(&child.meeting),
        ["mic.wav", "notes.md", "system.wav"]
    );

    let mic = read_wav_16k_mono(child.meeting.join("mic.wav")).unwrap();
    let system = read_wav_16k_mono(child.meeting.join("system.wav")).unwrap();
    // Both as long as the longer channel: two seconds.
    assert_eq!(mic.len(), 32_000);
    assert_eq!(system.len(), 32_000);

    // Every frame is there, silence included (the chunker would have
    // dropped it), to within the 16-bit rounding of the file format.
    let close = |a: &[f32], b: &[f32]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(a, b)| (a - b).abs() <= 1.0 / 32_768.0)
    };
    assert!(
        close(&mic, &mic_tape().samples()),
        "mic.wav is not what was played"
    );

    // The system channel begins 0.5 s into the meeting: silence, then its
    // audio at the right place, then silence to the common length.
    let begins = (SYSTEM_STARTS_AT_MS * 16) as usize;
    let played = system_tape().samples();
    assert!(system[..begins].iter().all(|s| *s == 0.0));
    assert!(
        close(&system[begins..begins + played.len()], &played),
        "system.wav is not what was played"
    );
    assert!(system[begins + played.len()..].iter().all(|s| *s == 0.0));
}

#[test]
fn with_the_variable_a_folder_that_cannot_be_written_costs_the_audio_not_the_meeting() {
    // The "folder" is a file: the WAVs cannot be created.
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("not-a-folder");
    fs::write(&blocker, "x").unwrap();
    let report = tmp.path().join("report.txt");
    let output = Command::new(env::current_exe().unwrap())
        .args([
            "child_records_a_short_meeting",
            "--exact",
            "--test-threads=1",
        ])
        .env(AUDIO_DIR_ENV, blocker.join("meeting"))
        .env(REPORT_ENV, &report)
        .env(KEEP_AUDIO_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = fs::read_to_string(&report).unwrap();
    assert!(
        report.starts_with("enabled=true\nlines=2\naudio=0 files, error Some("),
        "{report}"
    );
    assert!(!report.contains("audio_error=None"), "{report}");
}
