# Mneme PRDs

One PRD per build area. Source of truth for scope is [mneme-scope.md](../mneme-scope.md); these PRDs turn each section of it into requirements, and [issues/](../issues/README.md) breaks each PRD into work items.

| PRD | Area | Scope doc source | Issues |
| --- | --- | --- | --- |
| [00](00-foundation.md) | Foundation: workspace, shared types, local-only rules | Overview, MVP feature 1, Architecture | 001 |
| [01](01-audio-capture.md) | Audio capture: mic + system audio | Build step 1; gaps "System audio + mic", "Me vs. them" | 002–004 |
| [02](02-transcription-and-models.md) | Transcription and models | Build step 2; MVP feature 5; gap "Model download manager" | 005–008 |
| [03](03-live-pipeline.md) | Live pipeline | Build step 3; MVP feature 2; gap "Audio retention rule" | 009 |
| [04](04-storage.md) | Storage: meeting folders, transcript, notes, settings | Build step 4; "Storing meetings"; gap "Incremental saves" | 010–014, 026 |
| [05](05-ui.md) | UI: first run, Home, meeting views, Settings | Build step 5; MVP features 3 and 4; Design and UI | 015–020 |
| [06](06-integration-and-finish.md) | Integration and finish: commands, menu bar, shortcut, permissions, signing | Build step 6; gap "Recording indicator" | 021–025 |

## Decisions these PRDs rely on

All rows of the scope doc's two decision logs apply. Three more were settled on 2026-11-12 while setting up the build:

| Decision | Rationale |
| --- | --- |
| System audio uses Core Audio process taps; minimum macOS is 14.4 | Audio-only permission with no recurring re-prompt. Sits behind a trait so ScreenCaptureKit can be swapped in. |
| Rust code is a Cargo workspace: one crate per area (`mneme-core`, `mneme-audio`, `mneme-transcribe`, `mneme-store`, `mneme-pipeline`) plus the Tauri app in `src-tauri` | Each area builds and tests on its own, without the UI; issues in different areas do not touch the same files. |
| The frontend talks to the backend only through one typed API module with a mock implementation | UI can be built and tested before the backend commands exist. |

## Assumptions (change freely)

These were not in the scope doc. Each is a default picked to unblock the build.

- Meetings root is `~/Documents/Mneme/`. Models and `settings.json` live in the app data folder (`~/Library/Application Support/app.mneme.desktop/`).
- Bundle identifier is `app.mneme.desktop`.
- Speaker bleed: v1 recommends headphones and does no echo cancellation (settled 2026-11-24).
- Models come from the `ggerganov/whisper.cpp` repository on Hugging Face.
