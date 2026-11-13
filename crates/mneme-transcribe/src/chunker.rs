//! [`Chunker`]: cuts one speaker's stream of audio frames into chunks at
//! pauses, so Whisper gets whole phrases and never silence. The rules are in
//! the documentation of [`Chunker`]; the settings and the reasons for their
//! defaults are on [`ChunkerConfig`].

use std::collections::VecDeque;

use mneme_core::{AudioFrame, SAMPLE_RATE, Speaker};

use crate::transcriber::duration_ms;

/// Length of one analysis window. Long enough to hold two periods of the
/// lowest voices (about 100 Hz), short enough to place a chunk's start well
/// inside the one-second resolution of transcript timestamps.
const WINDOW_MS: u32 = 20;
const WINDOW_SAMPLES: usize = (SAMPLE_RATE / 1000 * WINDOW_MS) as usize;
const SAMPLES_PER_MS: u64 = (SAMPLE_RATE / 1000) as u64;

/// How much of the distance to a quieter window the noise floor covers per
/// window. Three windows (60 ms, a short gap between syllables) undo two
/// thirds of whatever the floor gained during speech.
const FLOOR_FALL: f32 = 0.3;
/// How much of the distance to a louder window that is still below the
/// speech threshold the noise floor covers per window: a time constant of
/// about one second.
const FLOOR_RISE: f32 = 0.02;
/// A forced cut scores each window boundary by the loudness of this many
/// windows on each side of it (100 ms), nearest windows weighing most.
const CUT_REACH_WINDOWS: usize = 5;
/// An earlier boundary replaces a later one only when its score is below
/// this share of the later one's, so that boundaries that differ by rounding
/// alone count as equal.
const CUT_MARGIN: f32 = 0.99;

/// Settings for a [`Chunker`]. `ChunkerConfig::default()` is what the app
/// uses.
///
/// Durations are rounded to whole 20 ms windows and every value is brought
/// into a workable range when the chunker is created;
/// [`Chunker::config`] returns the values in use.
///
/// The defaults were chosen from how Whisper behaves (see each field) and
/// from synthetic audio and one synthetic-speech recording. They have not
/// been tuned on real meeting audio yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkerConfig {
    /// A chunk ends after this long without speech. Default `700`.
    ///
    /// Whisper carries no context from one chunk to the next, so a cut in
    /// the middle of a sentence costs accuracy. Gaps inside a sentence are
    /// mostly under half a second; 700 ms mostly waits for the end of a
    /// sentence or a turn. Nothing shows the transcript live, so the extra
    /// wait costs nothing. Minimum 20.
    pub pause_ms: u32,

    /// No chunk is longer than this. Default `28_000`.
    ///
    /// whisper.cpp works on 30 s of audio at a time. A chunk under that is
    /// decoded in one piece; a longer one is split again inside whisper.cpp,
    /// where the chunker has no say in where. 28 s leaves room to spare and
    /// keeps forced cuts, which can land inside a sentence, rare.
    /// Minimum 1000.
    pub max_chunk_ms: u32,

    /// A chunk that reaches [`max_chunk_ms`](Self::max_chunk_ms) is cut at
    /// the quietest point of its last `cut_search_ms`. Default `8_000`.
    ///
    /// Eight seconds of speech nearly always hold a breath or a gap between
    /// words, so the cut rarely splits a word, and a forced chunk is still
    /// at least 20 s long. At most half of `max_chunk_ms`.
    pub cut_search_ms: u32,

    /// Audio kept before the first and after the last speech window of a
    /// chunk. Default `200`.
    ///
    /// Soft sounds at the edges of words (`f`, `s`, `h`, `th`, a trailing
    /// `t`) are often quieter than the speech threshold and can last 100 to
    /// 200 ms. The padding keeps them. It is a usual safety margin, not a
    /// measured need: the synthetic voice of the test recording has sharp
    /// word edges and loses no word even without padding. It also sets how
    /// far a chunk's `start_ms` sits before the detected start of speech.
    /// At most a quarter of `max_chunk_ms`.
    pub padding_ms: u32,

    /// A chunk with less speech than this is dropped. Default `160`.
    ///
    /// whisper.cpp returns nothing for less than 100 ms of audio, so
    /// anything shorter is wasted work. 160 ms is meant to sit between a
    /// click, pop or key press (a few hundredths of a second above the
    /// threshold) and the shortest answers ("yes", "no", "ok"). Minimum 20.
    pub min_speech_ms: u32,

    /// A window is speech when its RMS level is at least this many times the
    /// noise floor. Default `3.0` (about 9.5 dB).
    ///
    /// Lower finds quieter speech and mistakes more noise for it; higher
    /// does the opposite. Between 1 and 100.
    pub speech_ratio: f32,

    /// A window quieter than this RMS level is never speech, however quiet
    /// the room. Samples are nominally in `-1.0..=1.0`. Default `0.002`
    /// (-54 dBFS).
    ///
    /// This is what keeps digital silence and the hiss of a quiet input
    /// from counting as speech. Quiet speech from a laptop microphone is
    /// around 0.01. At least 0.00001.
    pub min_speech_level: f32,

    /// While sound stays above the speech threshold, the noise floor doubles
    /// every this many milliseconds. Default `2_000`.
    ///
    /// This is how long steady noise that starts suddenly keeps counting as
    /// speech: about 2 s for each doubling of loudness over the old
    /// threshold. Shorter adapts faster and starts to treat long loud
    /// stretches of speech as noise. Minimum 100.
    pub noise_doubling_ms: u32,

    /// A frame whose `start_ms` is further than this from where the previous
    /// frame ended is a break in the stream: the current chunk ends there.
    /// Default `100`.
    ///
    /// Smaller differences are taken as timestamp rounding or a dropped
    /// buffer, and the audio is treated as continuous: losing a few
    /// hundredths of a second inside a word hurts less than splitting the
    /// word between two chunks. Minimum 1.
    pub max_gap_ms: u32,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        ChunkerConfig {
            pause_ms: 700,
            max_chunk_ms: 28_000,
            cut_search_ms: 8_000,
            padding_ms: 200,
            min_speech_ms: 160,
            speech_ratio: 3.0,
            min_speech_level: 0.002,
            noise_doubling_ms: 2_000,
            max_gap_ms: 100,
        }
    }
}

