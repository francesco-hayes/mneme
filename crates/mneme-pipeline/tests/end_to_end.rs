//! The whole pipeline with nothing faked but the sources: the speech
//! fixture played on both channels, the real Whisper engine, the real store.
//!
//! Needs a Whisper model file, which is never committed. The test looks for
//!
//! 1. the path in the `MNEME_TEST_MODEL` environment variable, else
//! 2. `crates/mneme-transcribe/models/ggml-tiny.en.bin`, which
//!    `crates/mneme-transcribe/scripts/fetch-test-model.sh` downloads.
//!
//! When neither is there it prints a `SKIPPED` line and passes.

mod common;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mneme_audio::{AudioSource, FakeSource, SourcePhase};
use mneme_core::Speaker;
use mneme_pipeline::{EndReason, Session, SessionConfig, Transcriber, keep_audio_enabled};
use mneme_store::{MeetingStatus, NewMeeting, StartTime, Store, Transcript};
use mneme_transcribe::{Language, WhisperEngine};

use common::watchdog;

const DEFAULT_MODEL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../mneme-transcribe/models/ggml-tiny.en.bin"
);
/// 5.75 s of synthetic speech: "Hello, this is a test of the meeting
/// transcription. The quick brown fox jumps over the lazy dog."
const SPEECH_WAV: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../mneme-transcribe/tests/data/speech.wav"
);
/// Words the tiny English model must find in the fixture.
const EXPECTED_PHRASES: [&str; 4] = ["hello", "meeting", "quick brown fox", "lazy dog"];

/// Them says the same thing, starting this far into the meeting.
const THEM_STARTS_AT_MS: u64 = 3_000;

fn model_path() -> Option<PathBuf> {
    let path = match std::env::var_os("MNEME_TEST_MODEL") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(DEFAULT_MODEL),
    };
    path.is_file().then_some(path)
}

#[test]
fn speech_on_both_channels_becomes_an_ordered_transcript_in_a_meeting_folder() {
    let Some(model) = model_path() else {
        // Written straight to stderr so it shows without `--nocapture`.
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED speech_on_both_channels_becomes_an_ordered_transcript_in_a_meeting_folder: \
             no Whisper model file. Run crates/mneme-transcribe/scripts/fetch-test-model.sh, \
             or set MNEME_TEST_MODEL to a ggml-tiny.en.bin file."
        );
        return;
    };
    watchdog(move || {
        WhisperEngine::silence_log();
        // What the app does at start: load the model, create the meeting,
        // take its transcript writer, build the sources.
        let engine = Arc::new(WhisperEngine::load(&model).expect("the test model should load"));
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("meetings")).unwrap();
        let meeting = store
            .create_meeting(NewMeeting {
                started_at: StartTime::parse_rfc3339("2026-11-12T10:00:00+01:00").unwrap(),
                model: Some("tiny.en".to_owned()),
                language: Some("en".to_owned()),
            })
            .unwrap();
        let writer = store.transcript_writer(&meeting.id).unwrap();
        let sources: Vec<Box<dyn AudioSource>> = vec![
            Box::new(FakeSource::from_wav(Speaker::Me, SPEECH_WAV).unwrap()),
            Box::new(
                FakeSource::from_wav(Speaker::Them, SPEECH_WAV)
                    .unwrap()
                    .starting_at(Duration::from_millis(THEM_STARTS_AT_MS)),
            ),
        ];
        let transcriber: Arc<dyn Transcriber> = engine.clone();
        let mut config = SessionConfig::new(Language::english());
        config.audio_dir = Some(store.meeting(&meeting.id).unwrap().folder);
        config.recorder.head_start = Duration::ZERO;

        let session = Session::start(sources, transcriber, writer, config).unwrap();
        // Both files run out and the session finishes the transcript by
        // itself; what the app's stop command would do next follows.
        let summary = session.wait();
        let played_all = summary
            .sources
            .iter()
            .all(|source| source.samples >= 5 * 16_000);

        let finished = store
            .finish_meeting(&meeting.id, summary.duration.as_secs())
            .unwrap();
        drop(session);
        // The session holds nothing of the engine any more. Drop it before
        // the test process can exit: a model still loaded at exit aborts in
        // whisper.cpp's Metal backend.
        let engine = Arc::try_unwrap(engine)
            .unwrap_or_else(|_| panic!("the session still holds the engine after stop()"));
        drop(engine);

        assert!(played_all, "the fixture was not played whole: {summary:?}");
        assert!(summary.transcript_is_complete(), "{summary:?}");
        assert_eq!(summary.end, EndReason::SourcesEnded);
        assert!(summary.chunks.cut >= 2, "{:?}", summary.chunks);
        assert_eq!(finished.meeting.status, MeetingStatus::Complete);

        // The file itself, as it is on disk.
        let folder = store.meeting(&meeting.id).unwrap().folder;
        let text = std::fs::read_to_string(folder.join("transcript.md")).unwrap();
        // Shown with `--nocapture`.
        println!("transcript.md:\n{text}");
        let transcript = Transcript::parse(text.as_bytes());
        assert_eq!(transcript.skipped, 0, "{text}");
        assert_eq!(transcript.lines.len() as u64, summary.lines, "{text}");
        assert_eq!(
            store.transcript_line_count(&meeting.id).unwrap() as u64,
            summary.lines
        );

        // In time order, although the two speakers' chunks were transcribed
        // in whatever order they were cut.
        let times: Vec<u64> = transcript.lines.iter().map(|l| l.timestamp_ms).collect();
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "{text}");

        // Both speakers said the whole fixture, each at their own time.
        for speaker in [Speaker::Me, Speaker::Them] {
            let lines: Vec<_> = transcript
                .lines
                .iter()
                .filter(|line| line.speaker == speaker)
                .collect();
            assert!(!lines.is_empty(), "no lines for {speaker}:\n{text}");
            let said = lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            for phrase in EXPECTED_PHRASES {
                assert!(
                    said.contains(phrase),
                    "{speaker} is missing {phrase:?}:\n{text}"
                );
            }
            let offset = match speaker {
                Speaker::Me => 0,
                Speaker::Them => THEM_STARTS_AT_MS,
            };
            let first = lines.first().unwrap().timestamp_ms;
            let last = lines.last().unwrap().timestamp_ms;
            assert!(
                (offset..offset + 1_000).contains(&first),
                "{speaker} starts speaking at {offset} ms, first line at {first} ms:\n{text}"
            );
            assert!(
                last < offset + 6_000,
                "{speaker}'s last line at {last} ms:\n{text}"
            );
        }
        // Them began three seconds in, so the file starts with Me.
        assert_eq!(transcript.lines[0].speaker, Speaker::Me);

        // The sources report what happened to them.
        for source in &summary.sources {
            assert_eq!(source.phase, SourcePhase::Ended, "{source:?}");
            assert!(source.errors.is_empty());
        }

        // Audio retention: the meeting folder holds text files only, unless
        // this very run was started with MNEME_KEEP_AUDIO=1.
        let mut names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        if keep_audio_enabled() {
            assert!(names.contains(&"mic.wav".to_owned()), "{names:?}");
            assert!(names.contains(&"system.wav".to_owned()), "{names:?}");
        } else {
            assert_eq!(names, ["notes-timeline.jsonl", "notes.md", "transcript.md"]);
            assert_eq!(summary.audio, None);
        }
    });
}
