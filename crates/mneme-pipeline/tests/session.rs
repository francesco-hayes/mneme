//! The session with fake sources and a fake transcriber: lines and their
//! timestamps, stop in all its forms, backlog and its drain, queue order,
//! status.
//!
//! Synthetic "speech" is a tone burst over low noise (see `common::Tape`).
//! With the default chunker a burst becomes one chunk that starts 200 ms
//! before it and ends 200 ms after it.

mod common;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mneme_audio::{AudioSource, SourcePhase};
use mneme_core::{Segment, Speaker};
use mneme_pipeline::{
    ChunkCounts, EndReason, Session, SessionConfig, SessionState, StartError, Transcriber,
};
use mneme_transcribe::{FakeTranscriber, Language};

use common::{
    Gate, OpenOnDrop, RecordingSink, SinkEvent, SlowStartSource, Tape, TestTranscriber, bursts,
    config, sources, span, wait_for, wait_for_status, watchdog,
};

fn segment(speaker: Speaker, start_ms: u64, end_ms: u64, text: &str) -> Segment {
    Segment {
        speaker,
        start_ms,
        end_ms,
        text: text.to_owned(),
    }
}

#[test]
fn two_speakers_give_lines_with_chunk_start_plus_span_offset_and_one_finalize() {
    watchdog(|| {
        // Me speaks at 1.0 s (300 ms) and 4.0 s (500 ms); Them at 2.0 s
        // (400 ms) and 5.0 s (600 ms). Each chunk is its burst plus 200 ms
        // either side, so the four chunks have four different lengths.
        let me = Tape::new()
            .burst_at(1_000, 300)
            .burst_at(4_000, 500)
            .until(6_000);
        let them = Tape::new()
            .burst_at(2_000, 400)
            .burst_at(5_000, 600)
            .until(7_000);
        // Two spans per chunk, 100 ms and 350 ms into it, named after the
        // chunk's length.
        let transcriber = Arc::new(TestTranscriber::new(|_, samples| {
            let name = common::length_text(samples);
            Ok(vec![
                span(100, 300, &format!("{name}, first")),
                span(350, 600, &format!("{name}, second")),
            ])
        }));
        let sink = RecordingSink::new();

        let session = Session::start(
            sources([me.source(Speaker::Me), them.source(Speaker::Them)]),
            transcriber.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        // Both fakes run out; the session finishes by itself.
        let summary = session.wait();

        assert_eq!(
            sink.written_sorted(),
            [
                segment(Speaker::Me, 900, 1_100, "700 ms, first"),
                segment(Speaker::Me, 1_150, 1_400, "700 ms, second"),
                segment(Speaker::Them, 1_900, 2_100, "800 ms, first"),
                segment(Speaker::Them, 2_150, 2_400, "800 ms, second"),
                segment(Speaker::Me, 3_900, 4_100, "900 ms, first"),
                segment(Speaker::Me, 4_150, 4_400, "900 ms, second"),
                segment(Speaker::Them, 4_900, 5_100, "1000 ms, first"),
                segment(Speaker::Them, 5_150, 5_400, "1000 ms, second"),
            ]
        );
        assert!(sink.finalized_once_at_the_end(), "{:?}", sink.events());
        assert!(sink.dropped_by_session());

        assert_eq!(summary.end, EndReason::SourcesEnded);
        assert_eq!(summary.lines, 8);
        assert_eq!(
            summary.chunks,
            ChunkCounts {
                cut: 4,
                transcribed: 4,
                failed: 0,
                dropped: 0
            }
        );
        assert!(summary.finalized);
        assert!(summary.transcript_is_complete(), "{summary:?}");
        assert!(summary.source_errors().is_empty());
        assert_eq!(summary.audio, None);
        assert_eq!(transcriber.finished(), 4);
        assert!(!transcriber.overlapped(), "one worker, one call at a time");

        let status = session.status();
        assert_eq!(status.state, SessionState::Done);
        assert_eq!(status.lines, 8);
        assert_eq!(status.untranscribed(), Duration::ZERO);
    });
}

#[test]
fn the_language_given_reaches_the_transcriber_with_every_chunk() {
    watchdog(|| {
        let fake = Arc::new(FakeTranscriber::new());
        let german: Language = "de".parse().unwrap();
        let session = Session::start(
            sources([bursts(&[500, 2_000], 300, 3_000).source(Speaker::Me)]),
            fake.clone(),
            RecordingSink::new().for_session(),
            SessionConfig {
                language: german.clone(),
                ..config()
            },
        )
        .unwrap();
        session.wait();
        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call.language == german));
        // The chunks went in whole: burst plus padding.
        assert!(calls.iter().all(|call| call.sample_count == 700 * 16));
    });
}

