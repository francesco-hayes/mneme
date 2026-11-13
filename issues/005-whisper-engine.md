# 005: Whisper engine wrapper

Status: done · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 001 · Owns: `crates/mneme-transcribe/`

## Goal

Load a Whisper model and turn a slice of 16 kHz mono samples into text segments, on the GPU.

## Scope

1. **Maintenance check first.** Look at how actively `whisper-rs` is maintained (recent releases, whisper.cpp version it tracks, open issues on macOS/Metal). Record findings in the Result. If it is not fit to depend on, stop and propose the alternative before writing engine code.
2. **`Transcriber` trait**: `transcribe(samples, language) -> Vec<(start_offset_ms, end_offset_ms, text)>`. The live pipeline and tests depend on the trait, not on Whisper.
3. **`WhisperEngine`** implementing it with `whisper-rs`, Metal enabled. Load from a model file path once; reuse across calls. Trim whitespace; drop empty segments and Whisper's non-speech markers (bracketed or parenthesised tags such as blank-audio markers).
4. **Language**: `auto` or a language code. An English-only model with a non-English language returns a clear error at load or first use.
5. **`FakeTranscriber`** for tests and other crates: returns scripted text.
6. A script or documented command to fetch the smallest English model into a gitignored folder for local tests.

## Acceptance criteria

- [x] The crate builds with Metal on this Mac using Command Line Tools only (no full Xcode); if that is not possible, say exactly what is missing.
- [x] Integration test: transcribe a short speech WAV committed under the crate's test data (generate one with the macOS `say` command and convert it to 16 kHz mono) with the tiny English model; expected words appear. The test skips with a clear message when the model file is absent.
- [x] Unit tests for marker filtering and the English-only language rule.
- [x] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

Done 2026-11-12 on macOS 27.0.1, Apple M2 Pro, rustc 1.100.0, cmake 4.4.5, Xcode Command Line Tools only.

### Maintenance check: `whisper-rs`

Checked on 2026-11-12 against crates.io, the repository API and the crate source.

| Question | Finding |
| --- | --- |
| Where it lives | Moved from GitHub to Codeberg: `codeberg.org/tazz4843/whisper-rs`. Not archived. One maintainer. |
| Latest release | `whisper-rs` 0.16.0 with `whisper-rs-sys` 0.15.0, both published 2026-03-12. Eight months old. Earlier cadence: 0.15.1 in 2025-09, 0.14.4 in 2025-07. |
| whisper.cpp it tracks | v1.8.3 (2026-01-15), vendored in the sys crate. Upstream is at v1.9.5 (2026-10-06), ten tagged versions ahead (v1.8.4 to v1.9.5). |
| Recent activity | Last commit 2026-03-14. Last maintainer comment found: 2026-05-03. Issues opened since April have no maintainer reply. A pull request to move to whisper.cpp v1.9.5 was opened 2026-11-08 and is unreviewed. |
| Open issues | 31 open, 5 open pull requests. |
| macOS and Metal | No open issue reports Metal failing to build or run. Open and macOS-related: #176 "2x slower on macOS" (2024, never reproduced), #215 bindgen missing headers on macOS (did not happen here), #198 iOS builds. |
| Bugs that could touch us | #280: `FullParams::set_language` leaks a small string on every call (confirmed in the 0.16.0 source). #277: `set_abort_callback_safe` reads past a heap allocation. #271: the `*_safe` callbacks leak. |

Verdict: fit to depend on, with care. It is slow-moving rather than abandoned, it is the binding the PRD names, and it built and ran on this Mac with Metal at the first attempt. What was done about the risks:

- The version is pinned exactly (`=0.16.0`) so nothing changes without a decision.
- The engine never uses the callbacks from #277 and #271. For #280 it does not call `set_language` for `en` or `auto`; another language code leaks a few bytes per call, which stays in the kilobytes over a long meeting.
- Everything else in the app depends on the `Transcriber` trait, so the binding can be replaced inside one file (`src/whisper.rs`, about ten `whisper-rs` calls).

If the crate stays quiet, the alternative I would propose is a small sys crate of our own (cmake plus bindgen against a current whisper.cpp release) behind the same trait. Not needed today.

### What was built

All in `crates/mneme-transcribe/`. Public API, re-exported from the crate root:

