//! What the session does when a part of it fails: a source that does not
//! start or dies, a transcriber that errors or panics, a sink that refuses a
//! line, a backlog past its limit, a drain that is abandoned.

mod common;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mneme_audio::{AudioError, SourcePhase};
use mneme_core::Speaker;
use mneme_pipeline::{
    ChunkCounts, DropReason, DroppedChunk, EndReason, Session, SessionConfig, SessionState,
    SinkError, SinkFailure, TranscriptionFailure,
};
use mneme_transcribe::FakeTranscriber;

use common::{
    Gate, OpenOnDrop, RecordingSink, SinkEvent, Tape, TestTranscriber, bursts, config,
    engine_error, sources, wait_for, wait_for_status, watchdog, whole,
};

/// A burst of 200 ms every second from 0.3 s on: a chunk of 600 ms starting
/// at 100 ms, 1100 ms, and so on.
fn steady_talker(total_ms: u64) -> Tape {
    let starts: Vec<u64> = (0..total_ms / 1_000).map(|n| 300 + n * 1_000).collect();
    bursts(&starts, 200, total_ms)
}

#[test]
fn a_source_that_dies_mid_session_keeps_its_last_words_and_the_other_carries_on() {
    watchdog(|| {
        // Them is 300 ms into a sentence when its source dies at 1.3 s.
        let died = AudioError::Stream("the tap stopped delivering".to_owned());
        let them = bursts(&[1_000], 600, 5_000)
            .source(Speaker::Them)
            .failing_after(Duration::from_millis(1_300), died.clone());
        let me = steady_talker(60_000).live_source(Speaker::Me);
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([me, them]),
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();

        // The failure shows in the status while the meeting goes on, and
        // Them's unfinished sentence is transcribed now, not at stop.
        let after = wait_for_status(&session, "Them's last words", |s| {
            s.source(Speaker::Them).unwrap().phase == SourcePhase::Ended
                && sink.lines_of(Speaker::Them).len() == 1
        });
        assert_eq!(after.state, SessionState::Recording);
        assert_eq!(after.end, None);
        assert_eq!(
            after.source(Speaker::Them).unwrap().errors,
            std::slice::from_ref(&died)
        );
        assert_eq!(
            after.source(Speaker::Me).unwrap().phase,
            SourcePhase::Running
        );
        // 200 ms of padding and the 300 ms that were captured.
        assert_eq!(sink.lines_of(Speaker::Them), [(800, "500 ms".to_owned())]);

        // Me's lines keep arriving after the failure.
        let me_before = sink.lines_of(Speaker::Me).len();
        let later = wait_for_status(&session, "two more lines from Me", |_| {
            sink.lines_of(Speaker::Me).len() >= me_before + 2
        });
        assert_eq!(later.state, SessionState::Recording);

        let summary = session.stop();
        assert_eq!(summary.end, EndReason::StopRequested);
        let them = summary.source(Speaker::Them).unwrap();
        assert_eq!(them.phase, SourcePhase::Ended);
        assert_eq!(them.errors, std::slice::from_ref(&died));
        assert_eq!(summary.source_errors(), [(Speaker::Them, died)]);
        assert_eq!(summary.failed_sources().len(), 1);
        assert_eq!(summary.failed_sources()[0].speaker, Speaker::Them);
        assert_eq!(
            summary.source(Speaker::Me).unwrap().phase,
            SourcePhase::Stopped
        );

        // Nothing of Them was transcribed twice (once at its end, not again
        // at stop), and Me's lines are where its bursts were.
        assert_eq!(sink.lines_of(Speaker::Them).len(), 1);
        let me_lines = sink.lines_of(Speaker::Me);
        assert!(me_lines.len() >= me_before + 2);
        for (n, (start_ms, text)) in me_lines.iter().enumerate() {
            assert_eq!(*start_ms, 100 + n as u64 * 1_000);
            // The last one may have been cut short by the stop.
            if n + 1 < me_lines.len() {
                assert_eq!(text, "600 ms");
            }
        }
        assert!(summary.transcript_is_complete(), "{summary:?}");
        assert!(sink.finalized_once_at_the_end());
    });
}

