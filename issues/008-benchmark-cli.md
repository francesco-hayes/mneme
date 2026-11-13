# 008: Benchmark CLI and speed table

Status: paused · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 007 · Owns: `crates/mneme-transcribe/`

## Goal

Finish build step 2: speed numbers for each model size on this Mac, from a repeatable command.

## Scope

1. **`mneme-bench` binary**: `--model <path>` (or `--model-id` resolved through the model store) and `--audio <wav>`. Runs the WAV through the chunker and the engine as the live pipeline will. Prints audio length, model load time, transcription wall time, real-time factor, chunk count, and the transcript.
2. Accepts any WAV: converts to 16 kHz mono first.
3. **`docs/benchmarks.md`** in the crate (or repo `docs/`): a table of model, size, load time, real-time factor, with the machine and date. Fill in rows only from real runs on this machine. Rows for models that were not downloaded and run stay empty and marked as not measured.
4. From the measured rows, propose a target real-time factor per model and which model should be the first-run recommendation. Mark it as a proposal.

## Acceptance criteria

- [ ] `mneme-bench --help` works; running it on the test WAV with the tiny model prints all fields.
- [ ] The measured rows in the table come from runs made in this issue, with the command used.
- [ ] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

- [ ] Run `mneme-bench` on a real meeting recording (from `mneme-record`) for each model size I care about, and confirm the table and the recommendation.

## Result

**Paused on 2026-11-12 at Francesco's request (low priority; run later). Written by the orchestrator, because the implementing agent was stopped before it reported.**

What exists and was checked by the orchestrator after the stop:

- `src/bin/` holds the `mneme-bench` binary, with `tests/bench_cli.rs` and helper scripts `scripts/make-bench-audio.sh`, `scripts/bench-models.sh` and `scripts/bench-rows.py`. Small changes were also made to `src/models.rs`, `src/models/store.rs`, `src/whisper.rs`, `src/lib.rs` and `tests/models.rs`.
- `cargo test -p mneme-transcribe` passes (208 tests, 1 ignored) and `cargo clippy -p mneme-transcribe --all-targets -- -D warnings` is clean.

What does not exist:

- `docs/benchmarks.md`: no benchmark table, no measured numbers, no proposal for a target speed or a first-run recommendation.
- No report from the agent, so nothing here says what the CLI's flags are, what was run, or what it found. None of the acceptance criteria are ticked for that reason; read the binary's `--help` and source before relying on it.

Models on disk for when this resumes (gitignored, each SHA-256 checked against the catalog): `ggml-tiny.en.bin`, `ggml-base.bin`, `ggml-small.bin`, `ggml-large-v3-turbo.bin` in `crates/mneme-transcribe/models/`.

To resume: review the binary, run it in release mode on each of the four models on a quiet machine, write `docs/benchmarks.md`, then tick the criteria.
