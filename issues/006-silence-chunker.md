# 006: Silence chunker

Status: done · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 005 · Owns: `crates/mneme-transcribe/`

## Goal

Cut a continuous stream of frames for one speaker into chunks at pauses, and never hand Whisper silence.

## Scope

1. **`Chunker`**: push `AudioFrame`s for one speaker; get back zero or more finished chunks (`speaker`, `start_ms`, samples). `flush()` returns whatever is pending.
2. **Speech detection**: energy-based, with an adaptive noise floor so a noisy room does not count as speech. Keep it dependency-light; note in the Result if a model-based detector looks necessary after testing.
3. **Cut rules**: end a chunk after a pause of the configured length; force a cut at the maximum chunk length, at the quietest point in the last stretch before the limit; keep a little audio either side of speech so words are not clipped.
4. **Drop silent chunks**: a chunk with less than the minimum amount of speech is discarded.
5. Gaps in frame timestamps (a source restarting) end the current chunk; `start_ms` always reflects real meeting time.
6. A `ChunkerConfig` with documented defaults.

## Acceptance criteria

- [x] Tests with synthetic audio (tone bursts and noise as "speech", low noise as "silence"): one chunk per burst; chunk `start_ms` matches where the burst began; pure silence yields nothing; a long unbroken burst is force-cut under the maximum; `flush()` returns the tail; a timestamp gap splits chunks.
- [x] Test with the speech WAV from issue 005 containing an inserted long pause: two chunks, no words clipped (checked with the tiny model when present, skipped otherwise).
- [x] No unbounded memory growth while fed silence for a long simulated time.
- [x] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

Done 2026-11-12 on macOS 27.0.1, Apple M2 Pro, rustc 1.100.0. No new dependencies; `Cargo.toml` and `Cargo.lock` are unchanged.

### What was built

All in `crates/mneme-transcribe/`:

- `src/chunker.rs`: the chunker.
- `src/chunker/tests.rs`: 50 unit tests on synthetic audio.
- `tests/chunker.rs`: 5 tests on the speech fixture, 2 of them with the tiny model.
- `tests/common/mod.rs`: the fixture and model helpers, moved out of `tests/whisper_engine.rs` so both test files share them. The engine tests themselves are unchanged.
- `src/lib.rs`: re-exports and a crate example that goes from frames to chunks to segments.

Public API, re-exported from the crate root:

```rust
pub struct Chunk { pub speaker: Speaker, pub start_ms: u64, pub samples: Vec<f32> }
impl Chunk { pub fn duration_ms(&self) -> u64; pub fn end_ms(&self) -> u64; }

pub struct Chunker; // Send; one per speaker
impl Chunker {
    pub fn new(speaker: Speaker, config: ChunkerConfig) -> Self;
    pub fn push(&mut self, frame: &AudioFrame) -> Vec<Chunk>;          // chunks completed by this frame
    pub fn push_samples(&mut self, start_ms: u64, samples: &[f32]) -> Vec<Chunk>;
    pub fn flush(&mut self) -> Option<Chunk>;                          // the chunk in progress, if any
    pub fn speaker(&self) -> Speaker;
    pub fn config(&self) -> &ChunkerConfig;                            // values in use, after rounding
    pub fn stats(&self) -> ChunkerStats;
    pub fn noise_floor(&self) -> f32;                                  // RMS
    pub fn buffered_samples(&self) -> usize;
}
pub struct ChunkerStats { pub chunks: u64, pub dropped: u64, pub forced_cuts: u64, pub gaps: u64 }
pub struct ChunkerConfig { /* below */ }   // Default, Clone, PartialEq
```

How it works:

