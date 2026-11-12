# PRD 00: Foundation

Covers MVP feature 1 (runs locally) and the proposed architecture. Issues: 001.

## Problem

Every other area needs a place to live, shared types to agree on, and a rule set that keeps the app local-only. Without this first, the areas cannot be built side by side.

## Goals

- One repository that builds a Tauri 2 desktop app (Rust + React + TypeScript) on macOS 14.4+.
- A Cargo workspace where each area is its own crate and can be built and tested alone.
- Shared types defined once, so audio, transcription, storage and the pipeline agree on them.
- A typed frontend API module with a mock, so the UI does not wait for the backend.
- Local-only by construction: no telemetry, no network at runtime except model downloads.

## Non-goals

- Any feature behavior. This PRD only produces the skeleton and the contracts.
- Windows or Linux builds.

## Requirements

### Repository layout

```
mneme/
  Cargo.toml                 workspace root
  crates/
    mneme-core/              shared types, no heavy dependencies
    mneme-audio/             mic + system audio capture (PRD 01)
    mneme-transcribe/        whisper engine, chunker, models (PRD 02)
    mneme-store/             meeting folders, transcript, notes, settings (PRD 04)
    mneme-pipeline/          live session (PRD 03)
  src-tauri/                 Tauri app: commands, tray, shortcut (PRD 06)
  src/                       React + TypeScript UI (PRD 05)
  prds/  issues/  mneme-scope.md
```

### Shared types (`mneme-core`)

| Type | Shape | Notes |
| --- | --- | --- |
| `SAMPLE_RATE` | `16_000` | All audio between crates is 16 kHz mono `f32`, which is what Whisper takes. |
| `Speaker` | `Me`, `Them` | `Me` is the mic channel; `Them` is system audio. Displays as `Me` / `Them`. |
| `AudioFrame` | `speaker`, `start_ms: u64`, `samples: Vec<f32>` | `start_ms` is meeting time: milliseconds since recording started, one clock for both channels. |
| `Segment` | `speaker`, `start_ms`, `end_ms`, `text` | One transcribed stretch of speech. |
| `format_timestamp(ms)` / `parse_timestamp(str)` | `hh:mm:ss` | The only timestamp format on disk. |

### Frontend API contract

The UI imports one `api` object. Two implementations exist: a mock with in-memory data, and a Tauri one that calls backend commands. The contract is in [PRD 05](05-ui.md#api-contract); PRD 06 implements the backend side.

### Local-only rules

- No analytics, crash reporting or update checks.
- The only code allowed to open a network connection is the model downloader in `mneme-transcribe`.
- Tauri content security policy blocks remote scripts, styles and connections from the webview.
- MIT license file at the root.

## Done when

- `cargo test --workspace` and `pnpm build` pass on a clean checkout.
- `pnpm tauri dev` opens a window showing the placeholder UI running against the mock API.
- Each crate compiles alone with `cargo build -p <crate>`.