#[test]
fn a_source_that_ends_in_good_health_is_flushed_while_the_other_continues() {
    watchdog(|| {
        // Me's audio runs out in the middle of its second sentence.
        let me = Tape::new().burst_at(500, 300).burst_at(1_500, 400);
        let them = steady_talker(60_000).live_source(Speaker::Them);
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([me.source(Speaker::Me), them]),
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();

        let after = wait_for_status(&session, "both of Me's lines", |_| {
            sink.lines_of(Speaker::Me).len() == 2
        });
        assert_eq!(after.state, SessionState::Recording);
        assert_eq!(after.source(Speaker::Me).unwrap().phase, SourcePhase::Ended);
        assert!(after.source(Speaker::Me).unwrap().errors.is_empty());
        assert_eq!(
            sink.lines_of(Speaker::Me),
            [(300, "700 ms".to_owned()), (1_300, "600 ms".to_owned())]
        );

        wait_for_status(&session, "lines from Them afterwards", |_| {
            sink.lines_of(Speaker::Them).len() >= 2
        });
        let summary = session.stop();
        assert_eq!(
            summary.source(Speaker::Me).unwrap().phase,
            SourcePhase::Ended
        );
        assert_eq!(
            summary.source(Speaker::Them).unwrap().phase,
            SourcePhase::Stopped
        );
        assert!(summary.source_errors().is_empty());
        assert_eq!(sink.lines_of(Speaker::Me).len(), 2);
    });
}

#[test]
fn a_source_that_fails_to_start_is_reported_and_the_other_records() {
    watchdog(|| {
        let denied = AudioError::PermissionDenied("System Audio Recording".to_owned());
        let them = bursts(&[500], 300, 2_000)
            .source(Speaker::Them)
            .failing_to_start(denied.clone());
        let me = steady_talker(60_000).live_source(Speaker::Me);
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([me, them]),
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();

        let recording = wait_for_status(&session, "two lines from Me", |s| s.lines >= 2);
        assert_eq!(recording.state, SessionState::Recording);
        let them = recording.source(Speaker::Them).unwrap();
        assert_eq!(them.phase, SourcePhase::FailedToStart);
        assert_eq!(them.errors, std::slice::from_ref(&denied));

        let summary = session.stop();
        let them = summary.source(Speaker::Them).unwrap();
        assert_eq!(them.phase, SourcePhase::FailedToStart);
        assert_eq!(them.errors.first(), Some(&denied));
        assert_eq!(them.frames, 0);
        assert_eq!(
            summary.source(Speaker::Me).unwrap().phase,
            SourcePhase::Stopped
        );
        assert!(sink.lines_of(Speaker::Them).is_empty());
        assert!(sink.lines_of(Speaker::Me).len() >= 2);
        assert!(summary.transcript_is_complete());
    });
}

#[test]
fn when_no_source_starts_the_session_ends_by_itself_with_both_errors() {
    watchdog(|| {
        let no_mic = AudioError::NoInputDevice;
        let denied = AudioError::PermissionDenied("System Audio Recording".to_owned());
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&[500], 300, 2_000)
                    .source(Speaker::Me)
                    .failing_to_start(no_mic.clone()),
                bursts(&[500], 300, 2_000)
                    .source(Speaker::Them)
                    .failing_to_start(denied.clone()),
            ]),
            Arc::new(FakeTranscriber::new()),
            sink.for_session(),
            config(),
        )
        .unwrap();

        let summary = session.wait();
        assert_eq!(summary.end, EndReason::SourcesEnded);
        assert_eq!(
            summary.source_errors(),
            [(Speaker::Me, no_mic), (Speaker::Them, denied)]
        );
        assert!(
            summary
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::FailedToStart)
        );
        assert_eq!(summary.lines, 0);
        assert_eq!(sink.events(), [SinkEvent::Finalize]);
    });
}

