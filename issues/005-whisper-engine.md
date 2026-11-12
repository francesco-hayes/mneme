# 005: Whisper engine wrapper

Status: todo · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 001 · Owns: `crates/mneme-transcribe/`

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

- [ ] The crate builds with Metal on this Mac using Command Line Tools only (no full Xcode); if that is not possible, say exactly what is missing.
- [ ] Integration test: transcribe a short speech WAV committed under the crate's test data (generate one with the macOS `say` command and convert it to 16 kHz mono) with the tiny English model; expected words appear. The test skips with a clear message when the model file is absent.
- [ ] Unit tests for marker filtering and the English-only language rule.
- [ ] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
