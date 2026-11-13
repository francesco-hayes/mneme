//! What the measured times would have meant in a live meeting.
//!
//! A live session has one worker that transcribes chunks one at a time,
//! oldest first. A chunk joins the queue at the moment in the meeting when
//! the chunker releases it; the worker needs the measured time for it. From
//! those two lists this module works out how far the worker would have
//! fallen behind. Nothing here runs the engine or waits.

use serde::Serialize;

/// One chunk as the worker sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Job {
    /// Meeting time at which the chunk joined the queue, in milliseconds.
    pub available_at_ms: u64,
    /// Length of the chunk's audio, in milliseconds.
    pub audio_ms: u64,
    /// How long the engine took for it, in milliseconds.
    pub processing_ms: f64,
}

/// How a single worker would have coped. Times are in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Simulation {
    /// The longest any chunk went from joining the queue to having its text:
    /// how far the transcript ran behind the meeting at its worst. With a
    /// worker that keeps up this is the time the slowest chunk took.
    pub worst_lag_s: f64,
    /// The most audio that was waiting in the queue at any moment, not
    /// counting the chunk being transcribed. Zero when every chunk found the
    /// worker free.
    pub worst_queued_audio_s: f64,
    /// How long the session would have kept working after the recording
    /// stopped before the transcript was complete.
    pub stop_wait_s: f64,
    /// Chunks that had to wait because the worker was still busy.
    pub chunks_that_waited: usize,
}

/// Plays `jobs` (in queue order) through one worker. The recording stops at
/// `stop_at_ms`.
pub fn simulate(jobs: &[Job], stop_at_ms: u64) -> Simulation {
    let mut worst_lag_ms = 0.0_f64;
    let mut worst_queued_audio_ms = 0_u64;
    let mut chunks_that_waited = 0;
    // When the worker starts and finishes each job.
    let mut starts: Vec<f64> = Vec::with_capacity(jobs.len());
    let mut worker_free_at = 0.0_f64;

    for (index, job) in jobs.iter().enumerate() {
        let available_at = job.available_at_ms as f64;
        let start = available_at.max(worker_free_at);
        let finish = start + job.processing_ms.max(0.0);
        starts.push(start);
        worker_free_at = finish;

        worst_lag_ms = worst_lag_ms.max(finish - available_at);
        if start > available_at {
            chunks_that_waited += 1;
        }

        // The queue is longest just after a chunk joins it. Count the audio
        // of every chunk up to this one that has not been started yet.
        let queued: u64 = jobs[..=index]
            .iter()
            .zip(&starts)
            .filter(|(_, started)| **started > available_at)
            .map(|(queued, _)| queued.audio_ms)
            .sum();
        worst_queued_audio_ms = worst_queued_audio_ms.max(queued);
    }

    Simulation {
        worst_lag_s: worst_lag_ms / 1000.0,
        worst_queued_audio_s: worst_queued_audio_ms as f64 / 1000.0,
        stop_wait_s: (worker_free_at - stop_at_ms as f64).max(0.0) / 1000.0,
        chunks_that_waited,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(available_at_s: u64, audio_s: u64, processing_s: f64) -> Job {
        Job {
            available_at_ms: available_at_s * 1000,
            audio_ms: audio_s * 1000,
            processing_ms: processing_s * 1000.0,
        }
    }

    #[test]
    fn nothing_to_do_means_nothing_to_wait_for() {
        assert_eq!(
            simulate(&[], 60_000),
            Simulation {
                worst_lag_s: 0.0,
                worst_queued_audio_s: 0.0,
                stop_wait_s: 0.0,
                chunks_that_waited: 0,
            }
        );
    }

    #[test]
    fn a_fast_worker_is_never_behind_by_more_than_one_chunk_takes() {
        // A chunk every 10 s, one second of work each; the last one arrives
        // as the recording stops.
        let jobs = [job(10, 9, 1.0), job(20, 9, 0.5), job(30, 9, 2.0)];
        let result = simulate(&jobs, 30_000);
        assert_eq!(result.worst_lag_s, 2.0);
        assert_eq!(result.worst_queued_audio_s, 0.0);
        assert_eq!(result.chunks_that_waited, 0);
        // Stop still has to wait for the chunk that arrived with it.
        assert_eq!(result.stop_wait_s, 2.0);
    }

    #[test]
    fn stop_waits_for_nothing_when_the_last_chunk_was_done_earlier() {
        let jobs = [job(5, 4, 1.0), job(12, 6, 3.0)];
        let result = simulate(&jobs, 60_000);
        assert_eq!(result.stop_wait_s, 0.0);
        assert_eq!(result.worst_lag_s, 3.0);
    }

    #[test]
    fn a_slow_worker_falls_further_behind_with_every_chunk() {
        // 10 s of audio every 10 s, 15 s of work each.
        // Work runs 10-25, 25-40, 40-55.
        let jobs = [job(10, 10, 15.0), job(20, 10, 15.0), job(30, 10, 15.0)];
        let result = simulate(&jobs, 30_000);
        // Lags: 15, 20 and 25 s.
        assert_eq!(result.worst_lag_s, 25.0);
        // At 20 s the second chunk waits; at 30 s the second is being
        // worked on and the third waits. Never more than one chunk queued.
        assert_eq!(result.worst_queued_audio_s, 10.0);
        assert_eq!(result.chunks_that_waited, 2);
        assert_eq!(result.stop_wait_s, 25.0);
    }

    #[test]
    fn the_queue_holds_every_chunk_that_has_not_been_started() {
        // 35 s of work per chunk: work runs 10-45, 45-80, 80-115, 115-150.
        let jobs = [
            job(10, 10, 35.0),
            job(20, 7, 35.0),
            job(30, 8, 35.0),
            job(40, 9, 35.0),
        ];
        let result = simulate(&jobs, 40_000);
        // At 40 s the second, third and fourth chunk are all waiting.
        assert_eq!(result.worst_queued_audio_s, 24.0);
        assert_eq!(result.chunks_that_waited, 3);
        assert_eq!(result.worst_lag_s, 110.0);
        assert_eq!(result.stop_wait_s, 110.0);
    }

    #[test]
    fn a_backlog_drains_during_a_quiet_stretch() {
        // Two chunks close together, then a long gap before the third.
        // Work runs 10-18, 18-26, then 60-68.
        let jobs = [job(10, 9, 8.0), job(12, 2, 8.0), job(60, 9, 8.0)];
        let result = simulate(&jobs, 100_000);
        assert_eq!(result.worst_lag_s, 14.0);
        assert_eq!(result.worst_queued_audio_s, 2.0);
        assert_eq!(result.chunks_that_waited, 1);
        assert_eq!(result.stop_wait_s, 0.0);
    }

    #[test]
    fn slower_work_never_gives_a_better_result() {
        let base = [
            job(3, 2, 0.4),
            job(9, 5, 1.1),
            job(11, 1, 0.2),
            job(40, 28, 6.0),
            job(41, 1, 0.3),
        ];
        let mut previous = simulate(&base, 41_000);
        for factor in [2.0, 5.0, 20.0] {
            let slower: Vec<Job> = base
                .iter()
                .map(|job| Job {
                    processing_ms: job.processing_ms * factor,
                    ..*job
                })
                .collect();
            let result = simulate(&slower, 41_000);
            assert!(result.worst_lag_s >= previous.worst_lag_s);
            assert!(result.worst_queued_audio_s >= previous.worst_queued_audio_s);
            assert!(result.stop_wait_s >= previous.stop_wait_s);
            previous = result;
        }
    }
}