impl ChunkerConfig {
    /// The same settings with durations rounded to whole windows and every
    /// value inside its documented range. Values that are not numbers fall
    /// back to the defaults.
    fn normalized(&self) -> ChunkerConfig {
        let defaults = ChunkerConfig::default();
        let round_up = |ms: u32| ms.div_ceil(WINDOW_MS).saturating_mul(WINDOW_MS);
        let round_down = |ms: u32| ms / WINDOW_MS * WINDOW_MS;

        let max_chunk_ms = round_down(self.max_chunk_ms.max(1_000));
        let speech_ratio = if self.speech_ratio.is_finite() {
            self.speech_ratio.clamp(1.0, 100.0)
        } else {
            defaults.speech_ratio
        };
        let min_speech_level = if self.min_speech_level.is_finite() {
            self.min_speech_level.clamp(0.000_01, 1.0)
        } else {
            defaults.min_speech_level
        };
        ChunkerConfig {
            pause_ms: round_up(self.pause_ms.clamp(WINDOW_MS, max_chunk_ms)),
            max_chunk_ms,
            cut_search_ms: round_down(self.cut_search_ms.clamp(WINDOW_MS, max_chunk_ms / 2)),
            padding_ms: round_down(round_up(self.padding_ms).min(max_chunk_ms / 4)),
            min_speech_ms: round_up(self.min_speech_ms.clamp(WINDOW_MS, max_chunk_ms / 2)),
            speech_ratio,
            min_speech_level,
            noise_doubling_ms: self.noise_doubling_ms.max(100),
            max_gap_ms: self.max_gap_ms.max(1),
        }
    }
}

/// A stretch of one speaker's audio, cut at pauses, ready to transcribe.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Who is speaking.
    pub speaker: Speaker,
    /// Meeting time of the first sample, in milliseconds. A line's timestamp
    /// is this plus the offset of its text inside the chunk (see
    /// [`TextSpan::into_segment`](crate::TextSpan::into_segment)).
    pub start_ms: u64,
    /// Mono samples at [`SAMPLE_RATE`](mneme_core::SAMPLE_RATE).
    pub samples: Vec<f32>,
}

