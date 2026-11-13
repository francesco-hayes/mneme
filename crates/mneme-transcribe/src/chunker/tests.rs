//! Chunker tests on synthetic audio: a tone with some noise stands in for
//! speech, low noise for silence.

use super::*;

/// RMS level of the low noise used as silence: -66 dBFS, well under the
/// default minimum speech level.
const QUIET: f32 = 0.0005;
/// Amplitude of the tone used as speech.
const LOUD: f32 = 0.3;
/// Length in milliseconds of the frames the audio crate delivers.
const FRAME_MS: u64 = 20;
const FRAME: usize = WINDOW_SAMPLES;

fn samples_in(ms: u64) -> usize {
    (ms * SAMPLES_PER_MS) as usize
}

/// A small deterministic random generator (xorshift), so the tests need no
/// `rand` and give the same audio on every run.
struct Rng(u32);

impl Rng {
    /// Uniform in `-1.0..1.0`.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
    }
}

/// Builds test audio piece by piece.
struct Tape {
    samples: Vec<f32>,
    rng: Rng,
}

impl Tape {
    fn new() -> Self {
        Tape {
            samples: Vec::new(),
            rng: Rng(0x9E37_79B9),
        }
    }

    /// Length so far, in milliseconds.
    fn ms(&self) -> u64 {
        self.samples.len() as u64 / SAMPLES_PER_MS
    }

    /// White noise at the given RMS level.
    fn noise(mut self, ms: u64, rms: f32) -> Self {
        // Uniform noise in -a..a has an RMS of a / sqrt(3).
        let amplitude = rms * 3.0_f32.sqrt();
        for _ in 0..samples_in(ms) {
            let sample = amplitude * self.rng.next();
            self.samples.push(sample);
        }
        self
    }

    /// "Silence": low noise.
    fn quiet(self, ms: u64) -> Self {
        self.noise(ms, QUIET)
    }

    /// Digital silence.
    fn zeros(mut self, ms: u64) -> Self {
        self.samples
            .resize(self.samples.len() + samples_in(ms), 0.0);
        self
    }

    /// "Speech": a 220 Hz tone of the given amplitude plus a little noise.
    fn tone(mut self, ms: u64, amplitude: f32) -> Self {
        let step = std::f32::consts::TAU * 220.0 / SAMPLE_RATE as f32;
        for i in 0..samples_in(ms) {
            let sample = amplitude * (i as f32 * step).sin() + amplitude * 0.1 * self.rng.next();
            self.samples.push(sample);
        }
        self
    }

    /// "Speech" at the usual level.
    fn burst(self, ms: u64) -> Self {
        self.tone(ms, LOUD)
    }

    /// Unbroken "speech": bursts of `on_ms` separated by dips of `off_ms`,
    /// repeated to fill `ms`.
    fn syllables(mut self, ms: u64, on_ms: u64, off_ms: u64) -> Self {
        let end = self.ms() + ms;
        while self.ms() < end {
            let on = on_ms.min(end - self.ms());
            self = self.burst(on);
            let off = off_ms.min(end - self.ms());
            self = self.quiet(off);
        }
        self
    }

    /// Adds a constant offset to everything so far.
    fn offset(mut self, by: f32) -> Self {
        for sample in &mut self.samples {
            *sample += by;
        }
        self
    }

    /// The samples from `from_ms` to `to_ms`.
    fn slice(&self, from_ms: u64, to_ms: u64) -> &[f32] {
        &self.samples[samples_in(from_ms)..samples_in(to_ms)]
    }
}

fn chunker() -> Chunker {
    Chunker::new(Speaker::Me, ChunkerConfig::default())
}

/// Pushes `samples` in frames of `frame` samples, the first one stamped
/// `start_ms` and the rest by sample count, the way a source stamps them.
fn feed_at(chunker: &mut Chunker, samples: &[f32], frame: usize, start_ms: u64) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    for (n, part) in samples.chunks(frame).enumerate() {
        let at = start_ms + (n * frame) as u64 / SAMPLES_PER_MS;
        chunks.extend(chunker.push_samples(at, part));
    }
    chunks
}

fn feed(chunker: &mut Chunker, samples: &[f32]) -> Vec<Chunk> {
    feed_at(chunker, samples, FRAME, 0)
}

/// Runs a whole tape through a default chunker, flush included.
fn run(tape: &Tape) -> Vec<Chunk> {
    let mut chunker = chunker();
    let mut chunks = feed(&mut chunker, &tape.samples);
    chunks.extend(chunker.flush());
    chunks
}

/// Start and end of each chunk, in milliseconds.
fn spans(chunks: &[Chunk]) -> Vec<(u64, u64)> {
    chunks.iter().map(|c| (c.start_ms, c.end_ms())).collect()
}

/// The chunk holds exactly the tape's samples for its time range: nothing
/// moved, repeated or altered.
fn assert_cut_from(chunk: &Chunk, tape: &Tape) {
    let from = samples_in(chunk.start_ms);
    let to = from + chunk.samples.len();
    assert!(to <= tape.samples.len(), "chunk runs past the tape");
    assert!(
        chunk.samples == tape.samples[from..to],
        "chunk at {} ms does not match the tape there",
        chunk.start_ms
    );
}

// ---------------------------------------------------------------- pauses