#[test]
fn stop_racing_with_a_dying_source_always_returns_a_whole_summary() {
    watchdog(|| {
        for round in 0..24_u64 {
            // Them dies somewhere between 40 and 86 ms; stop is called
            // somewhere around that moment.
            let dies_after = Duration::from_millis(40 + round * 2);
            let died = AudioError::Stream("gone".to_owned());
            let them = Tape::new()
                .talking(10_000)
                .live_source(Speaker::Them)
                .failing_after(dies_after, died.clone());
            let me = Tape::new().talking(10_000).live_source(Speaker::Me);
            let sink = RecordingSink::new();
            let fake = Arc::new(FakeTranscriber::new());
            let session = Session::start(
                sources([me, them]),
                fake.clone(),
                sink.for_session(),
                config(),
            )
            .unwrap();

            let stop_at = dies_after + Duration::from_millis(round % 5 * 10);
            thread::sleep(stop_at.saturating_sub(Duration::from_millis(20)));
            let summary = session.stop();

            assert_eq!(summary.end, EndReason::StopRequested);
            assert!(sink.finalized_once_at_the_end(), "{:?}", sink.events());
            assert_eq!(summary.chunks.cut, summary.chunks.transcribed);
            assert_eq!(summary.lines, summary.chunks.cut);
            // At most one chunk per speaker, each transcribed once.
            assert!(sink.lines_of(Speaker::Them).len() <= 1);
            assert!(sink.lines_of(Speaker::Me).len() <= 1);
            let them = summary.source(Speaker::Them).unwrap();
            match them.phase {
                SourcePhase::Ended => assert_eq!(them.errors, [died]),
                // Stopped a moment before it would have died, or in the
                // moment between reporting its error and ending.
                SourcePhase::Stopped => assert!(them.errors.is_empty() || them.errors == [died]),
                other => panic!("unexpected phase {other:?}"),
            }
            assert_eq!(Arc::strong_count(&fake), 1);
        }
    });
}

