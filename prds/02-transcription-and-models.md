# PRD 02: Transcription and models

Covers build step 2, MVP feature 5 (default transcription model) and the gap "Model download manager". Issues: 005–008.

## Problem

Whisper is not a streaming model. It needs audio cut into chunks at silence, it can invent text on silent audio, and a large model on an older Mac can run slower than real time. Models are large files that must be downloaded, verified and managed.

## Goals

- Turn 16 kHz mono audio into text segments with whisper.cpp, on the GPU where available.
- Cut a continuous stream into chunks at silence and drop chunks with no speech.
- Let the user download, verify and delete Whisper models; none are bundled.
- Produce speed numbers per model size on this Mac.

## Non-goals

- Bring-your-own or non-Whisper models.
- Word-level timings or confidence output.
- Translation.

## Requirements

### Engine

- whisper.cpp through the `whisper-rs` crate with Metal enabled, linked into the binary.
- Load a model from a file path; transcribe a slice of samples; return text with start and end offsets inside the slice.
- Language setting: `auto` or a language code. English-only models (`*.en`) reject any language other than English with a clear error.
- A loaded model is reused across chunks. Loading happens once per recording.
- Before issue 005 starts, check how actively `whisper-rs` is maintained (scope doc risk). If it is stale, record the finding in the issue and propose the alternative before writing code.

### Chunker

- Input: a stream of `AudioFrame`s for one speaker. Output: chunks with `start_ms` and samples.
- Cut at a pause in speech. Force a cut when a chunk reaches the maximum length, at the quietest point near the limit.
- Drop chunks that contain no speech, so Whisper never sees silence.
- Pause length, maximum chunk length and the speech threshold are parameters with defaults chosen in issue 006 and re-checked against the benchmark.
- Flush on stop so the tail of the meeting is not lost.

### Model catalog and downloads

- A fixed catalog of Whisper models: id, display name, file name, size, SHA-256, English-only flag, and a short speed/quality note.
- Checksums and sizes are copied from the model source, never guessed.
- Download to a temporary file in the models folder, report progress, verify SHA-256, then rename into place. A failed or cancelled download leaves no partial model that looks installed.
- List installed models, delete a model, cancel a download.
- This is the only code in the app allowed to use the network.

### Benchmark

`mneme-bench --model <path> --audio <wav>` prints audio length, wall time and the real-time factor. Results per model size are recorded in a table in the repo. The scope doc's "Slow models fall behind" risk is settled from this table: a target real-time factor per model.

## Done when

- Unit tests cover the chunker (pauses, forced cuts, silence drop, flush) with synthetic audio, and the catalog/downloader against a local test server.
- An integration test transcribes a short known WAV with the tiny model and finds the expected words. It skips cleanly when the model file is absent.
- Manual: benchmark table filled in for each model size on this Mac.