#[test]
fn each_burst_becomes_one_chunk_that_starts_where_the_burst_began() {
    let tape = Tape::new()
        .quiet(1000)
        .burst(1000) // 1000..2000
        .quiet(1500)
        .burst(600) // 3500..4100
        .quiet(1000);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    // Each chunk starts and ends one padding (200 ms) around its burst.
    assert_eq!(spans(&chunks), [(800, 2200), (3300, 4300)]);
    for chunk in &chunks {
        assert_eq!(chunk.speaker, Speaker::Me);
        assert_cut_from(chunk, &tape);
    }
    // Both pauses had passed, so nothing was left for the flush.
    assert_eq!(chunker.flush(), None);
    assert_eq!(
        chunker.stats(),
        ChunkerStats {
            chunks: 2,
            ..ChunkerStats::default()
        }
    );
}

#[test]
fn a_chunk_is_returned_as_soon_as_the_pause_has_passed() {
    let tape = Tape::new().quiet(1000).burst(1000).quiet(1000);
    let mut chunker = chunker();

    // The burst ends at 2000 ms; the pause is 700 ms.
    let early = feed(&mut chunker, tape.slice(0, 2680));
    assert_eq!(early, []);
    let on_time = feed_at(&mut chunker, tape.slice(2680, 2700), FRAME, 2680);
    assert_eq!(spans(&on_time), [(800, 2200)]);
}

#[test]
fn start_follows_a_burst_that_begins_inside_a_window() {
    for begins in [1000, 1001, 1007, 1013, 1019, 1020, 1033] {
        let tape = Tape::new().quiet(begins).burst(500).quiet(1000);

        let chunks = run(&tape);

        assert_eq!(chunks.len(), 1, "burst at {begins} ms");
        let start = chunks[0].start_ms;
        // One padding before the burst, to within one 20 ms window.
        assert!(
            start <= begins - 200 && start + 20 > begins - 200,
            "burst at {begins} ms gave a chunk at {start} ms"
        );
        let end = chunks[0].end_ms();
        assert!(
            end >= begins + 700 && end < begins + 720,
            "burst at {begins} ms gave a chunk ending at {end} ms"
        );
        assert_cut_from(&chunks[0], &tape);
    }
}

#[test]
fn speech_at_the_very_start_begins_a_chunk_at_the_first_frame() {
    let tape = Tape::new().burst(500).quiet(1000);

    assert_eq!(spans(&run(&tape)), [(0, 700)]);

    // The same on a clock that does not start at zero.
    let mut chunker = chunker();
    let chunks = feed_at(&mut chunker, &tape.samples, FRAME, 250);
    assert_eq!(spans(&chunks), [(250, 950)]);
}

#[test]
fn start_ms_is_on_the_frames_clock() {
    let tape = Tape::new().quiet(1000).burst(500).quiet(1000);
    let mut chunker = chunker();

    let chunks = feed_at(&mut chunker, &tape.samples, FRAME, 3_600_000);

    assert_eq!(spans(&chunks), [(3_600_800, 3_601_700)]);
}

#[test]
fn a_gap_shorter_than_the_pause_stays_inside_one_chunk() {
    // 680 ms is one window short of the 700 ms pause.
    let tape = Tape::new()
        .quiet(1000)
        .burst(500)
        .quiet(680)
        .burst(500)
        .quiet(1000);

    let chunks = run(&tape);

    assert_eq!(spans(&chunks), [(800, 2880)]);
    assert_cut_from(&chunks[0], &tape);
}

#[test]
fn a_gap_as_long_as_the_pause_splits() {
    let tape = Tape::new()
        .quiet(1000)
        .burst(500)
        .quiet(700)
        .burst(500)
        .quiet(1000);

    let chunks = run(&tape);

    assert_eq!(spans(&chunks), [(800, 1700), (2000, 2900)]);
    for chunk in &chunks {
        assert_cut_from(chunk, &tape);
    }
}

#[test]
fn the_pause_length_is_configurable() {
    let tape = Tape::new()
        .quiet(1000)
        .burst(500)
        .quiet(400)
        .burst(500)
        .quiet(1000);
    let config = ChunkerConfig {
        pause_ms: 300,
        padding_ms: 100,
        ..ChunkerConfig::default()
    };
    let mut chunker = Chunker::new(Speaker::Them, config);

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(spans(&chunks), [(900, 1600), (1800, 2500)]);
    assert!(chunks.iter().all(|chunk| chunk.speaker == Speaker::Them));
}

#[test]
fn padding_never_repeats_audio_when_the_pause_is_short() {
    // Padding either side (2 x 200 ms) is more than the 300 ms between the
    // chunks, so the second chunk's lead-in is cut short where the first
    // ended.
    let tape = Tape::new()
        .quiet(1000)
        .burst(500)
        .quiet(300)
        .burst(500)
        .quiet(1000);
    let config = ChunkerConfig {
        pause_ms: 300,
        ..ChunkerConfig::default()
    };
    let mut chunker = Chunker::new(Speaker::Me, config);

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(spans(&chunks), [(800, 1700), (1700, 2500)]);
    for chunk in &chunks {
        assert_cut_from(chunk, &tape);
    }
}

// --------------------------------------------------------------- silence

#[test]
fn pure_silence_yields_nothing() {
    for tape in [
        Tape::new().zeros(20_000),
        Tape::new().quiet(20_000),
        Tape::new().quiet(5_000).zeros(5_000).quiet(5_000),
    ] {
        let mut chunker = chunker();

        let chunks = feed(&mut chunker, &tape.samples);

        assert_eq!(chunks, []);
        assert_eq!(chunker.flush(), None);
        assert_eq!(chunker.stats(), ChunkerStats::default());
    }
}