impl Chunk {
    /// Length of the chunk in milliseconds, rounded up.
    pub fn duration_ms(&self) -> u64 {
        duration_ms(self.samples.len())
    }

    /// Meeting time just after the last sample, in milliseconds.
    pub fn end_ms(&self) -> u64 {
        self.start_ms.saturating_add(self.duration_ms())
    }
}

/// Counts of what a [`Chunker`] has done since it was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkerStats {
    /// Chunks returned.
    pub chunks: u64,
    /// Chunks thrown away for holding too little speech.
    pub dropped: u64,
    /// Cuts forced by the maximum chunk length.
    pub forced_cuts: u64,
    /// Breaks in the frame timestamps.
    pub gaps: u64,
}

/// One analysis window of an open chunk.
#[derive(Debug, Clone, Copy)]
struct Window {
    /// Mean square of the samples around their mean.
    energy: f32,
    speech: bool,
}

/// The chunk being collected.
#[derive(Debug)]
struct OpenChunk {
    start_ms: u64,
    /// Whole windows, except that `finish_stream` may append a shorter tail.
    samples: Vec<f32>,
    /// One entry per [`WINDOW_SAMPLES`] of `samples`.
    windows: Vec<Window>,
    /// How many windows at the end are not speech.
    silence_run: usize,
}

/// Where the sample count is pinned to the meeting clock.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    ms: u64,
    /// Count of samples received before the one captured at `ms`.
    index: u64,
}

/// Cuts a stream of [`AudioFrame`]s from one speaker into [`Chunk`]s at
/// pauses, and drops the silence in between.
///
/// Use one chunker per speaker. Push frames in the order they were captured;
/// each push returns the chunks that became complete. Call
/// [`flush`](Self::flush) when the recording stops, so the last words are not
/// lost.
///
/// # How it works
///
/// Audio is looked at in 20 ms windows. A window is **speech** when its
/// loudness (RMS, with any constant offset removed) is at least
/// [`speech_ratio`](ChunkerConfig::speech_ratio) times the current **noise
/// floor**, and never below
/// [`min_speech_level`](ChunkerConfig::min_speech_level).
///
/// The noise floor follows the room:
///
/// - a window quieter than the floor pulls it down within a few windows;
/// - a window between the floor and the speech threshold pulls it up over
///   about a second;
/// - while sound stays above the threshold the floor still creeps up,
///   doubling every [`noise_doubling_ms`](ChunkerConfig::noise_doubling_ms),
///   so a fan that switches on stops counting as speech after a few seconds.
///   Real speech dips to the floor between words and syllables, which pulls
///   the floor straight back down.
///
/// A chunk opens at the first speech window, reaching back
/// [`padding_ms`](ChunkerConfig::padding_ms) so a soft first consonant is
/// kept. Silence before that is not part of the chunk, so `start_ms` tells
/// when the speech began. The chunk closes once
/// [`pause_ms`](ChunkerConfig::pause_ms) pass without speech, and keeps the
/// same padding after the last speech window.
///
/// A chunk that reaches [`max_chunk_ms`](ChunkerConfig::max_chunk_ms) is cut
/// at the quietest point of its last
/// [`cut_search_ms`](ChunkerConfig::cut_search_ms). The audio after the cut
/// starts the next chunk, with nothing lost and nothing repeated.
///
/// A chunk holding less than
/// [`min_speech_ms`](ChunkerConfig::min_speech_ms) of speech is thrown away.
///
/// A break in the frame timestamps ends the chunk in progress; see
/// [`push_samples`](Self::push_samples).
///
/// # Limits
///
/// The detector measures loudness only. It cannot tell speech from other
/// sound that comes and goes: typing, a cough, music, a door, people talking
/// in the background. Those become chunks when they are loud and long
/// enough. Sound at a steady level is the opposite case: after some seconds
/// it is treated as noise, whatever it is. And speech less than
/// `speech_ratio` times louder than the room is not found.
///
/// A chunk is returned once the pause after it has passed, or at the next
/// forced cut, so it arrives up to
/// [`max_chunk_ms`](ChunkerConfig::max_chunk_ms) after its `start_ms`.
///
/// Memory is bounded: the chunker holds the open chunk (at most
/// `max_chunk_ms` of audio) or, while nobody speaks, the last
/// [`padding_ms`](ChunkerConfig::padding_ms).
///
/// ```
/// use mneme_core::{AudioFrame, Speaker};
/// use mneme_transcribe::{Chunker, ChunkerConfig};
///
/// let mut chunker = Chunker::new(Speaker::Me, ChunkerConfig::default());
/// let mut chunks = Vec::new();
///
/// // One second of quiet, half a second of a 200 Hz tone, one second of quiet,
/// // in 20 ms frames.
/// for n in 0..125_u64 {
///     let loud = (50..75).contains(&n);
///     let samples = (0..320)
///         .map(|i| if loud { 0.3 * (i as f32 * 0.0785).sin() } else { 0.0 })
///         .collect();
///     let frame = AudioFrame { speaker: Speaker::Me, start_ms: n * 20, samples };
///     chunks.extend(chunker.push(&frame));
/// }
/// chunks.extend(chunker.flush());
///
/// assert_eq!(chunks.len(), 1);
/// // The tone began at 1000 ms; the chunk starts 200 ms of padding earlier.
/// assert_eq!(chunks[0].start_ms, 800);
/// assert_eq!(chunks[0].duration_ms(), 200 + 500 + 200);
/// ```
#[derive(Debug)]
pub struct Chunker {
    speaker: Speaker,
    config: ChunkerConfig,
    pause_windows: usize,
    padding_windows: usize,
    min_speech_windows: usize,
    max_windows: usize,
    search_windows: usize,
    /// What the noise floor is multiplied by for each speech window.
    floor_growth: f32,
    /// The noise floor never goes below this.
    min_floor: f32,

