//! Runs the built `mneme-record` binary. Nothing here opens a device: the
//! recordings use `--fake`, which plays two synthetic tones through the same
//! loop as a real recording.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_mneme-record");

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mneme-record-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The samples of a WAV the tool wrote, after checking that its header is
/// the finished one: the right format, and sizes that match the file.
fn read_wav(path: &Path) -> Vec<i16> {
    let reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, 16_000, 16)
    );
    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().len(),
        44 + 2 * samples.len() as u64,
        "{} was not finished",
        path.display()
    );
    samples
}

fn peak(samples: &[i16]) -> i16 {
    samples
        .iter()
        .map(|s| s.saturating_abs())
        .max()
        .unwrap_or(0)
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    for flag in ["--help", "-h"] {
        let output = run(&[flag]);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Usage: mneme-record --out <dir> [--seconds <n>]"));
        assert!(text.contains("mic.wav") && text.contains("system.wav"));
        assert!(text.contains("Ctrl-C"));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn wrong_usage_exits_with_2_and_says_why() {
    for (args, reason) in [
        (&[][..], "--out <dir> is required"),
        (&["--seconds", "5"][..], "--out <dir> is required"),
        (
            &["--out", "x", "--seconds", "soon"][..],
            "--seconds needs a number",
        ),
        (&["--out", "x", "--bogus"][..], "unexpected argument"),
        (&["--out", "x", "--fake"][..], "--fake needs --seconds"),
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let text = String::from_utf8(output.stderr).unwrap();
        assert!(text.contains(reason), "{args:?}: {text}");
        assert!(text.contains("Usage: mneme-record"));
        assert!(output.stdout.is_empty());
    }
    // Nothing was created for a refused command line.
    assert!(!Path::new("x").exists());
}

#[test]
fn a_timed_fake_recording_writes_two_aligned_files() {
    let dir = temp_dir("timed");
    let out = dir.join("nested/out");
    let output = run(&["--out", out.to_str().unwrap(), "--fake", "--seconds", "1"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");

    let mic = read_wav(&out.join("mic.wav"));
    let system = read_wav(&out.join("system.wav"));
    // Exactly the second asked for, both.
    assert_eq!((mic.len(), system.len()), (16_000, 16_000));
    // The microphone tone from the start; system audio only after its
    // half-second of lateness, which is padded with silence.
    assert!(peak(&mic[..1_600]) > 7_000);
    assert_eq!(peak(&system[..8_000]), 0);
    assert!(peak(&system[8_000..9_600]) > 7_000);

    assert!(stdout.contains("Recording ended: the time limit was reached."));
    assert!(stdout.contains("Duration: 1.00 s (both files)"));
    assert!(stdout.contains("mic.wav:") && stdout.contains("system.wav:"));
    assert!(stdout.contains("0.50 s silence padded"));
    assert!(stdout.contains("Source errors: none"));
    assert!(stdout.contains(out.join("mic.wav").to_str().unwrap()));
    assert!(stderr.contains("microphone: capturing"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ctrl_c_finishes_both_files() {
    let dir = temp_dir("interrupted");
    let mut child = Command::new(BIN)
        .args(["--out", dir.to_str().unwrap(), "--fake", "--seconds", "120"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Wait until it says it is recording (a freshly built binary can take
    // a while to get going), let it record for a while, then do what Ctrl-C
    // does.
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut said = String::new();
    while !said.contains("system audio: capturing") {
        let read = stderr.read_line(&mut said).unwrap();
        assert!(read > 0, "it ended before recording:\n{said}");
    }
    let recording_since = Instant::now();
    std::thread::sleep(Duration::from_millis(700));
    let killed = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    stderr.read_to_string(&mut said).unwrap();
    let output = child.wait_with_output().unwrap();
    let took = recording_since.elapsed();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // It ended by itself, in good order, long before its two minutes.
    assert_eq!(
        output.status.code(),
        Some(0),
        "{:?}\n{stdout}\n{said}",
        output.status
    );
    assert!(took < Duration::from_secs(60), "took {took:?}");
    assert!(said.contains("stopping: interrupted"), "{said}");
    assert!(stdout.contains("Recording ended: interrupted"));

    // `read_wav` checks the headers were finished.
    let mic = read_wav(&dir.join("mic.wav"));
    let system = read_wav(&dir.join("system.wav"));
    assert_eq!(mic.len(), system.len());
    let seconds = mic.len() as f64 / 16_000.0;
    assert!(seconds >= 0.7, "only {seconds} s recorded");
    assert!(seconds < 60.0, "{seconds} s recorded");
    assert!(peak(&mic[..1_600]) > 7_000);
    assert_eq!(peak(&system[..8_000]), 0);
    assert!(peak(&system[8_000..]) > 7_000);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unwritable_folder_fails_before_recording() {
    let dir = temp_dir("unwritable");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a-file");
    std::fs::write(&file, b"in the way").unwrap();
    let out = file.join("sub");
    let output = run(&["--out", out.to_str().unwrap(), "--fake", "--seconds", "1"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error: WAV error: could not create folder"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