#[test]
fn a_click_is_dropped() {
    let tape = Tape::new().quiet(1000).burst(60).quiet(2000);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(chunks, []);
    assert_eq!(chunker.flush(), None);
    assert_eq!(
        chunker.stats(),
        ChunkerStats {
            dropped: 1,
            ..ChunkerStats::default()
        }
    );
}

#[test]
fn the_minimum_speech_rule_is_exact() {
    // The default minimum is 160 ms.
    let short = Tape::new().quiet(1000).burst(140).quiet(2000);
    assert_eq!(run(&short), []);

    let enough = Tape::new().quiet(1000).burst(160).quiet(2000);
    assert_eq!(spans(&run(&enough)), [(800, 1360)]);
}

#[test]
fn clicks_do_not_add_up_to_speech_across_chunks() {
    // Three clicks, each alone in its own would-be chunk.
    let tape = Tape::new()
        .quiet(1000)
        .burst(80)
        .quiet(1000)
        .burst(80)
        .quiet(1000)
        .burst(80)
        .quiet(1000);
    let mut chunker = chunker();

    assert_eq!(feed(&mut chunker, &tape.samples), []);
    assert_eq!(chunker.stats().dropped, 3);
}

#[test]
fn sound_under_the_minimum_level_is_never_speech() {
    // Digital silence around it, so the noise floor is as low as it goes
    // and only the minimum speech level (0.002) decides.
    let faint = Tape::new().zeros(2_000).tone(1_000, 0.0024).zeros(2_000);
    let level = window_energy(faint.slice(2_500, 2_520)).sqrt();
    assert!((0.0015..0.002).contains(&level), "level is {level}");
    assert_eq!(run(&faint), []);

    let audible = Tape::new().zeros(2_000).tone(1_000, 0.004).zeros(2_000);
    let level = window_energy(audible.slice(2_500, 2_520)).sqrt();
    assert!((0.002..0.003).contains(&level), "level is {level}");
    assert_eq!(spans(&run(&audible)), [(1_800, 3_200)]);
}

#[test]
fn a_constant_offset_is_not_sound() {
    // A device with a large DC offset: quiet is still quiet.
    let silent = Tape::new().quiet(10_000).offset(0.2);
    assert_eq!(run(&silent), []);

    let tape = Tape::new().quiet(1000).burst(500).quiet(1000).offset(0.2);
    assert_eq!(spans(&run(&tape)), [(800, 1700)]);
}

#[test]
fn samples_that_are_not_numbers_count_as_silence() {
    let mut tape = Tape::new().quiet(1000).burst(500).quiet(1000);
    for sample in &mut tape.samples[samples_in(200)..samples_in(400)] {
        *sample = f32::NAN;
    }
    tape.samples[samples_in(500)] = f32::INFINITY;
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(spans(&chunks), [(800, 1700)]);
    assert!(chunker.noise_floor().is_finite());
}

// ----------------------------------------------------------- noise floor

#[test]
fn a_noisy_room_from_the_start_is_not_speech() {
    // Steady noise at twice the minimum speech level, for a minute.
    let tape = Tape::new().noise(60_000, 0.004);
    let mut chunker = chunker();

    assert_eq!(feed(&mut chunker, &tape.samples), []);
    assert_eq!(chunker.flush(), None);
    // The floor settled on the noise.
    let floor = chunker.noise_floor();
    assert!((0.0035..0.0045).contains(&floor), "floor is {floor}");
}

#[test]
fn noise_that_starts_suddenly_stops_counting_as_speech() {
    // A quiet room, then a fan ten times louder than the speech threshold.
    let tape = Tape::new().quiet(2_000).noise(60_000, 0.02);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    // The detector cannot know at once that the new sound is noise: it
    // takes it for speech until the floor has caught up, then goes quiet.
    assert_eq!(chunks.len(), 1, "{:?}", spans(&chunks));
    assert_eq!(chunks[0].start_ms, 1800);
    assert!(
        chunks[0].duration_ms() < 10_000,
        "the noise counted as speech for {} ms",
        chunks[0].duration_ms()
    );
    assert_eq!(chunker.flush(), None);
    let floor = chunker.noise_floor();
    assert!((0.018..0.022).contains(&floor), "floor is {floor}");
}

#[test]
fn speech_is_still_found_over_steady_noise() {
    let room = 0.02;
    let tape = Tape::new()
        .noise(20_000, room)
        .burst(1000) // 20_000..21_000
        .noise(2_000, room)
        .burst(1000) // 23_000..24_000
        .noise(2_000, room);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    // Whatever the first seconds of noise produced, the room has settled
    // long before the bursts.
    let late: Vec<_> = chunks.iter().filter(|c| c.start_ms > 15_000).collect();
    assert_eq!(late.len(), 2, "{:?}", spans(&chunks));
    assert_eq!((late[0].start_ms, late[0].end_ms()), (19_800, 21_200));
    assert_eq!((late[1].start_ms, late[1].end_ms()), (22_800, 24_200));
}