    anchor: Option<Anchor>,
    /// Samples received that are no longer in `partial`.
    consumed: u64,
    /// Samples waiting for the rest of their window; always fewer than
    /// [`WINDOW_SAMPLES`] between calls.
    partial: Vec<f32>,
    /// Current noise floor, as an RMS level.
    noise_floor: f32,
    /// While no chunk is open: the most recent windows, at most
    /// `padding_windows` of them. None of them is speech.
    preroll: VecDeque<f32>,
    open: Option<OpenChunk>,
    stats: ChunkerStats,
}

impl Chunker {
    /// Creates a chunker for one speaker's stream.
    ///
    /// Settings outside their documented range are brought into it, not
    /// rejected; [`config`](Self::config) returns the result.
    pub fn new(speaker: Speaker, config: ChunkerConfig) -> Self {
        let config = config.normalized();
        let windows = |ms: u32| (ms / WINDOW_MS) as usize;
        let padding_windows = windows(config.padding_ms);
        Chunker {
            speaker,
            pause_windows: windows(config.pause_ms),
            padding_windows,
            min_speech_windows: windows(config.min_speech_ms),
            max_windows: windows(config.max_chunk_ms),
            search_windows: windows(config.cut_search_ms),
            floor_growth: 2.0_f32.powf(WINDOW_MS as f32 / config.noise_doubling_ms as f32),
            min_floor: config.min_speech_level / config.speech_ratio,
            anchor: None,
            consumed: 0,
            partial: Vec::with_capacity(WINDOW_SAMPLES),
            // Start by assuming a room with some noise, so its hum is not
            // taken for speech in the first seconds. A quieter room pulls
            // the floor down within a few windows.
            noise_floor: config.min_speech_level,
            preroll: VecDeque::with_capacity((padding_windows + 1) * WINDOW_SAMPLES),
            open: None,
            stats: ChunkerStats::default(),
            config,
        }
    }

    /// The speaker label put on every chunk.
    pub fn speaker(&self) -> Speaker {
        self.speaker
    }

    /// The settings in use, after rounding and range checks.
    pub fn config(&self) -> &ChunkerConfig {
        &self.config
    }

    /// Counts of chunks returned, chunks dropped, forced cuts and timestamp
    /// gaps so far.
    pub fn stats(&self) -> ChunkerStats {
        self.stats
    }

    /// The current noise floor as an RMS level. A window at
    /// [`speech_ratio`](ChunkerConfig::speech_ratio) times this, or louder,
    /// counts as speech.
    pub fn noise_floor(&self) -> f32 {
        self.noise_floor
    }

