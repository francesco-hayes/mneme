//! The chunks waiting for the transcriber.

use std::collections::BTreeMap;

use mneme_core::Speaker;
use mneme_transcribe::Chunk;

/// What is known about the chunk the worker has taken, after its samples
/// have gone to the transcriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InWork {
    pub(crate) speaker: Speaker,
    pub(crate) start_ms: u64,
    pub(crate) duration_ms: u64,
}

/// Chunks from both speakers, served oldest `start_ms` first.
///
/// Chunks arrive in the order the chunkers complete them, which is not time
/// order: a speaker's chunk is only complete once a pause follows it. When
/// more than one chunk is waiting, the one that starts earliest goes first;
/// chunks with the same start go in arrival order.
///
/// The queue also knows how much audio it holds and how long the chunk the
/// worker has taken is, so the backlog can be read without asking the
/// worker.
#[derive(Debug, Default)]
pub(crate) struct ChunkQueue {
    /// Keyed by start time, then by arrival.
    waiting: BTreeMap<(u64, u64), Chunk>,
    arrivals: u64,
    waiting_ms: u64,
    in_work: Option<InWork>,
    closed: bool,
}

impl ChunkQueue {
    /// Adds a chunk, unless the queue would then hold more than `max_ms` of
    /// audio: the chunk comes back and the queue is unchanged.
    pub(crate) fn push(&mut self, chunk: Chunk, max_ms: u64) -> Result<(), Chunk> {
        let duration_ms = chunk.duration_ms();
        if self.waiting_ms.saturating_add(duration_ms) > max_ms {
            return Err(chunk);
        }
        self.waiting_ms += duration_ms;
        self.waiting.insert((chunk.start_ms, self.arrivals), chunk);
        self.arrivals += 1;
        Ok(())
    }

    /// Takes the chunk with the earliest start. It counts as in work until
    /// [`finish_work`](Self::finish_work).
    pub(crate) fn pop_oldest(&mut self) -> Option<Chunk> {
        let (_, chunk) = self.waiting.pop_first()?;
        let duration_ms = chunk.duration_ms();
        self.waiting_ms -= duration_ms;
        self.in_work = Some(InWork {
            speaker: chunk.speaker,
            start_ms: chunk.start_ms,
            duration_ms,
        });
        Some(chunk)
    }

    /// The chunk taken with [`pop_oldest`](Self::pop_oldest) is dealt with.
    pub(crate) fn finish_work(&mut self) {
        self.in_work = None;
    }

    /// The chunk taken with [`pop_oldest`](Self::pop_oldest) will never be
    /// dealt with (the worker died). Returns what it was, if there was one.
    pub(crate) fn abandon_work(&mut self) -> Option<InWork> {
        self.in_work.take()
    }

    /// Removes every waiting chunk, oldest first.
    pub(crate) fn take_all(&mut self) -> Vec<Chunk> {
        self.waiting_ms = 0;
        std::mem::take(&mut self.waiting).into_values().collect()
    }

    /// No more chunks will be pushed.
    pub(crate) fn close(&mut self) {
        self.closed = true;
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// Chunks waiting, not counting the one in work.
    pub(crate) fn len(&self) -> usize {
        self.waiting.len()
    }

    /// Audio waiting, not counting the chunk in work.
    pub(crate) fn waiting_ms(&self) -> u64 {
        self.waiting_ms
    }

    /// Length of the chunk in work, zero if there is none.
    pub(crate) fn in_work_ms(&self) -> u64 {
        self.in_work.map_or(0, |chunk| chunk.duration_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(speaker: Speaker, start_ms: u64, ms: usize) -> Chunk {
        Chunk {
            speaker,
            start_ms,
            samples: vec![0.0; ms * 16],
        }
    }

    #[test]
    fn serves_the_earliest_start_first_and_ties_in_arrival_order() {
        let mut queue = ChunkQueue::default();
        // Completion order, as two chunkers would deliver.
        for (speaker, start_ms) in [
            (Speaker::Me, 9_000),
            (Speaker::Them, 2_000),
            (Speaker::Them, 9_000),
            (Speaker::Me, 500),
        ] {
            queue.push(chunk(speaker, start_ms, 100), u64::MAX).unwrap();
        }
        let mut served = Vec::new();
        while let Some(chunk) = queue.pop_oldest() {
            served.push((chunk.speaker, chunk.start_ms));
        }
        assert_eq!(
            served,
            [
                (Speaker::Me, 500),
                (Speaker::Them, 2_000),
                (Speaker::Me, 9_000),
                (Speaker::Them, 9_000),
            ]
        );
    }

    #[test]
    fn counts_the_audio_waiting_and_in_work() {
        let mut queue = ChunkQueue::default();
        queue.push(chunk(Speaker::Me, 0, 1_000), u64::MAX).unwrap();
        queue
            .push(chunk(Speaker::Them, 100, 2_500), u64::MAX)
            .unwrap();
        assert_eq!((queue.len(), queue.waiting_ms()), (2, 3_500));
        assert_eq!(queue.in_work_ms(), 0);

        queue.pop_oldest().unwrap();
        assert_eq!((queue.len(), queue.waiting_ms()), (1, 2_500));
        assert_eq!(queue.in_work_ms(), 1_000);

        queue.finish_work();
        assert_eq!(queue.in_work_ms(), 0);
        assert_eq!(queue.abandon_work(), None);

        let taken = queue.pop_oldest().unwrap();
        queue.push(taken, u64::MAX).unwrap();
        assert_eq!(
            queue.abandon_work(),
            Some(InWork {
                speaker: Speaker::Them,
                start_ms: 100,
                duration_ms: 2_500
            })
        );
        assert_eq!(queue.in_work_ms(), 0);
        assert_eq!(queue.take_all().len(), 1);
        assert_eq!((queue.len(), queue.waiting_ms()), (0, 0));
    }

    #[test]
    fn refuses_a_chunk_that_would_pass_the_limit_and_hands_it_back() {
        let mut queue = ChunkQueue::default();
        queue.push(chunk(Speaker::Me, 0, 600), 1_000).unwrap();
        let refused = queue.push(chunk(Speaker::Me, 700, 600), 1_000).unwrap_err();
        assert_eq!(refused.start_ms, 700);
        assert_eq!((queue.len(), queue.waiting_ms()), (1, 600));
        // Exactly at the limit still fits.
        queue.push(chunk(Speaker::Me, 1_400, 400), 1_000).unwrap();
        // Room comes back when the worker takes a chunk.
        queue.pop_oldest().unwrap();
        queue.push(chunk(Speaker::Me, 2_000, 600), 1_000).unwrap();
        assert_eq!(queue.waiting_ms(), 1_000);
    }
}