#[test]
fn the_floor_comes_back_down_when_the_room_goes_quiet() {
    // Loud noise, then a quiet room and speech far quieter than that noise.
    let tape = Tape::new()
        .noise(30_000, 0.05)
        .quiet(1_000)
        .tone(1_000, 0.02) // 31_000..32_000
        .quiet(1_000);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    let last = chunks.last().expect("the quiet speech should be found");
    assert_eq!((last.start_ms, last.end_ms()), (30_800, 32_200));
}

#[test]
fn quiet_speech_after_long_loud_speech_is_found() {
    // Twenty seconds of loud talk must not leave the floor too high to
    // hear someone a good deal quieter.
    let tape = Tape::new()
        .quiet(1_000)
        .syllables(20_000, 180, 60)
        .quiet(1_500)
        .tone(1_000, 0.01) // 22_500..23_500
        .quiet(1_000);

    let chunks = run(&tape);

    let last = chunks.last().unwrap();
    assert_eq!((last.start_ms, last.end_ms()), (22_300, 23_700));
}

// ------------------------------------------------------------ forced cuts

#[test]
fn a_long_unbroken_burst_is_cut_under_the_maximum() {
    // 70 s of "speech" with no gap longer than 60 ms.
    let tape = Tape::new()
        .quiet(1_000)
        .syllables(70_080, 180, 60)
        .quiet(1_000);
    let mut chunker = chunker();

    let mut chunks = feed(&mut chunker, &tape.samples);
    chunks.extend(chunker.flush());

    assert_eq!(chunker.stats().forced_cuts, 2);
    assert_eq!(chunks.len(), 3, "{:?}", spans(&chunks));
    for chunk in &chunks {
        assert!(chunk.duration_ms() <= 28_000, "{:?}", spans(&chunks));
        assert_cut_from(chunk, &tape);
    }
    // A forced chunk is cut in its last 8 s, so it is at least 20 s long.
    assert!(chunks[0].duration_ms() >= 20_000);
    assert!(chunks[1].duration_ms() >= 20_000);
    // Nothing is lost or repeated at a forced cut.
    assert_eq!(chunks[0].start_ms, 800);
    assert_eq!(chunks[1].start_ms, chunks[0].end_ms());
    assert_eq!(chunks[2].start_ms, chunks[1].end_ms());
    // The last syllable ends at 71_020; its dip and the padding follow.
    assert_eq!(chunks[2].end_ms(), 71_220);
    // Each cut fell in a dip between syllables (the last 60 ms of every
    // 240 ms), not inside a syllable.
    for cut in [chunks[0].end_ms(), chunks[1].end_ms()] {
        let phase = (cut - 1_000) % 240;
        assert!(
            (180..240).contains(&phase) || phase == 0,
            "cut at {cut} ms is {phase} ms into a syllable"
        );
    }
}

#[test]
fn a_forced_cut_goes_to_the_longest_gap_in_the_last_stretch() {
    // The chunk opens at 800 ms and must be cut by 28_800 ms. Its last
    // stretch holds many 60 ms dips and one gap of 360 ms, from 24_940 to
    // 25_300 ms.
    let tape = Tape::new()
        .quiet(1_000)
        .syllables(24_000, 180, 60) // ends, after a dip, at 25_000
        .quiet(300)
        .syllables(20_040, 180, 60)
        .quiet(1_000);
    let mut chunker = chunker();

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(chunks.len(), 2, "{:?}", spans(&chunks));
    assert_eq!(chunker.stats().forced_cuts, 1);
    // The cut is inside the gap, at least 100 ms from the speech either
    // side, and each chunk keeps at most the padding of the gap.
    let first_end = chunks[0].end_ms();
    let second_start = chunks[1].start_ms;
    assert!((25_040..=25_140).contains(&first_end), "{first_end}");
    assert!((25_100..=25_200).contains(&second_start), "{second_start}");
    assert!(second_start >= first_end);
    for chunk in &chunks {
        assert_cut_from(chunk, &tape);
    }
}

#[test]
fn a_steady_tone_is_cut_at_the_limit() {
    // No dips at all: the cut can only be at the limit. A short doubling
    // time would turn a steady tone into "noise" first, so switch that off.
    let config = ChunkerConfig {
        max_chunk_ms: 2_000,
        cut_search_ms: 500,
        noise_doubling_ms: u32::MAX,
        ..ChunkerConfig::default()
    };
    let mut tape = Tape::new().quiet(1_000);
    let step = std::f32::consts::TAU * 200.0 / SAMPLE_RATE as f32;
    tape.samples
        .extend((0..samples_in(5_000)).map(|i| LOUD * (i as f32 * step).sin()));
    let tape = tape.quiet(1_000);
    let mut chunker = Chunker::new(Speaker::Me, config);

    let mut chunks = feed(&mut chunker, &tape.samples);
    chunks.extend(chunker.flush());

    assert_eq!(
        spans(&chunks),
        [(800, 2_800), (2_800, 4_800), (4_800, 6_200)]
    );
    for chunk in &chunks {
        assert_cut_from(chunk, &tape);
    }
}

