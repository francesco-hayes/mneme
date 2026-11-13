//! Live system audio tests. They create a real Core Audio tap, which makes
//! macOS ask for the System Audio Recording permission, so they are ignored
//! by default and listed under "Manual checks" in
//! `issues/003-system-audio-taps.md`. That file's Result section has the
//! full instructions; in short:
//!
//! ```sh
//! # In a bundle of its own that carries NSAudioCaptureUsageDescription:
//! crates/mneme-audio/scripts/live-system-test.sh live_system_audio_records_to_wav
//!
//! # Or directly, attributed to the terminal app:
//! cargo test -p mneme-audio --test live_system -- --ignored --nocapture --test-threads=1 \
//!     live_system_audio_records_to_wav
//! ```
//!
//! Run one test at a time (`--test-threads=1`, or name the test): they all
//! record the same system audio.
//!
//! Environment:
//!
//! - `MNEME_LIVE_SECONDS=N` changes how long a test records.
//! - `MNEME_LIVE_OUT_DIR=<dir>` is where the WAV files go; the default is the
//!   target temp directory. The script sets it to a folder under `$TMPDIR`,
//!   because an app bundle has no access to `~/Documents` and the like.
//! - `MNEME_SKIP_TAP_ACCESS_CHECK=1` starts capture without the access check
//!   (see `SystemAudioSource::without_access_check`), to tell a failing check
//!   from a missing permission.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mneme_audio::{
    AudioSource, FRAME_MS, SystemAudioSource, WavWriter, frame_channel,
    system_audio_permission_status,
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
    /// Root mean square of everything recorded.
    rms: f32,
    errors: Vec<String>,
}

