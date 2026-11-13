//! [`FakeTranscriber`]: a [`Transcriber`] that returns scripted text, for
//! tests in this crate and in the crates that build on it.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use crate::transcriber::{Language, TextSpan, TranscribeError, Transcriber, duration_ms};

/// A transcriber with no model: each call returns the next scripted reply.
///
/// Replies are used in the order they were added, one per call. Once the
/// script runs out, a call returns one span covering the whole slice with the
/// text `chunk N`, where `N` counts calls from 1, so a test can tell which
/// call produced which line without scripting every one. An empty slice
/// always returns no spans and does not use up a reply, like the real engine.
///
/// Every call is recorded; see [`FakeTranscriber::calls`].
///
/// ```
/// use mneme_transcribe::{FakeTranscriber, Language, Transcriber};
///
/// let fake = FakeTranscriber::new().then_text("Hello.").then_silence();
/// let audio = vec![0.0_f32; 16_000];
/// let language = Language::auto();
///
/// let spans = fake.transcribe(&audio, &language).unwrap();
/// assert_eq!(spans[0].text, "Hello.");
/// assert_eq!((spans[0].start_ms, spans[0].end_ms), (0, 1000));
/// assert!(fake.transcribe(&audio, &language).unwrap().is_empty());
/// assert_eq!(fake.transcribe(&audio, &language).unwrap()[0].text, "chunk 3");
/// ```
#[derive(Debug, Default)]
pub struct FakeTranscriber {
    delay: Duration,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    script: VecDeque<Reply>,
    calls: Vec<FakeCall>,
}

#[derive(Debug)]
enum Reply {
    /// One span covering the whole slice.
    Text(String),
    Spans(Vec<TextSpan>),
    Error(TranscribeError),
}

/// One recorded call to a [`FakeTranscriber`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeCall {
    /// How many samples the slice held.
    pub sample_count: usize,
    /// The language the caller asked for.
    pub language: Language,
}

impl FakeTranscriber {
    /// A fake with an empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake scripted with one text reply per item.
    pub fn with_texts<I, S>(texts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        texts
            .into_iter()
            .fold(Self::new(), |fake, text| fake.then_text(text))
    }

    /// Adds a reply: one span with this text covering the whole slice.
    pub fn then_text(self, text: impl Into<String>) -> Self {
        self.then(Reply::Text(text.into()))
    }

    /// Adds a reply: exactly these spans.
    pub fn then_spans(self, spans: Vec<TextSpan>) -> Self {
        self.then(Reply::Spans(spans))
    }

    /// Adds a reply with no spans, as for a slice with no speech.
    pub fn then_silence(self) -> Self {
        self.then(Reply::Spans(Vec::new()))
    }

    /// Adds a reply that fails with `error`.
    pub fn then_error(self, error: TranscribeError) -> Self {
        self.then(Reply::Error(error))
    }