#[test]
fn stop_ends_running_sources_keeps_the_last_words_and_freezes_the_clock() {
    watchdog(|| {
        // Me talks without a pause from 0.3 s on: no chunk is complete until
        // the stream is flushed. Them says one thing at 0.4 s and is quiet.
        let me = Tape::new().quiet(300).talking(60_000);
        let them = bursts(&[400], 300, 60_000);
        let sink = RecordingSink::new();
        let transcriber = Arc::new(TestTranscriber::naming_lengths());
        let session = Session::start(
            sources([me.live_source(Speaker::Me), them.live_source(Speaker::Them)]),
            transcriber.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();

        // Them's chunk is complete 700 ms after its burst.
        let before = wait_for_status(&session, "Them's line", |s| s.lines == 1);
        assert_eq!(before.state, SessionState::Recording);
        assert_eq!(before.end, None);
        assert_eq!(before.drain, None);
        assert_eq!(
            before.source(Speaker::Me).unwrap().phase,
            SourcePhase::Running
        );
        assert_eq!(sink.finalize_count(), 0);

        let stop_called = session.elapsed();
        let summary = session.stop();

        assert_eq!(summary.end, EndReason::StopRequested);
        assert_eq!(
            summary.source(Speaker::Me).unwrap().phase,
            SourcePhase::Stopped
        );
        assert_eq!(
            summary.source(Speaker::Them).unwrap().phase,
            SourcePhase::Stopped
        );
        assert!(summary.failed_sources().is_empty());
        assert!(summary.transcript_is_complete(), "{summary:?}");

        // Them's line, and Me's unfinished chunk, flushed at stop.
        assert_eq!(sink.lines_of(Speaker::Them), [(200, "700 ms".to_owned())]);
        let me_lines = sink.written_sorted();
        let me_line = me_lines
            .iter()
            .find(|line| line.speaker == Speaker::Me)
            .expect("Me's last words were flushed at stop");
        assert_eq!(me_line.start_ms, 100, "200 ms before the speech at 300 ms");
        assert_eq!(summary.lines, 2);
        assert!(sink.finalized_once_at_the_end(), "{:?}", sink.events());

        // The duration is the meeting time at the stop, and Me's chunk
        // reaches up to about there.
        assert!(summary.duration >= stop_called, "{summary:?}");
        assert!(summary.duration < stop_called + Duration::from_secs(5));
        let duration_ms = summary.duration.as_millis() as u64;
        assert!(
            me_line.end_ms <= duration_ms + 40 && me_line.end_ms + 500 >= duration_ms,
            "Me's chunk ends at {} ms, the meeting at {duration_ms} ms",
            me_line.end_ms
        );

        // The clock stands still.
        assert_eq!(session.elapsed(), summary.duration);
        thread::sleep(Duration::from_millis(30));
        assert_eq!(session.elapsed(), summary.duration);
        assert_eq!(session.status().elapsed, summary.duration);
        assert_eq!(session.status().state, SessionState::Done);
        assert!(session.is_done());
    });
}

#[test]
fn the_clock_is_meeting_time_while_recording() {
    watchdog(|| {
        let session = Session::start(
            sources([Tape::new().quiet(60_000).live_source(Speaker::Me)]),
            Arc::new(FakeTranscriber::new()),
            RecordingSink::new().for_session(),
            config(),
        )
        .unwrap();
        let since_start = session.started_at().elapsed();
        let elapsed = session.elapsed();
        assert!(elapsed >= since_start && elapsed < since_start + Duration::from_secs(2));
        thread::sleep(Duration::from_millis(50));
        let later = session.elapsed();
        assert!(later >= elapsed + Duration::from_millis(50));
        assert!(session.status().elapsed >= later);
        session.stop();
    });
}

#[test]
fn a_slow_transcriber_builds_a_backlog_and_stop_returns_only_when_it_is_gone() {
    watchdog(|| {
        const DELAY: Duration = Duration::from_millis(120);
        // Eight things said by Me, delivered at once. Them stays quiet in
        // real time, which keeps the session recording.
        let starts: Vec<u64> = (0..8).map(|n| 1_000 + n * 1_500).collect();
        let me = bursts(&starts, 300, 13_500);
        let them = Tape::new().quiet(120_000);
        let fake = Arc::new(FakeTranscriber::new().with_delay(DELAY));
        let sink = RecordingSink::new();
        let session = Arc::new(
            Session::start(
                sources([me.source(Speaker::Me), them.live_source(Speaker::Them)]),
                fake.clone(),
                sink.for_session(),
                config(),
            )
            .unwrap(),
        );

        // All eight chunks are cut long before the transcriber has got
        // through them.
        let behind = wait_for_status(&session, "all chunks cut", |s| s.chunks.cut == 8);
        assert_eq!(behind.state, SessionState::Recording);
        assert!(
            behind.backlog >= Duration::from_millis(4 * 700),
            "expected a backlog, got {behind:?}"
        );
        assert_eq!(
            behind.backlog,
            Duration::from_millis(700) * behind.queued_chunks as u32
        );
        assert!(fake.call_count() < 8);
        assert_eq!(behind.drain, None);

        // Watch the drain from another thread while stop() blocks.
        let watcher = {
            let session = Arc::clone(&session);
            thread::spawn(move || {
                let mut seen = Vec::new();
                loop {
                    let status = session.status();
                    if status.state == SessionState::Done {
                        return seen;
                    }
                    if let Some(drain) = status.drain {
                        assert_eq!(status.state, SessionState::Stopping);
                        seen.push((drain.total, drain.remaining));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };

        let stop_began = Instant::now();
        let summary = session.stop();
        let stop_took = stop_began.elapsed();

        // Every chunk was transcribed before stop() returned.
        assert_eq!(fake.call_count(), 8);
        assert_eq!(summary.lines, 8);
        assert_eq!(summary.chunks.transcribed, 8);
        assert_eq!(sink.written().len(), 8);
        assert!(sink.finalized_once_at_the_end());
        assert!(summary.transcript_is_complete(), "{summary:?}");
        assert!(summary.dropped.is_empty());

        // And that took as long as the backlog needed: one delay per chunk
        // that was still waiting when capture ended.
        assert!(summary.drain.chunks >= 3, "{:?}", summary.drain);
        assert!(summary.drain.audio >= Duration::from_millis(700) * summary.drain.chunks as u32);
        assert!(!summary.drain.abandoned);
        let least = DELAY * summary.drain.chunks as u32;
        assert!(summary.drain.took >= least, "{:?}", summary.drain);
        assert!(stop_took >= least, "stop took {stop_took:?}");
        assert!(summary.stopping_took >= summary.drain.took);

        // The drain could be followed while it ran.
        let seen = watcher.join().unwrap();
        assert!(seen.len() >= 3, "saw {seen:?}");
        assert!(seen.iter().all(|(total, _)| *total == summary.drain.audio));
        assert!(
            seen.windows(2).all(|pair| pair[1].1 <= pair[0].1),
            "remaining audio only goes down: {seen:?}"
        );
        assert!(seen.first().unwrap().1 > seen.last().unwrap().1);

        let after = session.status();
        assert_eq!(after.untranscribed(), Duration::ZERO);
        let drain = after.drain.unwrap();
        assert_eq!(drain.remaining, Duration::ZERO);
        assert_eq!(drain.running_for, summary.drain.took);
    });
}

#[test]
fn waiting_chunks_are_served_oldest_start_first_not_in_arrival_order() {
    watchdog(|| {
        // Me's speech is late in the meeting but arrives first; Them's is
        // early in the meeting and arrives second, because Them's source is
        // slow to start.
        let me = Tape::new()
            .burst_at(5_000, 300)
            .burst_at(8_000, 500)
            .until(10_000);
        let them = Tape::new()
            .burst_at(1_000, 400)
            .burst_at(3_000, 600)
            .until(5_000);
        let source_gate = Gate::closed();
        let transcriber_gate = Gate::closed();
        let transcriber = Arc::new(TestTranscriber::naming_lengths().gated(&transcriber_gate));
        let sink = RecordingSink::new();
        let all: Vec<Box<dyn AudioSource>> = vec![
            Box::new(me.source(Speaker::Me)),
            Box::new(SlowStartSource::new(
                them.source(Speaker::Them),
                &source_gate,
            )),
        ];
        let session =
            Session::start(all, transcriber.clone(), sink.for_session(), config()).unwrap();
        let _open = (
            OpenOnDrop(source_gate.clone()),
            OpenOnDrop(transcriber_gate.clone()),
        );

        // The worker holds Me's first chunk; Me's second is waiting.
        wait_for_status(&session, "Me's chunks", |s| s.chunks.cut == 2);
        wait_for("the first transcriber call", || {
            (transcriber.entered() == 1).then_some(())
        });
        source_gate.open();
        let waiting = wait_for_status(&session, "Them's chunks", |s| s.chunks.cut == 4);
        assert_eq!(waiting.queued_chunks, 3);
        assert_eq!(waiting.transcribing, Duration::from_millis(700));
        assert_eq!(waiting.backlog, Duration::from_millis(900 + 800 + 1_000));
        assert_eq!(waiting.lines, 0);

        transcriber_gate.open();
        session.wait();

        let served: Vec<(Speaker, u64)> = sink
            .offered()
            .iter()
            .map(|line| (line.speaker, line.start_ms))
            .collect();
        assert_eq!(
            served,
            [
                // Already with the transcriber when the others arrived.
                (Speaker::Me, 4_800),
                // Then by start time, although Me's 7.8 s arrived before both.
                (Speaker::Them, 800),
                (Speaker::Them, 2_800),
                (Speaker::Me, 7_800),
            ]
        );
    });
}

#[test]
fn status_is_read_at_once_while_the_transcriber_and_the_sink_are_busy() {
    watchdog(|| {
        let transcriber_gate = Gate::closed();
        let sink_gate = Gate::closed();
        let transcriber = Arc::new(TestTranscriber::naming_lengths().gated(&transcriber_gate));
        let sink = RecordingSink::new().gated(&sink_gate);
        let session = Session::start(
            sources([
                bursts(&[500, 2_000], 300, 3_000).source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            transcriber.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let _open = (
            OpenOnDrop(transcriber_gate.clone()),
            OpenOnDrop(sink_gate.clone()),
        );

        // The worker is inside transcribe() and stays there.
        wait_for("the transcriber call", || {
            (transcriber.entered() == 1).then_some(())
        });
        wait_for_status(&session, "both chunks", |s| s.chunks.cut == 2);
        let began = Instant::now();
        for _ in 0..1_000 {
            let status = session.status();
            assert_eq!(status.transcribing, Duration::from_millis(700));
            assert_eq!(status.backlog, Duration::from_millis(700));
            assert_eq!(status.lines, 0);
        }
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "{:?}",
            began.elapsed()
        );
        assert_eq!(transcriber.finished(), 0);

        // Now the worker is inside append() and stays there.
        transcriber_gate.open();
        wait_for("the second transcriber call or the append", || {
            (transcriber.finished() >= 1).then_some(())
        });
        let began = Instant::now();
        for _ in 0..1_000 {
            let status = session.status();
            assert_eq!(status.lines, 0, "the append has not returned");
            assert_eq!(status.state, SessionState::Recording);
            let _ = session.elapsed();
        }
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "{:?}",
            began.elapsed()
        );
        assert!(sink.events().is_empty());

        sink_gate.open();
        let summary = session.stop();
        assert_eq!(summary.lines, 2);
    });
}

#[test]
fn stop_twice_returns_the_same_summary_and_finalizes_once() {
    watchdog(|| {
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&[500], 300, 60_000).live_source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            Arc::new(FakeTranscriber::new()),
            sink.for_session(),
            config(),
        )
        .unwrap();
        wait_for_status(&session, "a line", |s| s.lines == 1);

        let first = session.stop();
        let events = sink.events();
        let second = session.stop();
        session.request_stop();
        session.abandon_backlog();
        let third = session.stop();
        let waited = session.wait();
        let timed = session.wait_timeout(Duration::ZERO);

        assert_eq!(first, second);
        assert_eq!(first, third);
        assert_eq!(first, waited);
        assert_eq!(Some(first.clone()), timed);
        assert_eq!(first.end, EndReason::StopRequested);
        assert_eq!(sink.finalize_count(), 1);
        assert_eq!(
            sink.events(),
            events,
            "nothing reached the sink after the first stop"
        );
        assert!(
            first.dropped.is_empty(),
            "abandoning after the end drops nothing"
        );
    });
}

#[test]
fn stop_from_two_threads_at_once_gives_both_the_same_summary() {
    watchdog(|| {
        for _ in 0..10 {
            let sink = RecordingSink::new();
            let session = Arc::new(
                Session::start(
                    sources([
                        Tape::new().talking(60_000).live_source(Speaker::Me),
                        Tape::new().talking(60_000).live_source(Speaker::Them),
                    ]),
                    Arc::new(FakeTranscriber::new().with_delay(Duration::from_millis(10))),
                    sink.for_session(),
                    config(),
                )
                .unwrap(),
            );
            // Enough speech from both for the flush at stop to keep it.
            wait_for_status(&session, "400 ms from both sources", |s| {
                s.sources
                    .iter()
                    .all(|source| source.end_ms.unwrap_or(0) >= 400)
            });
            let stoppers: Vec<_> = (0..2)
                .map(|_| {
                    let session = Arc::clone(&session);
                    thread::spawn(move || session.stop())
                })
                .collect();
            let third = session.stop();
            for stopper in stoppers {
                assert_eq!(stopper.join().unwrap(), third);
            }
            assert_eq!(sink.finalize_count(), 1);
            // Both speakers were mid-sentence: one flushed chunk each.
            assert_eq!(third.lines, 2);
            assert_eq!(third.chunks.cut, 2);
        }
    });
}

#[test]
fn request_stop_shows_stopping_at_once_and_wait_collects_the_summary() {
    watchdog(|| {
        let gate = Gate::closed();
        let transcriber = Arc::new(TestTranscriber::naming_lengths().gated(&gate));
        let session = Session::start(
            sources([
                bursts(&[500], 300, 2_000).source(Speaker::Me),
                Tape::new().quiet(60_000).live_source(Speaker::Them),
            ]),
            transcriber.clone(),
            RecordingSink::new().for_session(),
            config(),
        )
        .unwrap();
        let _open = OpenOnDrop(gate.clone());
        wait_for("the transcriber call", || {
            (transcriber.entered() == 1).then_some(())
        });

        session.request_stop();
        assert_eq!(session.status().state, SessionState::Stopping);
        // The transcriber is still holding the only chunk: not done.
        assert_eq!(session.wait_timeout(Duration::from_millis(100)), None);
        let stopping = session.status();
        assert_eq!(stopping.state, SessionState::Stopping);
        assert_eq!(stopping.end, Some(EndReason::StopRequested));
        let drain = stopping.drain.expect("capture is over");
        assert_eq!(drain.remaining, Duration::from_millis(700));
        assert_eq!(drain.chunks_remaining, 0);
        assert!(!session.is_done());

        gate.open();
        let summary = session.wait();
        assert_eq!(summary.lines, 1);
        assert_eq!(summary.end, EndReason::StopRequested);
    });
}

#[test]
fn stop_while_both_sources_are_still_starting_returns_and_abandons_them() {
    watchdog(|| {
        let gate = Gate::closed();
        let me = SlowStartSource::new(bursts(&[100], 300, 1_000).source(Speaker::Me), &gate);
        let them = SlowStartSource::new(bursts(&[100], 300, 1_000).source(Speaker::Them), &gate);
        let (me_entered, them_entered) = (me.entered.clone(), them.entered.clone());
        let fake = Arc::new(FakeTranscriber::new());
        let sink = RecordingSink::new();
        let all: Vec<Box<dyn AudioSource>> = vec![Box::new(me), Box::new(them)];
        let session = Session::start(all, fake.clone(), sink.for_session(), config()).unwrap();
        let _open = OpenOnDrop(gate.clone());

        wait_for("both starts to be entered", || {
            use std::sync::atomic::Ordering::SeqCst;
            (me_entered.load(SeqCst) && them_entered.load(SeqCst)).then_some(())
        });
        thread::sleep(Duration::from_millis(60));
        let starting = session.status();
        assert_eq!(starting.state, SessionState::Starting);
        assert!(
            starting
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Starting)
        );

        let began = Instant::now();
        let summary = session.stop();
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "stop waited for a blocked start"
        );

        assert_eq!(summary.end, EndReason::StopRequested);
        assert!(
            summary
                .sources
                .iter()
                .all(|s| s.phase == SourcePhase::Abandoned)
        );
        assert_eq!(summary.failed_sources().len(), 2);
        assert_eq!(summary.lines, 0);
        assert_eq!(summary.chunks, ChunkCounts::default());
        assert_eq!(fake.call_count(), 0);
        assert_eq!(sink.events(), [SinkEvent::Finalize]);
        assert!(summary.finalized);

        // The starts return after the session is over: nothing reaches it.
        gate.open();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(sink.events(), [SinkEvent::Finalize]);
        assert_eq!(session.stop(), summary);
    });
}

#[test]
fn stop_right_after_start_returns_whatever_the_sources_were_doing() {
    watchdog(|| {
        // No waiting between start and stop: the sources may be starting,
        // running or not yet looked at.
        for round in 0..30 {
            let sink = RecordingSink::new();
            let fake = Arc::new(FakeTranscriber::new());
            let session = Session::start(
                sources([
                    Tape::new().talking(30_000).live_source(Speaker::Me),
                    Tape::new().talking(30_000).live_source(Speaker::Them),
                ]),
                fake.clone(),
                sink.for_session(),
                config(),
            )
            .unwrap();
            if round % 3 == 1 {
                thread::sleep(Duration::from_millis(round));
            }
            let summary = session.stop();
            assert_eq!(summary.end, EndReason::StopRequested);
            assert!(sink.finalized_once_at_the_end(), "{:?}", sink.events());
            assert_eq!(summary.chunks.cut, summary.chunks.transcribed);
            assert_eq!(summary.lines, sink.written().len() as u64);
            assert_eq!(Arc::strong_count(&fake), 1);
        }
    });
}

#[test]
fn a_source_that_starts_late_joins_the_meeting_at_its_own_time() {
    watchdog(|| {
        // Them's start blocks (a permission prompt); its audio then begins
        // 2 s into the meeting. Me is recording all along.
        let gate = Gate::closed();
        let them_audio = bursts(&[500], 300, 60_000)
            .live_source(Speaker::Them)
            .starting_at(Duration::from_millis(2_000));
        let them = SlowStartSource::new(them_audio, &gate);
        let me = bursts(&[300], 400, 60_000).live_source(Speaker::Me);
        let sink = RecordingSink::new();
        let all: Vec<Box<dyn AudioSource>> = vec![Box::new(me), Box::new(them)];
        let session = Session::start(
            all,
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let _open = OpenOnDrop(gate.clone());

        // Me's line arrives while Them is still starting.
        let early = wait_for_status(&session, "Me's line", |s| s.lines == 1);
        assert_eq!(early.state, SessionState::Recording);
        assert_eq!(
            early.source(Speaker::Them).unwrap().phase,
            SourcePhase::Starting
        );
        assert_eq!(
            early.source(Speaker::Me).unwrap().phase,
            SourcePhase::Running
        );
        assert_eq!(sink.lines_of(Speaker::Me), [(100, "800 ms".to_owned())]);

        gate.open();
        let later = wait_for_status(&session, "Them's line", |s| s.lines == 2);
        assert_eq!(
            later.source(Speaker::Them).unwrap().phase,
            SourcePhase::Running
        );
        // 2.0 s offset + burst at 0.5 s - 200 ms of padding.
        assert_eq!(sink.lines_of(Speaker::Them), [(2_300, "700 ms".to_owned())]);

        let summary = session.stop();
        assert!(summary.failed_sources().is_empty());
        assert_eq!(
            summary.source(Speaker::Them).unwrap().first_frame_ms,
            Some(2_000)
        );
    });
}

#[test]
fn a_source_that_never_starts_is_abandoned_at_stop_and_the_other_is_kept() {
    watchdog(|| {
        let gate = Gate::closed();
        let them = SlowStartSource::new(bursts(&[100], 300, 1_000).source(Speaker::Them), &gate);
        let me = bursts(&[300], 400, 60_000).live_source(Speaker::Me);
        let sink = RecordingSink::new();
        let all: Vec<Box<dyn AudioSource>> = vec![Box::new(me), Box::new(them)];
        let session = Session::start(
            all,
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let _open = OpenOnDrop(gate.clone());
        wait_for_status(&session, "Me's line", |s| s.lines == 1);

        let summary = session.stop();
        assert_eq!(
            summary.source(Speaker::Them).unwrap().phase,
            SourcePhase::Abandoned
        );
        assert_eq!(
            summary.source(Speaker::Me).unwrap().phase,
            SourcePhase::Stopped
        );
        assert_eq!(summary.failed_sources().len(), 1);
        assert_eq!(sink.lines_of(Speaker::Me), [(100, "800 ms".to_owned())]);
        assert!(sink.lines_of(Speaker::Them).is_empty());
    });
}

#[test]
fn when_every_source_has_ended_the_session_finishes_by_itself() {
    watchdog(|| {
        let sink = RecordingSink::new();
        let fake = Arc::new(FakeTranscriber::new());
        let session = Session::start(
            sources([
                bursts(&[500], 300, 2_000).source(Speaker::Me),
                bursts(&[900], 300, 1_500).source(Speaker::Them),
            ]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();

        // Nobody calls stop.
        let done = wait_for_status(&session, "the session to finish by itself", |s| {
            s.state == SessionState::Done
        });
        assert_eq!(done.end, Some(EndReason::SourcesEnded));
        assert_eq!(done.lines, 2);
        assert!(sink.finalized_once_at_the_end());
        assert!(sink.dropped_by_session());
        assert_eq!(Arc::strong_count(&fake), 1, "the transcriber was let go of");

        // stop() afterwards has nothing to do and says what happened.
        let began = Instant::now();
        let summary = session.stop();
        assert!(began.elapsed() < Duration::from_secs(1));
        assert_eq!(summary.end, EndReason::SourcesEnded);
        assert_eq!(summary.lines, 2);
        assert_eq!(summary.sources[0].phase, SourcePhase::Ended);
        assert_eq!(summary.sources[1].phase, SourcePhase::Ended);
        assert!(summary.source_errors().is_empty());
        assert_eq!(sink.finalize_count(), 1);
        assert_eq!(session.status().elapsed, summary.duration);
    });
}

#[test]
fn a_meeting_with_no_speech_gives_an_empty_finalized_transcript() {
    watchdog(|| {
        let sink = RecordingSink::new();
        let fake = Arc::new(FakeTranscriber::new());
        let session = Session::start(
            sources([
                Tape::new().quiet(3_000).source(Speaker::Me),
                Tape::new().quiet(3_000).source(Speaker::Them),
            ]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.stop();

        assert_eq!(
            fake.call_count(),
            0,
            "silence never reaches the transcriber"
        );
        assert_eq!(summary.lines, 0);
        assert_eq!(summary.chunks, ChunkCounts::default());
        assert_eq!(sink.events(), [SinkEvent::Finalize]);
        assert!(summary.transcript_is_complete());
        assert_eq!(summary.drain.audio, Duration::ZERO);
    });
}

#[test]
fn a_transcriber_that_finds_no_words_gives_no_lines_and_no_error() {
    watchdog(|| {
        let sink = RecordingSink::new();
        let fake = Arc::new(
            FakeTranscriber::new()
                .then_silence()
                .then_text("Something was said.")
                .then_silence(),
        );
        let session = Session::start(
            sources([bursts(&[500, 2_000, 3_500], 300, 5_000).source(Speaker::Me)]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.wait();

        assert_eq!(fake.call_count(), 3);
        assert_eq!(summary.lines, 1);
        assert_eq!(
            summary.chunks,
            ChunkCounts {
                cut: 3,
                transcribed: 3,
                failed: 0,
                dropped: 0
            }
        );
        assert!(summary.transcription_errors.is_empty());
        assert!(summary.transcript_is_complete());
        assert_eq!(
            sink.written(),
            [segment(Speaker::Me, 1_800, 2_500, "Something was said.")]
        );
        assert_eq!(session.status().last_error, None);
    });
}

#[test]
fn a_span_with_no_text_is_offered_but_not_counted_as_a_line() {
    watchdog(|| {
        let sink = RecordingSink::new();
        // The fake sink, like the store's writer, writes nothing for blank
        // text and says so.
        let transcriber = Arc::new(TestTranscriber::new(|_, _| {
            Ok(vec![span(0, 100, "   "), span(100, 300, "Words.")])
        }));
        let session = Session::start(
            sources([bursts(&[500], 300, 2_000).source(Speaker::Me)]),
            transcriber,
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.wait();
        assert_eq!(sink.offered().len(), 2);
        assert_eq!(summary.lines, 1);
        assert!(summary.sink_errors.is_empty());
    });
}

#[test]
fn one_source_alone_is_a_session() {
    watchdog(|| {
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([bursts(&[500, 2_000], 300, 3_000).source(Speaker::Them)]),
            Arc::new(TestTranscriber::naming_lengths()),
            sink.for_session(),
            config(),
        )
        .unwrap();
        let summary = session.wait();
        assert_eq!(summary.sources.len(), 1);
        assert_eq!(
            sink.lines_of(Speaker::Them),
            [(300, "700 ms".to_owned()), (1_800, "700 ms".to_owned())]
        );
        assert!(sink.lines_of(Speaker::Me).is_empty());
    });
}

#[test]
fn start_refuses_no_sources_and_two_sources_with_one_label() {
    watchdog(|| {
        let fake = Arc::new(FakeTranscriber::new());
        let sink = RecordingSink::new();

        let none = Session::start(Vec::new(), fake.clone(), sink.for_session(), config());
        assert_eq!(none.unwrap_err(), StartError::NoSources);

        let twice = Session::start(
            sources([
                bursts(&[500], 300, 1_000).source(Speaker::Me),
                bursts(&[500], 300, 1_000).source(Speaker::Me),
            ]),
            fake.clone(),
            sink.for_session(),
            config(),
        );
        let error = twice.unwrap_err();
        assert_eq!(error, StartError::DuplicateSpeaker(Speaker::Me));
        assert!(error.to_string().contains("`Me`"));

        // Nothing ran, nothing is held.
        assert_eq!(Arc::strong_count(&fake), 1);
        assert!(sink.events().is_empty());
        assert_eq!(fake.call_count(), 0);
    });
}

#[test]
fn after_stop_the_session_holds_no_reference_to_the_transcriber() {
    watchdog(|| {
        let fake = Arc::new(FakeTranscriber::new().with_delay(Duration::from_millis(20)));
        let transcriber: Arc<dyn Transcriber> = fake.clone();
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&[200, 1_500, 3_000], 300, 60_000).source(Speaker::Me),
                Tape::new().talking(60_000).live_source(Speaker::Them),
            ]),
            transcriber,
            sink.for_session(),
            config(),
        )
        .unwrap();
        wait_for_status(&session, "a line", |s| s.lines >= 1);
        assert!(Arc::strong_count(&fake) > 1);

        session.stop();

        // The moment stop() returns, not some time later: this is what lets
        // the caller drop a Whisper engine before the process exits.
        assert_eq!(Arc::strong_count(&fake), 1);
        assert!(sink.dropped_by_session());
        // The session handle itself keeps nothing alive either.
        drop(session);
        assert!(Arc::try_unwrap(fake).is_ok());
    });
}

#[test]
fn dropping_a_session_stops_it_and_finishes_the_transcript() {
    watchdog(|| {
        let fake = Arc::new(FakeTranscriber::new().with_delay(Duration::from_millis(30)));
        let sink = RecordingSink::new();
        let session = Session::start(
            sources([
                bursts(&[200, 1_500, 3_000, 4_500], 300, 6_000).source(Speaker::Me),
                Tape::new().talking(60_000).live_source(Speaker::Them),
            ]),
            fake.clone(),
            sink.for_session(),
            config(),
        )
        .unwrap();
        wait_for_status(&session, "all of Me's chunks", |s| s.chunks.cut == 4);
        wait_for_status(&session, "half a second of Them", |s| {
            s.source(Speaker::Them).unwrap().end_ms.unwrap_or(0) >= 500
        });

        drop(session);

        // Everything queued was transcribed, Them's open chunk was flushed,
        // the sink was finalized and let go of, and so was the transcriber.
        assert_eq!(sink.lines_of(Speaker::Me).len(), 4);
        assert_eq!(sink.lines_of(Speaker::Them).len(), 1);
        assert!(sink.finalized_once_at_the_end());
        assert!(sink.dropped_by_session());
        assert_eq!(Arc::strong_count(&fake), 1);
    });
}