- **Speech detection.** Audio is measured in 20 ms windows: RMS with the window's mean removed, so a constant offset from a device is not sound. A window is speech when its level is at least `speech_ratio` times the noise floor and at least `min_speech_level`.
- **Noise floor.** It starts at `min_speech_level`. A quieter window pulls it down fast (30% of the distance per window). A window between the floor and the threshold pulls it up slowly (2% per window, about one second). While sound stays above the threshold it still doubles every `noise_doubling_ms`, so steady noise that starts suddenly stops counting as speech; dips between words pull it back down.
- **Start.** A chunk opens at the first speech window and reaches back `padding_ms` into the audio before it. Earlier silence is not in the chunk, so `start_ms` is the start of speech minus the padding, to within one window. This is what the finding from 005 asked for.
- **End.** A chunk closes after `pause_ms` without speech and keeps `padding_ms` after the last speech window. The rest of the silence is dropped. Padding never repeats audio: when the pause is shorter than twice the padding, the next chunk starts where the previous one ended.
- **Forced cut.** At `max_chunk_ms` the chunk is cut at a window boundary in its last `cut_search_ms`. Each boundary is scored by the energy of the 100 ms either side, nearest windows weighing most, so the cut goes to the middle of the widest quiet stretch. What follows the cut starts the next chunk: nothing lost, nothing repeated.
- **Drop.** A chunk with less than `min_speech_ms` of speech windows is discarded and counted in `stats().dropped`.
- **Gaps.** Each push compares `start_ms` with where the previous samples ended. More than `max_gap_ms` apart, either way, ends the chunk in progress there; audio from before the gap is not used as padding after it. Smaller differences are treated as continuous audio. The clock is re-read from every frame, so chunk times follow the frames' clock and do not drift.
- **Memory.** While nobody speaks the chunker holds at most the padding plus one window (3,520 samples by default). With a chunk open it holds at most `max_chunk_ms` of audio.

Default configuration:

| Field | Default | Why |
| --- | --- | --- |
| `pause_ms` | 700 | Whisper carries no context between chunks, so a cut inside a sentence costs accuracy. 700 ms is meant to wait for the end of a sentence or turn, not a gap between words. Nothing shows the transcript live, so waiting costs nothing. |
| `max_chunk_ms` | 28,000 | whisper.cpp works on 30 s at a time. Under that a chunk is decoded in one piece; over it whisper.cpp splits it again itself. 28 s leaves margin and keeps forced cuts rare. |
| `cut_search_ms` | 8,000 | The stretch before the limit searched for the quietest point. Long enough to usually hold a breath; a forced chunk is still at least 20 s. |
| `padding_ms` | 200 | Soft sounds at word edges (`f`, `s`, `h`, `th`) are below the threshold; the padding keeps them. A usual safety margin, not a measured need: on the fixture every word survives even with no padding, so the fixture cannot show how much is needed. |
| `min_speech_ms` | 160 | whisper.cpp returns nothing under 100 ms. 160 ms is meant to sit between a click and the shortest spoken answer. |
| `speech_ratio` | 3.0 | About 9.5 dB over the noise floor. |
| `min_speech_level` | 0.002 | RMS, -54 dBFS. Keeps digital silence and the hiss of a quiet input from counting as speech. |
| `noise_doubling_ms` | 2,000 | How fast the floor rises under sustained sound. |
| `max_gap_ms` | 100 | Timestamp differences up to this are rounding or a dropped buffer, not a break. |