    /// How many samples the chunker is holding: the open chunk, or the
    /// padding kept in case speech starts.
    pub fn buffered_samples(&self) -> usize {
        self.partial.len()
            + self.preroll.len()
            + self.open.as_ref().map_or(0, |chunk| chunk.samples.len())
    }

    /// Adds a frame and returns the chunks it completed, oldest first.
    /// Usually that is none.
    ///
    /// The frame's speaker should be this chunker's; chunks are labeled with
    /// the chunker's speaker either way (debug builds assert that they
    /// match). See [`push_samples`](Self::push_samples) for how `start_ms`
    /// is used.
    pub fn push(&mut self, frame: &AudioFrame) -> Vec<Chunk> {
        debug_assert_eq!(
            frame.speaker, self.speaker,
            "a chunker serves one speaker's stream"
        );
        self.push_samples(frame.start_ms, &frame.samples)
    }

    /// Adds samples whose first one was captured at meeting time `start_ms`,
    /// and returns the chunks they completed, oldest first.
    ///
    /// When `start_ms` is more than
    /// [`max_gap_ms`](ChunkerConfig::max_gap_ms) away from where the previous
    /// samples ended, in either direction, the stream has a break: the chunk
    /// in progress ends at the break and a new one can start after it. The
    /// meeting clock is re-read from `start_ms` on every call, so chunk times
    /// do not drift over a long recording.
    pub fn push_samples(&mut self, start_ms: u64, samples: &[f32]) -> Vec<Chunk> {
        let mut out = Vec::new();
        if samples.is_empty() {
            return out;
        }

        let received = self.consumed + self.partial.len() as u64;
        if self.anchor.is_some() {
            let expected_ms = self.time_of(received);
            if start_ms.abs_diff(expected_ms) > u64::from(self.config.max_gap_ms) {
                self.stats.gaps += 1;
                out.extend(self.finish_stream());
            }
        }
        // `finish_stream` moves `consumed` past the samples it discards.
        self.anchor = Some(Anchor {
            ms: start_ms,
            index: self.consumed + self.partial.len() as u64,
        });

        let mut rest = samples;
        if !self.partial.is_empty() {
            let take = (WINDOW_SAMPLES - self.partial.len()).min(rest.len());
            self.partial.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.partial.len() == WINDOW_SAMPLES {
                let mut window = std::mem::take(&mut self.partial);
                self.process_window(&window, &mut out);
                window.clear();
                self.partial = window;
            }
        }
        while rest.len() >= WINDOW_SAMPLES {
            let (window, after) = rest.split_at(WINDOW_SAMPLES);
            self.process_window(window, &mut out);
            rest = after;
        }
        self.partial.extend_from_slice(rest);
        out
    }

    /// Ends the stream and returns the chunk in progress, if it holds enough
    /// speech. Call it when the recording stops.
    ///
    /// The chunker can be used again afterwards; it keeps its noise floor
    /// and treats the next frame as the start of a new stream.
    pub fn flush(&mut self) -> Option<Chunk> {
        self.finish_stream()
    }

    /// Meeting time of the sample with this count, by the latest anchor.
    fn time_of(&self, index: u64) -> u64 {
        let Some(anchor) = self.anchor else {
            return 0;
        };
        if index >= anchor.index {
            anchor
                .ms
                .saturating_add((index - anchor.index) / SAMPLES_PER_MS)
        } else {
            anchor
                .ms
                .saturating_sub((anchor.index - index).div_ceil(SAMPLES_PER_MS))
        }
    }