fn seconds_from_env(default: u64) -> u64 {
    std::env::var("MNEME_LIVE_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn source() -> SystemAudioSource {
    let source = SystemAudioSource::new();
    if std::env::var_os("MNEME_SKIP_TAP_ACCESS_CHECK").is_some() {
        println!("MNEME_SKIP_TAP_ACCESS_CHECK is set: starting without the access check");
        source.without_access_check()
    } else {
        source
    }
}

/// Records system audio for `seconds` into `file_name` in the output
/// directory. Gaps in the timestamps are filled with silence so the file
/// plays back in meeting time.
fn record(seconds: u64, file_name: &str) -> Recording {
    println!(
        "system audio permission before starting: {:?} (None = macOS does not say)",
        system_audio_permission_status()
    );
    let directory = std::env::var_os("MNEME_LIVE_OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    let path = directory.join(file_name);
    let mut wav = WavWriter::create(&path).expect("create WAV");

    let mut system = source();
    let (tx, rx) = frame_channel();
    let start = Instant::now();
    // Blocks while macOS shows its permission prompt.
    system.start(start, tx).unwrap_or_else(|error| {
        panic!(
            "could not start system audio capture: {error}\n\
             If this is a permission error, allow the app that runs this test under System \
             Settings > Privacy & Security > Screen & System Audio Recording > System Audio \
             Recording Only, then run the test again."
        )
    });
    assert!(system.is_running());
    println!(
        "started after {:.1} s; recording for {seconds} s",
        start.elapsed().as_secs_f32()
    );
    // The prompt may have kept `start` waiting; record for the full time
    // from here.
    let deadline = Instant::now() + Duration::from_secs(seconds);

    let mut recording = Recording {
        path: path.clone(),
        frames: 0,
        samples: 0,
        first_ms: 0,
        end_ms: 0,
        gaps: Vec::new(),
        peak: 0.0,
        rms: 0.0,
        errors: Vec::new(),
    };
    let mut sum_of_squares = 0.0f64;
    let mut take = |frame: AudioFrame, wav: &mut WavWriter| {
        assert_eq!(frame.speaker, Speaker::Them);
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
        for sample in &frame.samples {
            recording.peak = recording.peak.max(sample.abs());
            sum_of_squares += f64::from(*sample).powi(2);
        }
    };

    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        if let Ok(frame) = rx.recv_timeout(left.min(Duration::from_millis(200))) {
            take(frame, &mut wav);
        }
        for error in system.take_errors() {
            println!("source error: {error}");
            recording.errors.push(error.to_string());
        }
    }
    system.stop();
    assert!(!system.is_running());
    // stop() dropped the sender, so this drains the tail and ends.
    for frame in rx {
        take(frame, &mut wav);
    }
    for error in system.take_errors() {
        println!("source error: {error}");
        recording.errors.push(error.to_string());
    }
    wav.finalize().expect("finalize WAV");
    recording.rms = (sum_of_squares / recording.samples.max(1) as f64).sqrt() as f32;

    println!(
        "{} frames, {:.2} s of audio from {} to {}, peak {:.4}, RMS {:.4}",
        recording.frames,
        recording.samples as f64 / f64::from(SAMPLE_RATE),
        format_timestamp(recording.first_ms),
        format_timestamp(recording.end_ms),
        recording.peak,
        recording.rms
    );
    for (from, to) in &recording.gaps {
        println!("gap of {} ms at {} ms", to - from, from);
    }
    println!("wrote {}", recording.path.display());
    recording
}

/// Asserts that audio arrived the whole time: no errors, no holes, and as
/// many samples as time passed.
fn assert_continuous(recording: &Recording) {
    assert!(recording.frames > 0, "no frames arrived");
    assert!(recording.errors.is_empty(), "{:?}", recording.errors);
    assert!(recording.gaps.is_empty(), "{:?}", recording.gaps);
    let covered_ms = recording.end_ms - recording.first_ms;
    let audio_ms = recording.samples * 1000 / u64::from(SAMPLE_RATE);
    assert!(
        covered_ms.abs_diff(audio_ms) <= FRAME_MS,
        "{audio_ms} ms of audio over {covered_ms} ms of meeting time"
    );
}

/// Manual check 1: play audio from another app (music, a video, a call)
/// while this runs, then play the WAV back. It must sound clean and run at
/// the right speed, and the sound must have kept playing from the speakers.
#[test]
#[ignore = "creates a real Core Audio tap; needs permission and audio playing"]
fn live_system_audio_records_to_wav() {
    let seconds = seconds_from_env(10);
    println!("play audio from another app for the next {seconds} s");
    let recording = record(seconds, "live-system.wav");

    assert_continuous(&recording);
    assert!(
        recording.peak > 0.001,
        "the recording is silent (peak {}). Either nothing was playing, or macOS is feeding the \
         tap silence because the System Audio Recording permission is missing.",
        recording.peak
    );
}

/// Checks the assumption the capture loop rests on: with nothing playing,
/// the tap still delivers (silent) buffers, so the clock runs on and the
/// stream is not taken for dead. Make sure nothing plays while this runs.
#[test]
#[ignore = "creates a real Core Audio tap; needs permission and a quiet system"]
fn live_system_audio_runs_through_silence() {
    let seconds = seconds_from_env(8);
    println!("keep the Mac silent for the next {seconds} s");
    let recording = record(seconds, "live-system-silence.wav");

    assert_continuous(&recording);
    println!(
        "peak during silence: {} (anything above 0 is some app's sound)",
        recording.peak
    );
}

/// Manual check 2: this process plays a tone while it captures. With nothing
/// else playing, the recording must be silent: the tap leaves this process
/// out. You should hear the tone (quietly) from the speakers.
#[test]
#[ignore = "creates a real Core Audio tap and plays a tone; needs permission and a quiet system"]
fn live_system_audio_excludes_this_process() {
    let seconds = seconds_from_env(5);

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("a default output device");
    let supported = device.default_output_config().expect("an output format");
    assert_eq!(
        supported.sample_format(),
        cpal::SampleFormat::F32,
        "this test only plays through float output devices"
    );
    let config = supported.config();
    let channels = usize::from(config.channels);
    let step = std::f32::consts::TAU * 440.0 / config.sample_rate as f32;
    let mut phase = 0.0f32;
    let tone = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    let sample = 0.1 * phase.sin();
                    phase = (phase + step) % std::f32::consts::TAU;
                    frame.fill(sample);
                }
            },
            |error| println!("tone playback error: {error}"),
            None,
        )
        .expect("build the tone stream");
    tone.play().expect("play the tone");
    println!("playing a quiet 440 Hz tone from this process; keep everything else silent");

    let recording = record(seconds, "live-system-own-sound.wav");
    drop(tone);

    assert_continuous(&recording);
    // The tone has an RMS of about 0.07. Other apps' sounds would show up
    // too, so a failure here is only meaningful on a quiet system.
    assert!(
        recording.rms < 0.005,
        "this process's own tone was captured (RMS {}, peak {})",
        recording.rms,
        recording.peak
    );
}

/// Manual check 3: while this runs and audio plays, switch the output device
/// (plug in headphones, connect AirPods, or pick another output in System
/// Settings > Sound). Capture must continue; the WAV has a short silence at
/// each switch.
#[test]
#[ignore = "creates a real Core Audio tap; needs permission and a person switching devices"]
fn live_system_audio_follows_device_changes() {
    let seconds = seconds_from_env(30);
    println!("play audio and switch the output device at least once during the next {seconds} s");
    let recording = record(seconds, "live-system-device-change.wav");

    assert!(recording.frames > 0, "no frames arrived");
    let lost_ms: u64 = recording.gaps.iter().map(|(from, to)| to - from).sum();
    println!(
        "{} gap(s), {lost_ms} ms without audio, {} error(s) reported",
        recording.gaps.len(),
        recording.errors.len()
    );
    assert!(
        lost_ms < 5000,
        "{lost_ms} ms of audio missing: rebuilding the tap took too long"
    );
    // Audio was still arriving at the end: capture survived the switch.
    let covered_ms = recording.end_ms - recording.first_ms;
    assert!(
        covered_ms + 1500 > seconds * 1000,
        "audio covers only {covered_ms} ms of {seconds} s: capture did not survive"
    );
    assert!(
        recording.peak > 0.001,
        "the recording is silent (peak {})",
        recording.peak
    );
}
