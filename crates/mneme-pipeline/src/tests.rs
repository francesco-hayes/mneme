//! Tests that need the crate's private hooks: a thread of the session
//! itself dying. Everything else is tested through the public API in
//! `tests/`.

#[path = "../tests/common/mod.rs"]
mod common;

use std::sync::Arc;
use std::time::Duration;

use mneme_core::Speaker;
use mneme_transcribe::FakeTranscriber;

use crate::shared::Hooks;
use crate::{DropReason, EndReason, Session, SessionState, Transcriber};

use common::{RecordingSink, Tape, bursts, config, sources, wait_for_status, watchdog};

#[test]
fn the_handle_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Session>();
}

#[test]
fn a_panic_on_the_capture_thread_ends_the_session_with_what_it_had() {
    watchdog(|| {
        // Two chunks are cut before the frame that kills the capture
        // thread (frame 200 is at 4 s); Them never gets a word in.
        let me = bursts(&[500, 2_000, 5_000], 300, 60_000).source(Speaker::Me);
        let fake = Arc::new(FakeTranscriber::new());
        let transcriber: Arc<dyn Transcriber> = fake.clone();
        let sink = RecordingSink::new();
        let session = Session::start_with_hooks(
            sources([me]),
            transcriber,
            Box::new(sink.for_session()),
            config(),
            Hooks {
                capture_panics_at_frame: Some(200),
                ..Hooks::default()
            },
        )
        .unwrap();

        // Nobody calls stop: the session notices by itself, and a caller
        // that does call stop is not left waiting.
        let done = wait_for_status(&session, "the session to end", |s| {
            s.state == SessionState::Done
        });
        assert_eq!(done.end, Some(EndReason::CaptureFailed));
        let summary = session.stop();

        assert_eq!(summary.end, EndReason::CaptureFailed);
        assert_eq!(summary.internal_errors.len(), 1, "{summary:?}");
        assert!(
            summary.internal_errors[0].contains("the capture thread panicked: test hook"),
            "{summary:?}"
        );
        // What was queued was still transcribed, and the sink finalized.
        assert_eq!(
            sink.lines_of(Speaker::Me),
            [(300, "chunk 1".to_owned()), (1_800, "chunk 2".to_owned())]
        );
        assert_eq!(summary.lines, 2);
        assert!(summary.finalized);
        assert!(sink.finalized_once_at_the_end());
        assert!(!summary.transcript_is_complete());
        assert_eq!(Arc::strong_count(&fake), 1);
    });
}

#[test]
fn a_dead_transcription_thread_is_reported_and_stop_still_returns() {
    watchdog(|| {
        // The thread dies as it takes the second chunk. The meeting goes
        // on; Them keeps the session recording.
        let me = bursts(&[500, 2_000, 3_500, 5_000], 300, 7_000).source(Speaker::Me);
        let them = Tape::new()
            .quiet(600)
            .talking(60_000)
            .live_source(Speaker::Them);
        let fake = Arc::new(FakeTranscriber::new());
        let transcriber: Arc<dyn Transcriber> = fake.clone();
        let sink = RecordingSink::new();
        let session = Session::start_with_hooks(
            sources([me, them]),
            transcriber,
            Box::new(sink.for_session()),
            config(),
            Hooks {
                worker_panics_at_chunk: Some(2),
                ..Hooks::default()
            },
        )
        .unwrap();

        // The status says so while the meeting is still running, and the
        // chunks nobody will transcribe are counted, not hoarded.
        let broken = wait_for_status(&session, "the dead thread to be noticed", |s| {
            s.fatal.is_some() && s.chunks.cut == 4
        });
        assert_eq!(broken.state, SessionState::Recording);
        assert_eq!(broken.chunks.dropped, 3);
        assert_eq!(broken.untranscribed(), Duration::ZERO);
        assert_eq!(broken.lines, 1);
        // The engine was let go of when the thread died.
        assert_eq!(Arc::strong_count(&fake), 1);
        wait_for_status(&session, "a second of Them", |s| {
            s.source(Speaker::Them).unwrap().end_ms.unwrap_or(0) >= 1_000
        });

        let summary = session.stop();

        assert_eq!(summary.end, EndReason::StopRequested);
        assert_eq!(summary.internal_errors.len(), 1, "{summary:?}");
        assert!(
            summary.internal_errors[0].contains("the transcription thread panicked: test hook"),
            "{summary:?}"
        );
        assert_eq!(sink.lines_of(Speaker::Me), [(300, "chunk 1".to_owned())]);
        assert_eq!(summary.lines, 1);
        // The chunk it died with, the two that were waiting, and Them's
        // words flushed at stop: all listed.
        let lost: Vec<(Speaker, u64, DropReason)> = summary
            .dropped
            .iter()
            .map(|chunk| (chunk.speaker, chunk.start_ms, chunk.reason))
            .collect();
        assert_eq!(
            lost,
            [
                (Speaker::Me, 1_800, DropReason::WorkerDied),
                (Speaker::Me, 3_300, DropReason::WorkerDied),
                (Speaker::Me, 4_800, DropReason::WorkerDied),
                (Speaker::Them, 400, DropReason::WorkerDied),
            ]
        );
        assert_eq!(summary.chunks.cut, 5);
        assert_eq!(summary.chunks.transcribed, 1);
        assert_eq!(summary.chunks.dropped, 4);
        // The sink went down with the thread: it was never finalized.
        assert!(!summary.finalized);
        assert_eq!(summary.finalize_error, None);
        assert_eq!(sink.finalize_count(), 0);
        assert!(!summary.transcript_is_complete());
    });
}
