# 008: Benchmark CLI and speed table

Status: todo · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 007 · Owns: `crates/mneme-transcribe/`

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

_Filled in by the implementer._