#[test]
fn a_cut_just_before_silence_leaves_no_silent_chunk() {
    // Speech stops 420 ms before the limit, so the limit (800 + 2000) is
    // reached before the pause (2380 + 700). The cut falls in the silence,
    // and what follows the cut holds no speech.
    let config = ChunkerConfig {
        max_chunk_ms: 2_000,
        cut_search_ms: 1_000,
        ..ChunkerConfig::default()
    };
    let tape = Tape::new()
        .quiet(1_000)
        .syllables(1_440, 180, 60) // last syllable ends at 2_380
        .quiet(3_000)
        .burst(500) // 5_440..5_940
        .quiet(1_000);
    let mut chunker = Chunker::new(Speaker::Me, config);

    let chunks = feed(&mut chunker, &tape.samples);

    assert_eq!(chunker.stats().forced_cuts, 1);
    assert_eq!(chunker.stats().dropped, 0);
    assert_eq!(chunks.len(), 2, "{:?}", spans(&chunks));
    // The first chunk ends in the silence, at least 100 ms after the speech
    // and with no more than the padding.
    let first_end = chunks[0].end_ms();
    assert_eq!(chunks[0].start_ms, 800);
    assert!((2_480..=2_580).contains(&first_end), "{first_end}");
    // The second is a normal chunk around the burst.
    assert_eq!((chunks[1].start_ms, chunks[1].end_ms()), (5_240, 6_140));
    for chunk in &chunks {
        assert_cut_from(chunk, &tape);
    }
}

#[test]
fn quietest_boundary_finds_the_middle_of_the_widest_gap() {
    let loud = Window {
        energy: 0.04,
        speech: true,
    };
    let soft = Window {
        energy: 0.000_001,
        speech: false,
    };
    let mut windows = vec![loud; 100];
    // A two-window dip and a six-window gap.
    windows[30..32].fill(soft);
    windows[60..66].fill(soft);

    assert_eq!(quietest_boundary(&windows, 1), 63);
    // Only the dip is in reach.
    assert_eq!(quietest_boundary(&windows[..50], 1), 31);
    // Nothing quiet in reach: the latest boundary.
    assert_eq!(quietest_boundary(&windows[..20], 1), 20);
    // The search never goes before the first candidate.
    assert_eq!(quietest_boundary(&windows, 80), 100);
    // Audio that ends in quiet is cut in that quiet.
    assert_eq!(quietest_boundary(&windows[..64], 1), 64);
}

// ------------------------------------------------------------------ flush

#[test]
fn flush_returns_the_tail() {
    // The recording stops in the middle of a burst, 7 ms into a window.
    let tape = Tape::new().quiet(500).burst(1_007);
    let mut chunker = chunker();

    assert_eq!(feed(&mut chunker, &tape.samples), []);
    let tail = chunker.flush().expect("the open chunk");

    assert_eq!(tail.start_ms, 300);
    assert_eq!(tail.samples.len(), samples_in(1_207));
    assert_cut_from(&tail, &tape);
    assert_eq!(chunker.flush(), None);
    assert_eq!(chunker.buffered_samples(), 0);
}

#[test]
fn flush_during_the_pause_keeps_only_the_padding() {
    // Speech ended 500 ms ago: not yet a pause, so the chunk is still open.
    let tape = Tape::new().quiet(500).burst(1_000).quiet(500);
    let mut chunker = chunker();

    assert_eq!(feed(&mut chunker, &tape.samples), []);
    let tail = chunker.flush().expect("the open chunk");

    assert_eq!((tail.start_ms, tail.end_ms()), (300, 1_700));
    assert_cut_from(&tail, &tape);
}

#[test]
fn flush_drops_a_tail_with_too_little_speech() {
    let tape = Tape::new().quiet(500).burst(100);
    let mut chunker = chunker();

    assert_eq!(feed(&mut chunker, &tape.samples), []);
    assert_eq!(chunker.flush(), None);
    assert_eq!(chunker.stats().dropped, 1);
}

#[test]
fn flush_with_nothing_pending_returns_nothing() {
    let mut chunker = chunker();
    assert_eq!(chunker.flush(), None);

    let tape = Tape::new().quiet(3_000);
    assert_eq!(feed(&mut chunker, &tape.samples), []);
    assert_eq!(chunker.flush(), None);
    assert_eq!(chunker.buffered_samples(), 0);
}

#[test]
fn the_chunker_can_be_used_again_after_a_flush() {
    let tape = Tape::new().quiet(500).burst(500).quiet(1_000);
    let mut chunker = chunker();

    let mut first = feed(&mut chunker, tape.slice(0, 800));
    first.extend(chunker.flush());
    assert_eq!(spans(&first), [(300, 800)]);

    // A new stream on a new clock.
    let second = feed_at(&mut chunker, &tape.samples, FRAME, 60_000);
    assert_eq!(spans(&second), [(60_300, 61_200)]);
}

// ------------------------------------------------------------------- gaps

#[test]
fn a_timestamp_gap_splits_a_chunk() {
    // Two seconds of unbroken speech, but the source restarted in the
    // middle: the second half is stamped five seconds later.
    let tape = Tape::new().burst(2_000).quiet(1_000);
    let mut chunker = chunker();

    let before = feed_at(&mut chunker, tape.slice(0, 1_000), FRAME, 10_000);
    assert_eq!(before, []);
    let after = feed_at(&mut chunker, tape.slice(1_000, 3_000), FRAME, 16_000);

    assert_eq!(spans(&after), [(10_000, 11_000), (16_000, 17_200)]);
    assert_eq!(after[0].samples, tape.slice(0, 1_000));
    assert_eq!(after[1].samples, tape.slice(1_000, 2_200));
    assert_eq!(chunker.stats().gaps, 1);
}