```rust
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, samples: &[f32], language: &Language)
        -> Result<Vec<TextSpan>, TranscribeError>;
}
// Also implemented for &T, Box<T> and Arc<T>, so `Arc<dyn Transcriber>` works.

pub struct TextSpan { pub start_ms: u64, pub end_ms: u64, pub text: String } // offsets inside the slice
impl TextSpan { pub fn into_segment(self, speaker: Speaker, chunk_start_ms: u64) -> mneme_core::Segment; }
pub fn duration_ms(sample_count: usize) -> u64;

pub struct Language; // `auto` or a 2-3 letter code; FromStr, Display, Default = auto
impl Language { pub const fn auto() -> Self; pub fn english() -> Self; pub fn is_auto(&self) -> bool;
                pub fn code(&self) -> Option<&str>; pub fn as_str(&self) -> &str; }

pub enum TranscribeError { ModelNotFound { path }, ModelLoad { path, message },
    EnglishOnlyModel { requested }, UnsupportedLanguage { code }, Engine { message } }

pub struct WhisperOptions { pub use_gpu: bool /* true */, pub threads: Option<u32> /* None */ }
pub struct WhisperEngine; // implements Transcriber
impl WhisperEngine {
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, TranscribeError>;
    pub fn load_with(model_path: impl AsRef<Path>, options: &WhisperOptions) -> Result<Self, TranscribeError>;
    pub fn model_path(&self) -> &Path;
    pub fn is_multilingual(&self) -> bool;
    pub fn check_language(&self, language: &Language) -> Result<(), TranscribeError>;
    pub fn whisper_cpp_version() -> &'static str; // "1.8.3"
    pub fn system_info() -> String;
}

pub struct FakeTranscriber; // implements Transcriber
impl FakeTranscriber {
    pub fn new() -> Self; pub fn with_texts(texts) -> Self;
    pub fn then_text(self, text) -> Self; pub fn then_spans(self, spans: Vec<TextSpan>) -> Self;
    pub fn then_silence(self) -> Self; pub fn then_error(self, error: TranscribeError) -> Self;
    pub fn with_delay(self, delay: Duration) -> Self;
    pub fn calls(&self) -> Vec<FakeCall>; pub fn call_count(&self) -> usize;
}
pub struct FakeCall { pub sample_count: usize, pub language: Language }
```

Behaviour:

- **Engine.** The model and its working memory are loaded once and reused; calls from several threads run one after another. Decoding is greedy, with no context carried from one call to the next, because consecutive slices can come from different speakers. An empty slice returns nothing. whisper.cpp ignores anything under 100 ms.
- **Text clean-up** (`src/text.rs`). Whitespace is trimmed and collapsed. A segment made only of markers, punctuation or symbols is dropped. A marker is `[...]` or `(...)` holding up to 40 characters of letters, spaces, `_`, `-`, `'`. Square-bracketed markers are also removed from inside a segment that has words. Parenthesised text next to words is kept.
- **Language.** A multilingual model takes `auto` or any code in whisper.cpp's language table; an unknown code is `UnsupportedLanguage`. An English-only model takes `en`, treats `auto` as `en`, and returns `EnglishOnlyModel` for anything else, from `check_language` and from every `transcribe` call. whisper.cpp itself would silently transcribe as English.
- **FakeTranscriber.** One scripted reply per call, in order. When the script runs out a call returns one span covering the slice with the text `chunk N`. It records every call and can sleep per call to imitate a slow model.

Files for the next issues:

- Fixture: `crates/mneme-transcribe/tests/data/speech.wav`, 5.75 s, 16 kHz mono 16-bit, 188,248 bytes, made with `say -v Samantha` and `afconvert` (commands in the `.gitignore` beside it, which re-includes the file). It says "Hello, this is a test of the meeting transcription. The quick brown fox jumps over the lazy dog."
- Model: `crates/mneme-transcribe/models/ggml-tiny.en.bin`, ignored by the root `.gitignore` (`models/`, `*.bin`). Fetch it with `crates/mneme-transcribe/scripts/fetch-test-model.sh`, which downloads to a `.part` file, checks the SHA-256, then renames.
- How tests find the model: the `MNEME_TEST_MODEL` environment variable if set, else the path above. Other crates can use the same two steps.

Model download, verified:

| | Hugging Face lists (`ggerganov/whisper.cpp`, `ggml-tiny.en.bin`) | Downloaded file |
| --- | --- | --- |
| SHA-256 | `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f` | `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f` |
| Size | 77,704,715 bytes | 77,704,715 bytes |

### Verification

| Command | Outcome |
| --- | --- |
| `cargo build -p mneme-transcribe` (first build, compiles whisper.cpp) | Passed in 24 s. cmake reported "Metal framework found" and "Embedding Metal library". bindgen generated the bindings with the Command Line Tools' libclang. |
| `xcrun -f metal` | "unable to find utility "metal"": the Metal shader compiler is not installed, and the build did not need it. |
| `cargo test -p mneme-transcribe` with the model present | Passed: 36 unit tests, 10 integration tests, 2 doc tests. |
| `MNEME_TEST_MODEL=/nonexistent/model.bin cargo test -p mneme-transcribe --test whisper_engine` | Passed: the 8 tests that need the model each print `SKIPPED <test>: no Whisper model file. Run crates/mneme-transcribe/scripts/fetch-test-model.sh, or set MNEME_TEST_MODEL ...`. The same happened before the model was downloaded. |
| `cargo clippy -p mneme-transcribe --all-targets -- -D warnings` | Passed, no warnings. |
| `cargo fmt -p mneme-transcribe -- --check` | Passed. |
| `cargo test -p mneme-transcribe --release --test whisper_engine` | Passed, 10 tests. Release build took 22 s. |
| `crates/mneme-transcribe/scripts/fetch-test-model.sh` | Downloaded and verified the model; a second run reported it already there and verified. |