    /// Classifies one whole window and moves the chunk state along.
    fn process_window(&mut self, window: &[f32], out: &mut Vec<Chunk>) {
        debug_assert_eq!(window.len(), WINDOW_SAMPLES);
        let window_index = self.consumed;
        self.consumed += WINDOW_SAMPLES as u64;

        let energy = window_energy(window);
        let level = energy.sqrt();
        let threshold =
            (self.noise_floor * self.config.speech_ratio).max(self.config.min_speech_level);
        let speech = level >= threshold;
        self.track_noise(level, speech);

        let Some(chunk) = self.open.as_mut() else {
            if speech {
                self.open_chunk(window, energy, window_index);
            } else {
                self.preroll.extend(window);
                let limit = self.padding_windows * WINDOW_SAMPLES;
                if self.preroll.len() > limit {
                    self.preroll.drain(..self.preroll.len() - limit);
                }
            }
            return;
        };

        chunk.samples.extend_from_slice(window);
        chunk.windows.push(Window { energy, speech });
        chunk.silence_run = if speech { 0 } else { chunk.silence_run + 1 };

        if chunk.silence_run >= self.pause_windows {
            self.close_at_pause(out);
        } else if chunk.windows.len() >= self.max_windows {
            self.force_cut(out);
        }
    }

    /// Moves the noise floor towards the level of a window.
    fn track_noise(&mut self, level: f32, speech: bool) {
        let floor = self.noise_floor;
        let moved = if speech {
            floor * self.floor_growth
        } else if level < floor {
            floor + (level - floor) * FLOOR_FALL
        } else {
            floor + (level - floor) * FLOOR_RISE
        };
        self.noise_floor = moved.max(self.min_floor);
    }

    /// Starts a chunk at a speech window, reaching back over the pre-roll.
    fn open_chunk(&mut self, window: &[f32], energy: f32, window_index: u64) {
        let preroll_samples = self.preroll.len();
        let start_ms = self.time_of(window_index - preroll_samples as u64);

        let mut samples = Vec::with_capacity(preroll_samples + 64 * WINDOW_SAMPLES);
        samples.extend(self.preroll.drain(..));
        let (quiet_windows, _) = samples.as_chunks::<WINDOW_SAMPLES>();
        let mut windows: Vec<Window> = quiet_windows
            .iter()
            .map(|quiet| Window {
                energy: window_energy(quiet),
                speech: false,
            })
            .collect();
        samples.extend_from_slice(window);
        windows.push(Window {
            energy,
            speech: true,
        });

        self.open = Some(OpenChunk {
            start_ms,
            samples,
            windows,
            silence_run: 0,
        });
    }

    /// Ends the open chunk after a pause. The silence past the padding is
    /// not part of the chunk; its end becomes the pre-roll of the next one.
    fn close_at_pause(&mut self, out: &mut Vec<Chunk>) {
        let Some(chunk) = self.open.take() else {
            return;
        };
        let total = chunk.windows.len();
        let kept = total - chunk.silence_run + chunk.silence_run.min(self.padding_windows);
        let preroll_windows = (total - kept).min(self.padding_windows);
        self.preroll.clear();
        self.preroll
            .extend(&chunk.samples[(total - preroll_windows) * WINDOW_SAMPLES..]);
        self.emit(chunk.start_ms, chunk.samples, &chunk.windows, out);
    }

    /// Cuts a chunk that reached the maximum length at the quietest point of
    /// its last stretch. What follows the cut starts the next chunk.
    fn force_cut(&mut self, out: &mut Vec<Chunk>) {
        let Some(chunk) = self.open.take() else {
            return;
        };
        self.stats.forced_cuts += 1;
        let total = chunk.windows.len();
        let first_candidate = total.saturating_sub(self.search_windows).max(1);
        let cut = quietest_boundary(&chunk.windows, first_candidate);

        let OpenChunk {
            start_ms,
            mut samples,
            windows,
            ..
        } = chunk;
        let rest_samples = samples.split_off(cut * WINDOW_SAMPLES);
        let (head, rest) = windows.split_at(cut);
        self.emit(start_ms, samples, head, out);

        let rest_index = self.consumed - rest_samples.len() as u64;
        match rest.iter().position(|window| window.speech) {
            // Only quiet audio after the cut: wait for speech, keeping the
            // end of it as pre-roll.
            None => {
                let keep = rest.len().min(self.padding_windows);
                self.preroll.clear();
                self.preroll
                    .extend(&rest_samples[(rest.len() - keep) * WINDOW_SAMPLES..]);
            }
            // The next chunk is already under way. Like any chunk it starts
            // no more than the padding before its first speech.
            Some(first_speech) => {
                let skip = first_speech.saturating_sub(self.padding_windows);
                let windows = rest[skip..].to_vec();
                let silence_run = windows.iter().rev().take_while(|w| !w.speech).count();
                self.open = Some(OpenChunk {
                    start_ms: self.time_of(rest_index + (skip * WINDOW_SAMPLES) as u64),
                    samples: rest_samples[skip * WINDOW_SAMPLES..].to_vec(),
                    windows,
                    silence_run,
                });
            }
        }
    }