#[test]
fn the_chunk_before_a_gap_is_returned_when_the_gap_is_seen() {
    let tape = Tape::new().burst(1_000);
    let mut chunker = chunker();
    assert_eq!(feed_at(&mut chunker, &tape.samples, FRAME, 0), []);

    // One quiet frame from after the gap is enough.
    let quiet = Tape::new().quiet(20);
    let chunks = chunker.push_samples(30_000, &quiet.samples);

    assert_eq!(spans(&chunks), [(0, 1_000)]);
}

#[test]
fn a_clock_that_jumps_back_splits_too() {
    let tape = Tape::new().burst(2_000).quiet(1_000);
    let mut chunker = chunker();

    let mut chunks = feed_at(&mut chunker, tape.slice(0, 1_000), FRAME, 10_000);
    chunks.extend(feed_at(
        &mut chunker,
        tape.slice(1_000, 3_000),
        FRAME,
        4_000,
    ));

    assert_eq!(spans(&chunks), [(10_000, 11_000), (4_000, 5_200)]);
    assert_eq!(chunker.stats().gaps, 1);
}

#[test]
fn audio_from_before_a_gap_is_not_used_as_padding() {
    // Quiet, a gap, then speech at once: the chunk starts at the first
    // frame after the gap, not 200 ms inside the gap.
    let quiet = Tape::new().quiet(1_000);
    let speech = Tape::new().burst(500).quiet(1_000);
    let mut chunker = chunker();

    let mut chunks = feed_at(&mut chunker, &quiet.samples, FRAME, 0);
    chunks.extend(feed_at(&mut chunker, &speech.samples, FRAME, 5_000));

    assert_eq!(spans(&chunks), [(5_000, 5_700)]);
    assert_eq!(chunks[0].samples, speech.slice(0, 700));
}

#[test]
fn a_short_frame_before_a_gap_is_kept() {
    // The audio crate's last frame before a gap can be short. Here it is
    // 5 ms of speech.
    let tape = Tape::new().burst(1_005);
    let mut chunker = chunker();
    assert_eq!(feed_at(&mut chunker, &tape.samples, FRAME, 0), []);

    let quiet = Tape::new().quiet(20);
    let chunks = chunker.push_samples(9_000, &quiet.samples);

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].samples, tape.samples);
}

#[test]
fn small_timestamp_differences_do_not_split() {
    // Frames 60 ms late (under the 100 ms limit), again and again: the
    // audio is treated as continuous and stays one chunk.
    let tape = Tape::new().quiet(500).burst(1_000).quiet(1_000);
    let mut chunker = chunker();
    let mut chunks = Vec::new();

    for (n, frame) in tape.samples.chunks(FRAME).enumerate() {
        let late = if n >= 30 { 60 * (n as u64 / 10 - 2) } else { 0 };
        chunks.extend(chunker.push_samples(n as u64 * FRAME_MS + late, frame));
    }

    assert_eq!(chunker.stats().gaps, 0);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_ms, 300);
    assert_eq!(chunks[0].samples, tape.slice(300, 1_700));
}

#[test]
fn chunk_times_follow_the_frame_clock_when_it_drifts() {
    // A source whose stamps run 1% ahead of its sample count. After 100 s
    // the two disagree by a second; chunk times must follow the stamps,
    // and the slow drift must not look like a gap.
    let tape = Tape::new().quiet(100_000).burst(500).quiet(1_000);
    let mut chunker = chunker();
    let mut chunks = Vec::new();

    for (n, frame) in tape.samples.chunks(FRAME).enumerate() {
        let stamp = n as u64 * 2_020 / 100;
        chunks.extend(chunker.push_samples(stamp, frame));
    }

    assert_eq!(chunker.stats().gaps, 0);
    assert_eq!(chunks.len(), 1);
    // The burst's first frame was stamped 101_000; padding is 200 ms of
    // samples before it.
    let start = chunks[0].start_ms;
    assert!((100_795..=100_800).contains(&start), "{start}");
}

// ----------------------------------------------------------------- frames

#[test]
fn frame_size_does_not_change_the_chunks() {
    let tape = Tape::new()
        .quiet(1_013)
        .burst(777)
        .quiet(900)
        .syllables(3_000, 150, 50)
        .quiet(1_000)
        .burst(100)
        .quiet(333);
    let reference = run(&tape);
    assert_eq!(reference.len(), 2);

    for frame in [1, 7, 160, 441, 1_024, 16_000, tape.samples.len()] {
        let mut chunker = chunker();
        let mut chunks = feed_at(&mut chunker, &tape.samples, frame, 0);
        chunks.extend(chunker.flush());

        assert_eq!(chunks.len(), reference.len(), "frames of {frame}");
        for (chunk, expected) in chunks.iter().zip(&reference) {
            assert!(
                chunk.samples == expected.samples,
                "frames of {frame}: different samples"
            );
            // Stamps are whole milliseconds, so a frame that does not start
            // on one moves the result by up to a millisecond.
            assert!(
                chunk.start_ms.abs_diff(expected.start_ms) <= 1,
                "frames of {frame}: {} vs {}",
                chunk.start_ms,
                expected.start_ms
            );
        }
    }
}