    /// Makes every call block for `delay` before replying, to imitate a model
    /// that is slower than the audio coming in.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Every call so far, oldest first.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.lock().calls.clone()
    }

    /// How many calls have been made.
    pub fn call_count(&self) -> usize {
        self.lock().calls.len()
    }

    fn then(self, reply: Reply) -> Self {
        self.lock().script.push_back(reply);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Transcriber for FakeTranscriber {
    fn transcribe(
        &self,
        samples: &[f32],
        language: &Language,
    ) -> Result<Vec<TextSpan>, TranscribeError> {
        // Take the reply before sleeping, so replies go out in call order
        // even when several threads call at once.
        let reply = {
            let mut inner = self.lock();
            inner.calls.push(FakeCall {
                sample_count: samples.len(),
                language: language.clone(),
            });
            let call_number = inner.calls.len();
            if samples.is_empty() {
                Reply::Spans(Vec::new())
            } else {
                inner
                    .script
                    .pop_front()
                    .unwrap_or_else(|| Reply::Text(format!("chunk {call_number}")))
            }
        };

        if !self.delay.is_zero() {
            thread::sleep(self.delay);
        }

        match reply {
            Reply::Text(text) => Ok(vec![TextSpan {
                start_ms: 0,
                end_ms: duration_ms(samples.len()),
                text,
            }]),
            Reply::Spans(spans) => Ok(spans),
            Reply::Error(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;

    static ONE_SECOND: [f32; 16_000] = [0.0; 16_000];

    fn texts(spans: &[TextSpan]) -> Vec<&str> {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn replies_in_script_order() {
        let fake = FakeTranscriber::with_texts(["one", "two"]);
        let language = Language::auto();
        assert_eq!(
            texts(&fake.transcribe(&ONE_SECOND, &language).unwrap()),
            ["one"]
        );
        assert_eq!(
            texts(&fake.transcribe(&ONE_SECOND, &language).unwrap()),
            ["two"]
        );
    }

    #[test]
    fn text_reply_covers_the_whole_slice() {
        let fake = FakeTranscriber::new().then_text("Hello.");
        let spans = fake
            .transcribe(&ONE_SECOND[..8_000], &Language::auto())
            .unwrap();
        assert_eq!(
            spans,
            [TextSpan {
                start_ms: 0,
                end_ms: 500,
                text: "Hello.".to_owned()
            }]
        );
    }

    #[test]
    fn numbers_calls_after_the_script_runs_out() {
        let fake = FakeTranscriber::new().then_text("scripted");
        let language = Language::auto();
        let mut seen = Vec::new();
        for _ in 0..3 {
            let spans = fake.transcribe(&ONE_SECOND, &language).unwrap();
            seen.push(spans[0].text.clone());
        }
        assert_eq!(seen, ["scripted", "chunk 2", "chunk 3"]);
    }

    #[test]
    fn returns_scripted_spans_silence_and_errors() {
        let spans = vec![
            TextSpan {
                start_ms: 0,
                end_ms: 400,
                text: "First.".to_owned(),
            },
            TextSpan {
                start_ms: 600,
                end_ms: 900,
                text: "Second.".to_owned(),
            },
        ];
        let error = TranscribeError::Engine {
            message: "scripted failure".to_owned(),
        };
        let fake = FakeTranscriber::new()
            .then_spans(spans.clone())
            .then_silence()
            .then_error(error.clone())
            .then_text("after the error");
        let language = Language::auto();

        assert_eq!(fake.transcribe(&ONE_SECOND, &language), Ok(spans));
        assert_eq!(fake.transcribe(&ONE_SECOND, &language), Ok(Vec::new()));
        assert_eq!(fake.transcribe(&ONE_SECOND, &language), Err(error));
        assert_eq!(
            texts(&fake.transcribe(&ONE_SECOND, &language).unwrap()),
            ["after the error"]
        );
    }

    #[test]
    fn empty_slice_gives_nothing_and_keeps_the_script() {
        let fake = FakeTranscriber::new().then_text("kept");
        let language = Language::auto();
        assert_eq!(fake.transcribe(&[], &language), Ok(Vec::new()));
        assert_eq!(
            texts(&fake.transcribe(&ONE_SECOND, &language).unwrap()),
            ["kept"]
        );
    }

    #[test]
    fn records_every_call() {
        let fake = FakeTranscriber::new();
        let german: Language = "de".parse().unwrap();
        fake.transcribe(&ONE_SECOND, &Language::auto()).unwrap();
        fake.transcribe(&ONE_SECOND[..160], &german).unwrap();
        assert_eq!(fake.call_count(), 2);
        assert_eq!(
            fake.calls(),
            [
                FakeCall {
                    sample_count: 16_000,
                    language: Language::auto()
                },
                FakeCall {
                    sample_count: 160,
                    language: german
                },
            ]
        );
    }

    #[test]
    fn delay_blocks_each_call() {
        let delay = Duration::from_millis(30);
        let fake = FakeTranscriber::new().with_delay(delay);
        let started = Instant::now();
        fake.transcribe(&ONE_SECOND, &Language::auto()).unwrap();
        assert!(started.elapsed() >= delay);
    }

    #[test]
    fn works_as_a_shared_trait_object_across_threads() {
        let fake = Arc::new(FakeTranscriber::new());
        let shared: Arc<dyn Transcriber> = fake.clone();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let transcriber = Arc::clone(&shared);
                thread::spawn(move || {
                    transcriber
                        .transcribe(&ONE_SECOND, &Language::auto())
                        .unwrap()
                })
            })
            .collect();
        let mut seen: Vec<String> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().remove(0).text)
            .collect();
        seen.sort();
        assert_eq!(seen, ["chunk 1", "chunk 2", "chunk 3", "chunk 4"]);
        assert_eq!(fake.call_count(), 4);
    }
}