#[test]
fn a_transcriber_error_on_one_chunk_is_recorded_and_the_session_continues() {
    watchdog(|| {
        let failed = engine_error("out of memory");
        let fake = Arc::new(
            FakeTranscriber::new()
                .then_text("First.")
                .then_error(failed.clone())
                .then_text("Third."),
        );
        let sink = RecordingSink::new();
        // One speaker, so chunks are served in the order they were said.
        let session = Session::start(
            sources([
                Tape::new()
                    .burst_at(500, 300)
                    .burst_at(2_000, 400)
                    .burst_at(4_000, 500)
                    .until(6_000)
                    .source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();

        // Still recording after the failure, and the status says so.
        let status = wait_for_status(&session, "the third chunk", |s| s.lines == 2);
        assert_eq!(status.state, SessionState::Recording);
        assert_eq!(status.chunks.failed, 1);
        let last_error = status.last_error.expect("the failure is in the status");
        assert!(last_error.contains("out of memory"), "{last_error}");
        assert!(last_error.contains("1800 ms"), "{last_error}");

        let summary = session.stop();
        assert_eq!(fake.call_count(), 3, "the failed chunk was not tried again");
        assert_eq!(
            summary.transcription_errors,
            [TranscriptionFailure {
                speaker: Speaker::Me,
                start_ms: 1_800,
                duration_ms: 800,
                error: failed,
            }]
        );
        assert_eq!(
            summary.chunks,
            ChunkCounts {
                cut: 3,
                transcribed: 2,
                failed: 1,
                dropped: 0
            }
        );
        assert_eq!(
            sink.lines_of(Speaker::Me),
            [(300, "First.".to_owned()), (3_800, "Third.".to_owned())]
        );
        assert_eq!(summary.lines, 2);
        assert!(!summary.transcript_is_complete());
        assert!(summary.finalized);
        assert!(sink.finalized_once_at_the_end());
    });
}

#[test]
fn a_transcriber_that_fails_every_time_still_lets_the_session_stop() {
    watchdog(|| {
        let transcriber = Arc::new(TestTranscriber::new(|_, _| Err(engine_error("broken"))));
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([bursts(&[500, 2_000, 3_500], 300, 5_000).source(Speaker::Me)]),
            transcriber,
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.stop();
        assert_eq!(summary.chunks.failed, summary.chunks.cut);
        assert_eq!(
            summary.transcription_errors.len() as u64,
            summary.chunks.cut
        );
        assert_eq!(summary.lines, 0);
        assert_eq!(sink.events(), [SinkEvent::Finalize]);
    });
}

#[test]
fn a_transcriber_that_panics_on_a_chunk_does_not_hang_stop_or_end_the_session() {
    watchdog(|| {
        let transcriber = Arc::new(TestTranscriber::new(|call, samples| {
            if call == 2 {
                panic!("scripted transcriber panic");
            }
            Ok(vec![whole(samples, format!("call {call}"))])
        }));
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&[500, 2_000, 3_500], 300, 5_000).source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            transcriber.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();

        let status = wait_for_status(&session, "the chunk after the panic", |s| s.lines == 2);
        assert_eq!(status.state, SessionState::Recording);
        assert_eq!(status.fatal, None);

        let summary = session.stop();
        assert_eq!(summary.transcription_errors.len(), 1);
        let failure = &summary.transcription_errors[0];
        assert_eq!((failure.speaker, failure.start_ms), (Speaker::Me, 1_800));
        assert!(
            failure
                .error
                .to_string()
                .contains("scripted transcriber panic"),
            "{}",
            failure.error
        );
        assert_eq!(
            sink.lines_of(Speaker::Me),
            [(300, "call 1".to_owned()), (3_300, "call 3".to_owned())]
        );
        assert!(
            summary.internal_errors.is_empty(),
            "the thread itself survived"
        );
        assert!(summary.finalized);
        assert_eq!(Arc::strong_count(&transcriber), 1);
    });
}

#[test]
fn a_line_the_sink_refuses_is_not_offered_again_and_is_kept_in_the_summary() {
    watchdog(|| {
        let sink = RecordingSink::new().failing_on_append(2);
        let session = Session::start(
            sources([
                bursts(&[500, 2_000, 3_500], 300, 5_000).source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            Arc::new(FakeTranscriber::with_texts(["One.", "Two.", "Three."])),
            sink.for_session(),
            config(),
        )
        .unwrap();

        let status = wait_for_status(&session, "the line after the failure", |s| s.lines == 2);
        assert_eq!(status.state, SessionState::Recording);
        assert_eq!(status.sink_errors, 1);
        assert!(
            status
                .last_error
                .unwrap()
                .contains("scripted failure on append 2")
        );

        let summary = session.stop();

        // Each line was offered exactly once: the refused one may be in the
        // sink already, so a second attempt could write it twice.
        let offered: Vec<String> = sink.offered().into_iter().map(|s| s.text).collect();
        assert_eq!(offered, ["One.", "Two.", "Three."]);
        assert_eq!(
            sink.lines_of(Speaker::Me),
            [(300, "One.".to_owned()), (3_300, "Three.".to_owned())]
        );
        assert_eq!(summary.lines, 2);

        // The text is not lost: it is handed back with the error.
        assert_eq!(
            summary.sink_errors,
            [SinkFailure {
                segment: mneme_core::Segment {
                    speaker: Speaker::Me,
                    start_ms: 1_800,
                    end_ms: 2_500,
                    text: "Two.".to_owned(),
                },
                error: SinkError::new("scripted failure on append 2"),
            }]
        );
        assert_eq!(summary.chunks.transcribed, 3);
        assert!(summary.transcription_errors.is_empty());
        assert!(!summary.transcript_is_complete());
        assert!(summary.finalized);
        assert!(sink.finalized_once_at_the_end());
    });
}

#[test]
fn a_sink_that_panics_on_a_line_is_treated_as_refusing_it() {
    watchdog(|| {
        let sink = RecordingSink::new().panicking_on_append(1);
        let session = Session::start(
            sources([bursts(&[500, 2_000], 300, 3_000).source(Speaker::Me)]),
            Arc::new(FakeTranscriber::with_texts(["One.", "Two."])),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.wait();
        assert_eq!(summary.sink_errors.len(), 1);
        assert_eq!(summary.sink_errors[0].segment.text, "One.");
        assert!(
            summary.sink_errors[0]
                .error
                .message()
                .contains("scripted sink panic"),
            "{}",
            summary.sink_errors[0].error
        );
        assert_eq!(sink.lines_of(Speaker::Me), [(1_800, "Two.".to_owned())]);
        assert!(summary.finalized);
        assert!(summary.internal_errors.is_empty());
    });
}

#[test]
fn a_finalize_that_fails_is_reported_and_not_repeated() {
    watchdog(|| {
        let sink = RecordingSink::new().failing_on_finalize();
        let session = Session::start(
            sources([bursts(&[500], 300, 2_000).source(Speaker::Me)]),
            Arc::new(FakeTranscriber::new()),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.stop();
        assert!(!summary.finalized);
        assert_eq!(
            summary.finalize_error,
            Some(SinkError::new("scripted finalize failure"))
        );
        assert_eq!(summary.lines, 1, "the line is where the sink put it");
        assert_eq!(sink.finalize_count(), 1);
        assert!(!summary.transcript_is_complete());
    });
}

#[test]
fn a_sink_that_panics_when_dropped_takes_the_thread_down_but_not_stop() {
    watchdog(|| {
        let sink = RecordingSink::new().panicking_on_drop();
        let fake = Arc::new(FakeTranscriber::new());
        let session = Session::start(
            sources([bursts(&[500], 300, 2_000).source(Speaker::Me)]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.stop();
        // The transcript was complete before the panic.
        assert_eq!(summary.lines, 1);
        assert!(summary.finalized);
        assert_eq!(summary.internal_errors.len(), 1, "{summary:?}");
        assert!(summary.internal_errors[0].contains("scripted sink panic on drop"));
        assert!(!summary.transcript_is_complete());
        assert_eq!(Arc::strong_count(&fake), 1);
    });
}

#[test]
fn past_the_backlog_limit_chunks_are_dropped_and_every_one_is_reported() {
    watchdog(|| {
        // Six chunks of 700 ms, one a second, in real time. The transcriber
        // holds the first; the limit lets one more wait.
        let starts: Vec<u64> = (0..6).map(|n| 300 + n * 1_000).collect();
        let gate = Gate::closed();
        let transcriber = Arc::new(TestTranscriber::naming_lengths().gated(&gate));
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&starts, 300, 6_400).live_source(Speaker::Me),
                Tape::new().quiet(6_400).live_source(Speaker::Them),
            ]),
            transcriber.clone(),
            sink.for_session(),
            SessionConfig {
                max_backlog: Duration::from_millis(1_000),
                ..config()
            },
        )
        .unwrap();
        let _open = OpenOnDrop(gate.clone());

        // Chunk 1 is with the transcriber, chunk 2 waits, chunk 3 does not
        // fit and neither does chunk 4.
        let full = wait_for_status(&session, "two dropped chunks", |s| s.chunks.dropped == 2);
        assert_eq!(full.state, SessionState::Recording);
        assert_eq!(full.chunks.cut, 4);
        assert_eq!(full.queued_chunks, 1);
        assert_eq!(full.backlog, Duration::from_millis(700));
        let said = full
            .last_error
            .expect("a dropped chunk is reported at once");
        assert!(said.contains("not transcribed"), "{said}");

        // With the transcriber moving again there is room: later speech is
        // transcribed as usual.
        gate.open();
        // Both sources run out at 6.4 s and the session ends by itself.
        let summary = session.wait();
        assert_eq!(summary.end, EndReason::SourcesEnded);

        assert_eq!(
            summary.dropped,
            [
                DroppedChunk {
                    speaker: Speaker::Me,
                    start_ms: 2_100,
                    duration_ms: 700,
                    reason: DropReason::BacklogFull
                },
                DroppedChunk {
                    speaker: Speaker::Me,
                    start_ms: 3_100,
                    duration_ms: 700,
                    reason: DropReason::BacklogFull
                },
            ]
        );
        let written: Vec<u64> = sink.lines_of(Speaker::Me).iter().map(|l| l.0).collect();
        assert_eq!(written, [100, 1_100, 4_100, 5_100]);
        assert_eq!(
            summary.chunks,
            ChunkCounts {
                cut: 6,
                transcribed: 4,
                failed: 0,
                dropped: 2
            }
        );
        assert!(!summary.transcript_is_complete());
        assert!(!summary.drain.abandoned);
    });
}

#[test]
fn an_abandoned_backlog_is_dropped_at_stop_and_listed_and_the_chunk_in_work_is_kept() {
    watchdog(|| {
        let starts: Vec<u64> = (0..5).map(|n| 500 + n * 1_500).collect();
        let gate = Gate::closed();
        let transcriber = Arc::new(TestTranscriber::naming_lengths().gated(&gate));
        let sink = RecordingSink::new();
        let session = Arc::new(
            Session::start(
                sources([
                    bursts(&starts, 300, 8_000).source(Speaker::Me),
                    // Mid-sentence at stop: its flushed chunk is abandoned too.
                    Tape::new().talking(60_000).live_source(Speaker::Them),
                ]),
                transcriber.clone(),
                sink.for_session(),
                config(),
            )
            .unwrap(),
        );
        let _open = OpenOnDrop(gate.clone());
        wait_for_status(&session, "five chunks, one in work", |s| {
            s.chunks.cut == 5 && s.queued_chunks == 4
        });
        wait_for_status(&session, "half a second of Them", |s| {
            s.source(Speaker::Them).unwrap().end_ms.unwrap_or(0) >= 500
        });

        // Abandoning alone changes nothing while the meeting runs.
        session.abandon_backlog();
        thread::sleep(Duration::from_millis(50));
        let still = session.status();
        assert_eq!(still.state, SessionState::Recording);
        assert_eq!(still.queued_chunks, 4);
        assert_eq!(still.chunks.dropped, 0);

        // stop() cannot interrupt the transcriber call in progress...
        let stopper = {
            let session = Arc::clone(&session);
            thread::spawn(move || session.stop())
        };
        let stopping = wait_for_status(&session, "the backlog to be dropped", |s| {
            s.chunks.dropped == 5
        });
        assert_eq!(stopping.state, SessionState::Stopping);
        assert!(stopping.drain.unwrap().abandoned);
        assert_eq!(stopping.backlog, Duration::ZERO);
        assert_eq!(stopping.transcribing, Duration::from_millis(700));
        assert!(!stopper.is_finished());

        // ...and returns as soon as that call does.
        gate.open();
        let summary = stopper.join().unwrap();

        assert_eq!(transcriber.finished(), 1, "nothing else was transcribed");
        assert_eq!(sink.lines_of(Speaker::Me), [(300, "700 ms".to_owned())]);
        assert_eq!(summary.lines, 1);
        assert_eq!(summary.dropped.len(), 5);
        assert!(
            summary
                .dropped
                .iter()
                .all(|c| c.reason == DropReason::DrainAbandoned)
        );
        let lost: Vec<(Speaker, u64)> = summary
            .dropped
            .iter()
            .map(|c| (c.speaker, c.start_ms))
            .collect();
        assert_eq!(
            lost,
            [
                (Speaker::Me, 1_800),
                (Speaker::Me, 3_300),
                (Speaker::Me, 4_800),
                (Speaker::Me, 6_300),
                (Speaker::Them, 0),
            ]
        );
        assert!(summary.drain.abandoned);
        assert!(summary.finalized, "what was written is still sorted");
        assert!(sink.finalized_once_at_the_end());
        assert!(!summary.transcript_is_complete());
        assert_eq!(Arc::strong_count(&transcriber), 1);
    });
}

#[test]
fn abandoning_during_a_drain_cuts_it_short() {
    watchdog(|| {
        const DELAY: Duration = Duration::from_millis(150);
        let starts: Vec<u64> = (0..40).map(|n| 500 + n * 1_500).collect();
        let fake = Arc::new(FakeTranscriber::new().with_delay(DELAY));
        let sink = RecordingSink::new();
        let session = Arc::new(
            Session::start(
                sources([bursts(&starts, 300, 62_000).source(Speaker::Me)]),
                fake.clone(),
                sink.for_session(),
                config(),
            )
            .unwrap(),
        );
        wait_for_status(&session, "all chunks", |s| s.chunks.cut == 40);

        // A full drain would take 40 x 150 ms = 6 s.
        let stopper = {
            let session = Arc::clone(&session);
            thread::spawn(move || session.stop())
        };
        wait_for_status(&session, "the drain to begin", |s| s.drain.is_some());
        thread::sleep(DELAY);
        let abandoned_at = Instant::now();
        session.abandon_backlog();
        let summary = stopper.join().unwrap();

        // At most the call in progress was waited for.
        assert!(
            abandoned_at.elapsed() < DELAY * 4,
            "{:?}",
            abandoned_at.elapsed()
        );
        assert!(summary.drain.abandoned);
        let transcribed = summary.chunks.transcribed;
        assert!((1..40).contains(&transcribed), "{:?}", summary.chunks);
        assert_eq!(summary.dropped.len() as u64, 40 - transcribed);
        assert_eq!(summary.lines, transcribed);
        assert_eq!(fake.call_count() as u64, transcribed);
        // What was kept is the oldest speech; what was lost follows it.
        let kept: Vec<u64> = sink.lines_of(Speaker::Me).iter().map(|l| l.0).collect();
        let lost: Vec<u64> = summary.dropped.iter().map(|c| c.start_ms).collect();
        assert!(kept.last().unwrap() < lost.first().unwrap());
        assert!(sink.finalized_once_at_the_end());
        assert_eq!(Arc::strong_count(&fake), 1);
    });
}

#[test]
fn nothing_is_dropped_however_far_behind_the_transcriber_is_below_the_limit() {
    watchdog(|| {
        // 200 chunks, all waiting on a transcriber that takes its time.
        let starts: Vec<u64> = (0..200).map(|n| 500 + n * 1_500).collect();
        let fake = Arc::new(FakeTranscriber::new().with_delay(Duration::from_millis(2)));
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([bursts(&starts, 300, 301_000).source(Speaker::Me)]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.stop();
        // stop() may have cut the playback short; whatever was cut is there.
        assert!(summary.dropped.is_empty());
        assert_eq!(summary.chunks.cut, summary.chunks.transcribed);
        assert_eq!(summary.lines, summary.chunks.cut);
        assert_eq!(fake.call_count() as u64, summary.chunks.cut);
        let starts_written: Vec<u64> = sink.written().iter().map(|l| l.start_ms).collect();
        let mut sorted = starts_written.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len() as u64,
            summary.lines,
            "no chunk was transcribed twice"
        );
    });
}

#[test]
fn a_sink_blocked_on_a_line_delays_stop_but_not_the_recording() {
    watchdog(|| {
        let gate = Gate::closed();
        let sink = RecordingSink::new().gated(&gate);
        let session = Arc::new(
            Session::start(
                sources([
                    bursts(&[300], 300, 60_000).live_source(Speaker::Me),
                    Tape::new().quiet(60_000).live_source(Speaker::Them),
                ]),
                Arc::new(FakeTranscriber::new()),
                sink.for_session(),
                config(),
            )
            .unwrap(),
        );
        let _open = OpenOnDrop(gate.clone());
        // The chunk has been transcribed and the worker sits in append().
        wait_for_status(&session, "the chunk to leave the queue", |s| {
            s.chunks.cut == 1 && s.queued_chunks == 0
        });
        let frames_then = session.status().source(Speaker::Me).unwrap().frames;
        thread::sleep(Duration::from_millis(100));
        let now = session.status();
        assert!(
            now.source(Speaker::Me).unwrap().frames > frames_then,
            "capture goes on"
        );
        assert_eq!(now.lines, 0);

        let stopper = {
            let session = Arc::clone(&session);
            thread::spawn(move || session.stop())
        };
        wait_for("capture to end", || session.status().drain.map(|_| ()));
        thread::sleep(Duration::from_millis(50));
        assert!(
            !stopper.is_finished(),
            "stop waits for the line to be written"
        );
        gate.open();
        let summary = stopper.join().unwrap();
        assert_eq!(summary.lines, 1);
        assert!(sink.finalized_once_at_the_end());
    });
}
