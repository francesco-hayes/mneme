//! Live microphone tests. They open the real default input device, so they
//! are ignored by default and listed under "Manual checks" in
//! `issues/002-mic-capture.md`.
//!
//! Run from a terminal that has (or may be given) microphone permission:
//!
//! ```sh
//! cargo test -p mneme-audio --test live_mic -- --ignored --nocapture live_mic_records_to_wav
//! cargo test -p mneme-audio --test live_mic -- --ignored --nocapture live_mic_follows_device_changes
//! ```
//!
//! macOS attributes the microphone request to the terminal app, and shows
//! its prompt the first time. Each test prints the path of the WAV it wrote;
//! play it back to check that it sounds clean and runs at the right speed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mneme_audio::{
    AudioSource, FRAME_MS, MicSource, PermissionStatus, WavWriter, frame_channel,
    microphone_permission_status, request_microphone_permission,
};
use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker, format_timestamp};

/// What a recording run produced.
struct Recording {
    path: PathBuf,
    frames: usize,
    /// Samples of real audio, not counting silence written into gaps.
    samples: u64,
    /// Meeting time of the first frame.
    first_ms: u64,
    /// Meeting time at which the last frame ended.
    end_ms: u64,
    /// `(from_ms, to_ms)` of every hole in the timestamps.
    gaps: Vec<(u64, u64)>,
    peak: f32,
    errors: Vec<String>,
}

fn seconds_from_env(default: u64) -> u64 {
    std::env::var("MNEME_LIVE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn ensure_permission() {
    let status = match microphone_permission_status() {
        PermissionStatus::NotDetermined => {
            println!("asking for microphone permission; answer the system prompt");
            request_microphone_permission()
        }
        status => status,
    };
    assert_eq!(
        status,
        PermissionStatus::Granted,
        "microphone permission is {status:?}: allow the terminal app under \
         System Settings > Privacy & Security > Microphone and run again"
    );
}

/// Records the microphone for `seconds` into `file_name` under the target
/// temp directory. Gaps in the timestamps are filled with silence so the file
/// plays back in meeting time.
fn record(seconds: u64, file_name: &str) -> Recording {
    ensure_permission();
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(file_name);
    let mut wav = WavWriter::create(&path).expect("create WAV");

    let mut mic = MicSource::new();
    let (tx, rx) = frame_channel();
    let start = Instant::now();
    mic.start(start, tx).expect("start microphone");
    assert!(mic.is_running());
    println!("recording for {seconds} s: speak now");

    let mut recording = Recording {
        path: path.clone(),
        frames: 0,
        samples: 0,
        first_ms: 0,
        end_ms: 0,
        gaps: Vec::new(),
        peak: 0.0,
        errors: Vec::new(),
    };
    let mut take = |frame: AudioFrame, wav: &mut WavWriter| {
        assert_eq!(frame.speaker, Speaker::Me);
        if recording.frames == 0 {
            recording.first_ms = frame.start_ms;
            recording.end_ms = frame.start_ms;
        }
        assert!(
            frame.start_ms >= recording.end_ms,
            "frame at {} ms starts before the previous one ended at {} ms",
            frame.start_ms,
            recording.end_ms
        );
        if frame.start_ms > recording.end_ms {
            recording.gaps.push((recording.end_ms, frame.start_ms));
            let silence = (frame.start_ms - recording.end_ms) * u64::from(SAMPLE_RATE) / 1000;
            wav.write_silence(silence).expect("write silence");
        }
        wav.write(&frame.samples).expect("write samples");
        recording.frames += 1;
        recording.samples += frame.samples.len() as u64;
        recording.end_ms =
            frame.start_ms + (frame.samples.len() as u64 * 1000).div_ceil(u64::from(SAMPLE_RATE));
        recording.peak = frame
            .samples
            .iter()
            .fold(recording.peak, |peak, s| peak.max(s.abs()));
    };

    let deadline = start + Duration::from_secs(seconds);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if let Ok(frame) = rx.recv_timeout(left) {
            take(frame, &mut wav);
        }
        for error in mic.take_errors() {
            println!("source error: {error}");
            recording.errors.push(error.to_string());
        }
    }
    mic.stop();
    assert!(!mic.is_running());
    // stop() dropped the sender, so this drains the tail and ends.
    for frame in rx {
        take(frame, &mut wav);
    }
    recording
        .errors
        .extend(mic.take_errors().iter().map(|e| e.to_string()));
    wav.finalize().expect("finalize WAV");

    println!(
        "{} frames, {:.2} s of audio from {} to {}, peak level {:.3}",
        recording.frames,
        recording.samples as f64 / f64::from(SAMPLE_RATE),
        format_timestamp(recording.first_ms),
        format_timestamp(recording.end_ms),
        recording.peak
    );
    for (from, to) in &recording.gaps {
        println!("gap of {} ms at {} ms", to - from, from);
    }
    println!("wrote {}", recording.path.display());
    recording
}

/// Manual check 1: speak during the recording, then play the WAV back.
#[test]
#[ignore = "opens the real microphone; needs permission and a person"]
fn live_mic_records_to_wav() {
    let seconds = seconds_from_env(5);
    let recording = record(seconds, "live-mic.wav");

    assert!(recording.frames > 0, "no frames arrived");
    assert!(recording.errors.is_empty(), "{:?}", recording.errors);
    assert!(recording.gaps.is_empty(), "{:?}", recording.gaps);
    // The stream comes up within a second of the start instant and runs to
    // the end: the clock is the meeting clock, not a clock of its own.
    assert!(
        recording.first_ms < 1000,
        "first frame at {} ms",
        recording.first_ms
    );
    let expected_end = seconds * 1000;
    assert!(
        recording.end_ms.abs_diff(expected_end) < 300,
        "audio ended at {} ms, expected about {expected_end} ms",
        recording.end_ms
    );
    // Sample count matches elapsed time: right speed, nothing dropped.
    let covered_ms = recording.end_ms - recording.first_ms;
    let audio_ms = recording.samples * 1000 / u64::from(SAMPLE_RATE);
    assert!(covered_ms.abs_diff(audio_ms) <= FRAME_MS);
    assert!(
        recording.peak > 0.001,
        "the recording is silent (peak {}): is the microphone muted or permission missing?",
        recording.peak
    );
}

/// Manual check 2: while this runs, switch the input device (plug in a
/// headset, connect AirPods, or pick another input in System Settings >
/// Sound). Capture must continue; the WAV has a short silence at the switch.
#[test]
#[ignore = "opens the real microphone; needs permission and a person switching devices"]
fn live_mic_follows_device_changes() {
    let seconds = seconds_from_env(30);
    println!("switch the input device at least once during the next {seconds} s");
    let recording = record(seconds, "live-mic-device-change.wav");

    assert!(recording.frames > 0, "no frames arrived");
    let expected_end = seconds * 1000;
    assert!(
        recording.end_ms.abs_diff(expected_end) < 1500,
        "audio ended at {} ms, expected about {expected_end} ms: capture did not survive",
        recording.end_ms
    );
    let lost_ms: u64 = recording.gaps.iter().map(|(from, to)| to - from).sum();
    println!(
        "{} gap(s), {lost_ms} ms without audio, {} error(s) reported",
        recording.gaps.len(),
        recording.errors.len()
    );
    assert!(
        lost_ms < 5000,
        "{lost_ms} ms of audio missing: re-opening took too long"
    );
}