Durations are rounded to whole 20 ms windows and out-of-range values are brought into range, not rejected. The defaults come from how Whisper behaves and from the tests below. They have not been tuned on real meeting audio; issue 008 is where that happens.

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-transcribe` with the model present | Passed: 86 unit tests (36 from 005, 50 new), 5 chunker integration tests, 10 engine tests, 3 doc tests. |
| `MNEME_TEST_MODEL=/nonexistent/model.bin cargo test -p mneme-transcribe --test chunker` | Passed: the 2 tests that need the model print `SKIPPED ...` and pass; the other 3 run. |
| `cargo test -p mneme-transcribe --release` | Passed: 85 unit tests (one test checks a debug assertion and is debug-only), 5, 10 and 3. |
| `cargo clippy -p mneme-transcribe --all-targets -- -D warnings` | Passed, no warnings. |
| `cargo fmt -p mneme-transcribe -- --check` | Passed. |
| `cargo doc -p mneme-transcribe --no-deps` | No warnings. |

Acceptance criteria:

1. **Synthetic audio: pass.** A 220 Hz tone with noise stands in for speech, white noise at RMS 0.0005 for silence.
   - One chunk per burst, and `start_ms` matches the burst: bursts at 1000 to 2000 ms and 3500 to 4100 ms give chunks at exactly 800 to 2200 and 3300 to 4300 ms. Bursts that begin inside a window (seven offsets) start one padding earlier to within 20 ms. Every chunk is compared sample for sample with the input at the place its `start_ms` names.
   - Pure silence (zeros, low noise, both) yields nothing, at push and at flush.
   - A long unbroken burst is cut under the maximum: 70 s with no gap over 60 ms gives 3 chunks, each at most 28 s, the forced ones at least 20 s, contiguous, and both cuts inside a dip. A second test puts one 360 ms gap in the last stretch and the cut lands in it.
   - `flush()` returns the tail, including samples short of a whole window; a second flush returns nothing.
   - A timestamp gap splits: 2 s of unbroken tone with the second half stamped 5 s later gives chunks at 10,000 and 16,000 ms. A clock that jumps back splits too.
   - Also covered: the pause boundary (680 ms does not split, 700 ms does), clicks dropped, the 160 ms rule at 140 and 160 ms, a constant offset, samples that are not numbers, steady noise from the start, noise that starts suddenly, speech over noise, the floor coming back down, frame sizes from 1 sample to the whole recording giving the same chunks, a clock that drifts 1% from the sample count, and configuration rounding and clamping.
2. **Speech WAV with an inserted pause: pass.** The fixture gets 2 s of low noise in front, 3 s between its sentences and 1.5 s after, and is pushed as 20 ms frames from meeting time 600,000 ms. Result: two chunks, each starting one padding before its sentence to within 20 ms. With the tiny model the first chunk transcribes as "Hello, this is a test of the meeting transcription." and the second as "The quick brown fox jumps over the lazy dog.", every word present. A second test cuts the unmodified fixture with a 160 ms pause into three chunks (0 to 660, 660 to 3000, 3000 to 5700 ms) and all 18 words still come out in order. Both skip cleanly without the model.
3. **No unbounded memory growth: pass.** Three simulated hours of silence in 20 ms frames: never more than 3,520 samples held, the two buffers keep the capacity they were created with, no chunk. An hour in 441-sample frames, an hour with a click every ten seconds (360 chunks opened and dropped, under one second held) and ten minutes of unbroken speech (never more than 28 s held) are bounded too.
4. **`cargo test -p mneme-transcribe` and clippy: pass.**

Do the tests fail when the behavior is broken? I broke `chunker.rs` in 27 ways, one at a time, ran the chunker tests and restored the file each time: no padding before speech, pause doubled, minimum-speech rule off, gaps ignored, noise floor frozen (three variants), forced cut without the search, forced cut off, flush returning nothing, flush dropping the partial window, pre-roll never trimmed, trailing silence kept, offset not removed, clock anchored once, padding zero, pre-roll overlapping the previous chunk, audio after a forced cut lost, old audio kept across a gap, chunk start 40 ms late, five that clip 100 to 400 ms off chunk edges, and two on the minimum level. 26 made tests fail. The one that did not is removing the `min_speech_level` term from the threshold alone, which changes nothing because the floor's lower limit already enforces the same level; removing both is one of the 26. Most of these runs were made before the last three tests were added and before the final wording of the doc comments; the code they mutate did not change afterwards.

How sharp the word check is: the two model tests catch a cut that goes about 100 ms or more into speech (300 ms off the start of every chunk loses "The" and "Hello this is"; 400 ms off the end turns "transcription" into "transcript"). They do not notice the padding going missing: with the padding set to zero both still pass. Edges that fine are held by the timing assertions, not by the words.

### Measured while testing

These came from temporary tests that are not in the repository.

Speech fixture mixed with steady white noise, default settings, tiny model:

| Room noise (RMS) | Result |
| --- | --- |
| 0.0005 and 0.003 | Two chunks, both sentences exact, nothing else. |
| 0.01 | Both sentences exact, plus one 1.7 s chunk of noise at the very start. |
| 0.02 | Both sentences exact, plus a 3.6 s chunk of noise at the start. |
| 0.04 | Both sentences found, plus a 5.5 s noise chunk. Whisper misheard two words ("transmission", "round"). |
| 0.08 | Speech not found. It was less than three times louder than the noise. |

The tiny model returned no text for any of the noise chunks.

Time per `transcribe` call, tiny model, Metal, same in debug and release: 37 ms for 0.5 s of audio, 45 ms for 1.4 s, 58 ms for 2.9 s, 80 ms for 5.75 s, 295 ms for 28.7 s. That is roughly 33 ms per call plus 9 ms per second of audio, so short chunks cost more per second of audio than long ones.

### Not verified

- **Real meeting audio.** Everything ran on synthetic signals and one recording of a synthetic voice. Real microphones, real rooms, two people talking over each other, laptop fans and keyboards were not tested.
- **The defaults as the best values.** They are reasoned, not tuned.
- **Whether a model-based detector is needed.** Nothing in these tests shows it is. The cases where measuring loudness fails by design are listed below, and only real recordings can say how much they matter.
- **Whisper on noise-only chunks with larger models.** The tiny model returned nothing; a larger model may invent text.
- **Chunks near the 28 s limit through the engine in a test.** The chunker's 28 s chunks were not transcribed in a kept test. One 28.7 s slice was transcribed while measuring cost and came back with one word missing ("The" in the fifth repetition), which may be the model and not the length.
- **The live path.** No real `AudioSource` fed the chunker; the frame contract was read from `crates/mneme-audio/src/framer.rs` and `source.rs`, which are still in progress.

Known weak spots of the detector:

- It measures loudness, not speech. Typing, coughs, music, a door, other people talking in the room become chunks when they are loud and last 160 ms or more within one chunk.
- When steady noise starts or gets louder, the first seconds of it are taken for speech until the floor catches up: about 2 s for each doubling over the old threshold. That includes the start of a recording in a room louder than about 0.006 RMS (-44 dBFS).
- Sound at a steady level is treated as noise after some seconds, whatever it is. Measured: a steady tone of amplitude 0.3 counts as speech for 13.4 s after digital silence and 10.3 s at the start of a stream; one of amplitude 0.05 for 8.3 s. Real speech dips between syllables, so this did not happen with the fixture or the syllable-like test signal.
- Speech less than three times louder than the room is not found.
- A forced cut can still land inside a word when 8 s of speech hold no gap.

### Deviations from the issue text

- `flush()` returns `Option<Chunk>`, not a list: at most one chunk can be pending.
- `start_ms` is the start of speech minus the padding, not the start of speech itself, because the padding is part of the chunk. Lines are therefore up to 200 ms early, far inside the one-second resolution of transcript timestamps.
- The "long unbroken burst" test uses bursts with 60 ms dips, not one steady tone, because the adaptive floor treats a steady tone as noise after 8 to 13 s. A steady tone cut at the limit is tested separately with the adaptation switched off.
- A timestamp difference of up to 100 ms does not end a chunk.
- A frame whose speaker differs from the chunker's trips a debug assertion; release builds label the chunk with the chunker's speaker.
- Added beyond the scope: `push_samples`, `ChunkerStats`, `noise_floor()`, `buffered_samples()`, `Chunk::duration_ms`/`end_ms`, and more settings than the three the PRD names.

### Follow-ups

- **008: tune the defaults on real recordings.** `mneme-bench` can use `Chunker::push_samples(start_ms, &samples)` on the whole WAV, then `flush()`, and print `stats()`. Things to look at: chunks that come back with no text (noise taken for speech), `dropped`, `forced_cuts`, and words missing at chunk edges. The settings most likely to need changing are `pause_ms`, `speech_ratio`, `min_speech_level` and `noise_doubling_ms`.
- **008: measure the cost of short chunks per model.** With the tiny model a call costs about 33 ms before any audio. If larger models have a larger fixed cost, a longer `pause_ms` or joining short chunks would cut the load. Not built, because it is unmeasured.
- **008 or later: a model-based detector** (for example Silero VAD) is the next step if real recordings show noise chunks or invented text. It would replace the per-window speech decision only; the cut rules can stay.
- **009: wiring.** One `Chunker` per speaker. `push(&frame)` returns the chunks to queue. Call `flush()` at stop and when a source ends or fails, or its last words are lost. A line is `span.into_segment(chunk.speaker, chunk.start_ms)`.
- **009: order.** A chunk is returned when its pause has passed or at a forced cut, up to 28 s after its `start_ms`, so chunks from the two speakers reach the queue in order of completion, not of start time. The sort at stop handles it.
- **009: empty results.** The engine can return no spans for a chunk (noise). That is not an error and produces no line.
- **009: backlog.** Seconds of audio waiting is the sum of `chunk.duration_ms()` over the queue. `buffered_samples()` gives what is still inside a chunker.
- **009: `MNEME_KEEP_AUDIO`.** Write the WAV files from the frames, not from the chunks: the chunker drops the silence.
- **009: gaps.** The chunk before a timestamp gap is returned by the push that brings the first frame after the gap, or by `flush()`.
- **`issues/README.md`:** the table row for 006 still says `in progress`; it is outside my folder.