    /// Ends the stream at a flush or a timestamp gap: returns the open chunk
    /// if it holds enough speech and forgets everything buffered.
    fn finish_stream(&mut self) -> Option<Chunk> {
        let mut out = Vec::new();
        if let Some(mut chunk) = self.open.take() {
            // Samples short of a whole window belong to the chunk when it
            // runs to the end of the stream.
            if chunk.silence_run <= self.padding_windows {
                chunk.samples.extend_from_slice(&self.partial);
            }
            self.emit(chunk.start_ms, chunk.samples, &chunk.windows, &mut out);
        }
        self.consumed += self.partial.len() as u64;
        self.partial.clear();
        self.preroll.clear();
        self.anchor = None;
        out.pop()
    }

    /// Returns a finished chunk, or drops it for holding too little speech.
    ///
    /// `windows` describes the whole windows at the start of `samples`.
    /// Silence at the end beyond the padding is cut off.
    fn emit(
        &mut self,
        start_ms: u64,
        mut samples: Vec<f32>,
        windows: &[Window],
        out: &mut Vec<Chunk>,
    ) {
        let speech_windows = windows.iter().filter(|window| window.speech).count();
        let last_speech = windows.iter().rposition(|window| window.speech);
        let Some(last_speech) = last_speech.filter(|_| speech_windows >= self.min_speech_windows)
        else {
            self.stats.dropped += 1;
            return;
        };
        let kept = last_speech + 1 + self.padding_windows;
        if kept < windows.len() {
            samples.truncate(kept * WINDOW_SAMPLES);
        }
        // Chunks can wait in a queue for the transcriber; do not let each
        // one hold spare capacity while it does.
        samples.shrink_to_fit();
        self.stats.chunks += 1;
        out.push(Chunk {
            speaker: self.speaker,
            start_ms,
            samples,
        });
    }
}

/// Mean square of a window around its own mean, so a constant offset from
/// the input device does not read as sound. Zero if a sample is not a number.
fn window_energy(window: &[f32]) -> f32 {
    if window.is_empty() {
        return 0.0;
    }
    let count = window.len() as f32;
    let mean = window.iter().sum::<f32>() / count;
    let energy = window
        .iter()
        .map(|sample| (sample - mean) * (sample - mean))
        .sum::<f32>()
        / count;
    if energy.is_finite() { energy } else { 0.0 }
}

/// The window boundary in `first_candidate..=windows.len()` with the least
/// sound around it; a cut there puts `windows[..boundary]` in the first part.
///
/// Each boundary is scored by the energy of the [`CUT_REACH_WINDOWS`] windows
/// on each side, the nearest weighing most, so the result is the middle of
/// the widest quiet stretch and not merely the single quietest window.
/// Among boundaries that are equally quiet ([`CUT_MARGIN`]) the latest wins,
/// which keeps the first part long.
fn quietest_boundary(windows: &[Window], first_candidate: usize) -> usize {
    let total = windows.len();
    let mut best = total;
    let mut best_score = f32::INFINITY;
    for boundary in (first_candidate..=total).rev() {
        let from = boundary.saturating_sub(CUT_REACH_WINDOWS);
        let to = (boundary + CUT_REACH_WINDOWS).min(total);
        let mut weighted = 0.0;
        let mut weights = 0.0;
        for (index, window) in windows.iter().enumerate().take(to).skip(from) {
            // Distance from the boundary in windows: 0 for the two windows
            // that touch it.
            let distance = if index < boundary {
                boundary - 1 - index
            } else {
                index - boundary
            };
            let weight = (CUT_REACH_WINDOWS - distance) as f32;
            weighted += weight * window.energy;
            weights += weight;
        }
        if weights > 0.0 && weighted / weights < best_score * CUT_MARGIN {
            best = boundary;
            best_score = weighted / weights;
        }
    }
    best
}

#[cfg(test)]
mod tests;