#[test]
fn push_takes_audio_frames() {
    let tape = Tape::new().quiet(1_000).burst(500).quiet(1_000);
    let mut chunker = Chunker::new(Speaker::Them, ChunkerConfig::default());
    assert_eq!(chunker.speaker(), Speaker::Them);
    let mut chunks = Vec::new();

    for (n, samples) in tape.samples.chunks(FRAME).enumerate() {
        let frame = AudioFrame {
            speaker: Speaker::Them,
            start_ms: n as u64 * FRAME_MS,
            samples: samples.to_vec(),
        };
        chunks.extend(chunker.push(&frame));
    }

    assert_eq!(spans(&chunks), [(800, 1_700)]);
    assert_eq!(chunks[0].speaker, Speaker::Them);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "one speaker's stream")]
fn a_frame_from_the_other_speaker_is_caught_in_debug_builds() {
    let frame = AudioFrame {
        speaker: Speaker::Them,
        start_ms: 0,
        samples: vec![0.0; FRAME],
    };
    chunker().push(&frame);
}

#[test]
fn an_empty_frame_changes_nothing() {
    let tape = Tape::new().burst(1_000).quiet(1_000);
    let mut chunker = chunker();

    let mut chunks = feed(&mut chunker, tape.slice(0, 500));
    // Even one stamped far away: there is no audio in it to place.
    chunks.extend(chunker.push_samples(99_000, &[]));
    chunks.extend(feed_at(&mut chunker, tape.slice(500, 2_000), FRAME, 500));

    assert_eq!(spans(&chunks), [(0, 1_200)]);
    assert_eq!(chunker.stats().gaps, 0);
}

// ----------------------------------------------------------------- memory

/// The most a default chunker may hold while nobody speaks: the padding
/// plus a window in progress.
const IDLE_LIMIT: usize = (200 / WINDOW_MS as usize + 1) * WINDOW_SAMPLES;

#[test]
fn hours_of_silence_do_not_grow_memory() {
    // Three simulated hours of a quiet room, in 20 ms frames.
    let second = Tape::new().quiet(990).zeros(10);
    let mut chunker = chunker();
    let preroll_capacity = chunker.preroll.capacity();
    let partial_capacity = chunker.partial.capacity();
    let mut most = 0;

    for n in 0..3 * 3_600 * 50_u64 {
        let at = (n as usize % 50) * FRAME;
        let frame = &second.samples[at..at + FRAME];
        let chunks = chunker.push_samples(n * FRAME_MS, frame);
        assert!(chunks.is_empty());
        most = most.max(chunker.buffered_samples());
    }

    assert!(most <= IDLE_LIMIT, "held {most} samples");
    assert!(chunker.open.is_none());
    // The buffers are the ones allocated at the start.
    assert_eq!(chunker.preroll.capacity(), preroll_capacity);
    assert_eq!(chunker.partial.capacity(), partial_capacity);
    assert_eq!(chunker.flush(), None);
    assert_eq!(chunker.stats(), ChunkerStats::default());
}

#[test]
fn silence_in_odd_sized_frames_does_not_grow_memory() {
    // Frames that never line up with the 20 ms windows.
    let second = Tape::new().quiet(1_000);
    let mut chunker = chunker();
    let mut sent = 0_u64;
    let mut most = 0;

    while sent < 3_600 * u64::from(SAMPLE_RATE) {
        let frame = &second.samples[..441];
        assert!(
            chunker
                .push_samples(sent / SAMPLES_PER_MS, frame)
                .is_empty()
        );
        sent += frame.len() as u64;
        most = most.max(chunker.buffered_samples());
    }

    assert!(most <= IDLE_LIMIT, "held {most} samples");
    assert!(chunker.partial.capacity() <= 2 * WINDOW_SAMPLES);
    assert_eq!(chunker.stats(), ChunkerStats::default());
}

#[test]
fn a_room_with_clicks_does_not_grow_memory() {
    // An hour of quiet with a click every ten seconds. Each click opens a
    // chunk that is dropped at the next pause.
    let ten_seconds = Tape::new().quiet(5_000).burst(60).quiet(4_940);
    let mut chunker = chunker();
    let mut most = 0;

    for n in 0..3_600 * 50_u64 {
        let at = (n as usize % 500) * FRAME;
        let frame = &ten_seconds.samples[at..at + FRAME];
        assert!(chunker.push_samples(n * FRAME_MS, frame).is_empty());
        most = most.max(chunker.buffered_samples());
    }

    // Padding, click and pause: under a second of audio.
    assert!(most <= samples_in(1_000), "held {most} samples");
    assert_eq!(
        chunker.stats(),
        ChunkerStats {
            dropped: 360,
            ..ChunkerStats::default()
        }
    );
}

#[test]
fn unbroken_speech_never_holds_more_than_one_chunk() {
    // Ten minutes of "speech" with no pause: memory stays within one
    // maximum chunk.
    let block = Tape::new().syllables(2_400, 180, 60);
    let mut chunker = chunker();
    let mut most = 0;
    let mut chunks = 0;

    for n in 0..600 * 50_u64 {
        let at = (n as usize % 120) * FRAME;
        let frame = &block.samples[at..at + FRAME];
        for chunk in chunker.push_samples(n * FRAME_MS, frame) {
            assert!(chunk.duration_ms() <= 28_000);
            assert!(chunk.samples.capacity() <= samples_in(28_000));
            chunks += 1;
        }
        most = most.max(chunker.buffered_samples());
    }

    assert!(most <= samples_in(28_000), "held {most} samples");
    assert!((21..=30).contains(&chunks), "{chunks} chunks");
}