Acceptance criteria:

1. **Builds with Metal on Command Line Tools only: yes.** Nothing is missing. `whisper-rs-sys` embeds the Metal shader source in the binary and Metal compiles it when the model loads, so the Xcode-only `metal` compiler is never called. Needed on the machine: cmake and the Command Line Tools.
2. **Integration test: passes.** The engine returned "Hello, this is a test of the meeting transcription, the quick brown fox jumps over the lazy" (0 to 5120 ms) and "dog." (5120 to 5480 ms). The test checks for "hello", "meeting", "quick brown fox" and "lazy dog", and skips cleanly without the model.
3. **Unit tests: pass.** 12 for marker filtering, 6 for the language rule, plus the English-only refusal against the real tiny.en model.
4. **`cargo test` and clippy: pass.**

Metal is in use at run time. whisper.cpp's log for the default engine shows `use gpu = 1`, `whisper_backend_init_gpu: using Metal backend`, `ggml_metal_init: picking default device: Apple M2 Pro`, and 20 Metal kernels compiled during the run. The same test with `use_gpu: false` logs `whisper_backend_init_gpu: no GPU found` and compiles none. One call on the 5.75 s fixture took about 77 ms in a debug build.

### Not verified

- A multilingual model. Only `ggml-tiny.en.bin` was downloaded, so language detection and non-English codes were never run on a real model. The rule itself is unit-tested.
- Audio longer than 30 s, real meeting audio, and accuracy in general.
- The string leak in `set_language` was read in the source, not measured.
- The abort at exit (see follow-ups) was seen with an engine held in a `static`. That `std::process::exit` triggers it too is inferred from the same code path, not run.
- What makes the first model load slow again (see follow-ups).

### Deviations from the issue text

- `transcribe` returns `Result<Vec<TextSpan>, TranscribeError>` with named fields in place of bare tuples, and takes `&self` with `Send + Sync` so one loaded model can be shared.
- The language is a `Language` type parsed from `auto` or a code, not a string.
- An English-only model with `auto` transcribes as English and is not an error. It has no language detection, and failing on the default setting would make every `.en` model unusable until the setting is changed.
- Parenthesised tags are dropped only when the segment has no words besides them. Removing them next to words could delete something that was said.
- Segments with only punctuation or symbols (`...`, `♪`) are dropped too.
- Added beyond the scope: `WhisperOptions` (`use_gpu`, `threads`), `check_language`, and a CPU-only test, which is also what proves the GPU path differs.

### Follow-ups

- **009, 021: drop the engine before the process exits.** With a model still loaded at exit, whisper.cpp's Metal backend fails an assertion (`ggml-metal-device.m:608: GGML_ASSERT([rsets->data count] == 0)`) and the process aborts. Do not keep the engine in a `static` or in app state that outlives quit; quitting during a recording must stop the session and drop the engine first.
- **UI: the first model load can take about 7 s longer.** Metal compiles the embedded shaders on first use: 7.6 s on the first debug run and 7.4 s on the first release run, 0.01 s on every later run. Show a loading state. Shipping a precompiled shader library would avoid it but needs full Xcode.
- **006:** Whisper's segment start times are coarse. With 2 s of silence in front of the fixture the first span still started at 0 ms. Trim leading silence in the chunker so the chunk's `start_ms` carries the timing. Slices under 100 ms produce nothing.
- **008:** decoding is fixed to greedy. Measure beam search against it and add the choice to `WhisperOptions` if it earns its cost. `hound` is a dev-dependency only; the benchmark binary needs its own WAV reader.
- **Logging:** whisper.cpp writes to stderr. Debug builds also print every decoded token, which includes the transcript text; release builds do not. `whisper_rs::install_logging_hooks()` can silence it once the app has a logging plan.
- **Settings UI:** the engine validates language codes against whisper.cpp's table but exports no list of languages yet.
- **Root files, not changed by me:** `Cargo.lock` gained the new dependencies and needs committing. The README's build requirements should name cmake and the Xcode Command Line Tools. No dev-profile change is needed: whisper.cpp is compiled optimized in debug builds.
- **whisper-rs upkeep:** re-check before release whether the whisper.cpp v1.9.5 pull request (#284) and the fixes for #277 and #280 have landed, and bump the pin deliberately.