// ----------------------------------------------------------------- config

#[test]
fn defaults_are_the_documented_ones() {
    let config = ChunkerConfig::default();
    assert_eq!(config.pause_ms, 700);
    assert_eq!(config.max_chunk_ms, 28_000);
    assert_eq!(config.cut_search_ms, 8_000);
    assert_eq!(config.padding_ms, 200);
    assert_eq!(config.min_speech_ms, 160);
    assert_eq!(config.speech_ratio, 3.0);
    assert_eq!(config.min_speech_level, 0.002);
    assert_eq!(config.noise_doubling_ms, 2_000);
    assert_eq!(config.max_gap_ms, 100);
    // The defaults need no adjusting.
    assert_eq!(*chunker().config(), config);
    // Under whisper.cpp's 30 s window, and no chunk under its 100 ms floor.
    assert!(config.max_chunk_ms < 30_000);
    assert!(config.min_speech_ms >= 100);
}

#[test]
fn settings_are_rounded_to_whole_windows() {
    let config = ChunkerConfig {
        pause_ms: 505,
        max_chunk_ms: 10_019,
        cut_search_ms: 2_015,
        padding_ms: 101,
        min_speech_ms: 90,
        ..ChunkerConfig::default()
    };

    let used = Chunker::new(Speaker::Me, config).config().clone();

    assert_eq!(used.pause_ms, 520);
    assert_eq!(used.max_chunk_ms, 10_000);
    assert_eq!(used.cut_search_ms, 2_000);
    assert_eq!(used.padding_ms, 120);
    assert_eq!(used.min_speech_ms, 100);
}

#[test]
fn unusable_settings_are_brought_into_range() {
    let zeros = ChunkerConfig {
        pause_ms: 0,
        max_chunk_ms: 0,
        cut_search_ms: 0,
        padding_ms: 0,
        min_speech_ms: 0,
        speech_ratio: 0.0,
        min_speech_level: 0.0,
        noise_doubling_ms: 0,
        max_gap_ms: 0,
    };
    let used = Chunker::new(Speaker::Me, zeros).config().clone();
    assert_eq!(
        used,
        ChunkerConfig {
            pause_ms: 20,
            max_chunk_ms: 1_000,
            cut_search_ms: 20,
            padding_ms: 0,
            min_speech_ms: 20,
            speech_ratio: 1.0,
            min_speech_level: 0.000_01,
            noise_doubling_ms: 100,
            max_gap_ms: 1,
        }
    );

    let huge = ChunkerConfig {
        pause_ms: u32::MAX,
        max_chunk_ms: 60_000,
        cut_search_ms: u32::MAX,
        padding_ms: u32::MAX,
        min_speech_ms: u32::MAX,
        speech_ratio: f32::NAN,
        min_speech_level: f32::INFINITY,
        noise_doubling_ms: u32::MAX,
        max_gap_ms: u32::MAX,
    };
    let used = Chunker::new(Speaker::Me, huge).config().clone();
    assert_eq!(
        used,
        ChunkerConfig {
            pause_ms: 60_000,
            max_chunk_ms: 60_000,
            cut_search_ms: 30_000,
            padding_ms: 15_000,
            min_speech_ms: 30_000,
            speech_ratio: 3.0,
            min_speech_level: 0.002,
            noise_doubling_ms: u32::MAX,
            max_gap_ms: u32::MAX,
        }
    );
}

#[test]
fn extreme_settings_still_chunk_without_panicking() {
    let tape = Tape::new()
        .quiet(1_000)
        .syllables(5_000, 180, 60)
        .quiet(1_000)
        .burst(300)
        .quiet(500);
    let configs = [
        ChunkerConfig {
            pause_ms: 0,
            max_chunk_ms: 0,
            cut_search_ms: 0,
            padding_ms: 0,
            min_speech_ms: 0,
            speech_ratio: 0.0,
            min_speech_level: 0.0,
            noise_doubling_ms: 0,
            max_gap_ms: 0,
        },
        ChunkerConfig {
            pause_ms: u32::MAX,
            max_chunk_ms: u32::MAX,
            cut_search_ms: u32::MAX,
            padding_ms: u32::MAX,
            min_speech_ms: u32::MAX,
            speech_ratio: f32::MAX,
            min_speech_level: f32::MAX,
            noise_doubling_ms: u32::MAX,
            max_gap_ms: u32::MAX,
        },
        ChunkerConfig {
            // Padding longer than the pause, and as long as the search.
            pause_ms: 100,
            max_chunk_ms: 1_000,
            cut_search_ms: 500,
            padding_ms: 250,
            ..ChunkerConfig::default()
        },
    ];

    for config in configs {
        let mut chunker = Chunker::new(Speaker::Me, config);
        let limit = u64::from(chunker.config().max_chunk_ms);
        let mut chunks = feed_at(&mut chunker, &tape.samples, 441, 0);
        chunks.extend(chunker.flush());

        let mut previous_end = 0;
        for chunk in &chunks {
            assert!(chunk.duration_ms() <= limit, "{:?}", chunker.config());
            assert!(!chunk.samples.is_empty());
            // In order, never overlapping. Stamps are whole milliseconds,
            // hence the one of slack.
            assert!(chunk.start_ms + 1 >= previous_end, "{:?}", chunker.config());
            previous_end = chunk.end_ms();
        }
    }
}
