# 001: Scaffold workspace and app shell

Status: todo · PRD: [00](../prds/00-foundation.md) · Lane: base · Depends on: — · Owns: whole repo (except `prds/`, `issues/`, `mneme-scope.md`)

## Goal

A repository every other issue can build in: Tauri 2 app, Cargo workspace with one crate per area, shared types, and a typed frontend API with a mock.

## Scope

1. **Cargo workspace** at the repo root with members `crates/mneme-core`, `crates/mneme-audio`, `crates/mneme-transcribe`, `crates/mneme-store`, `crates/mneme-pipeline`, `src-tauri`. Shared `[workspace.package]` (edition, MIT license, version `0.1.0`) and `[workspace.dependencies]` for crates used in more than one place.
2. **`mneme-core`** with the shared types exactly as in PRD 00: `SAMPLE_RATE`, `Speaker` (with `Display` giving `Me` / `Them` and `FromStr`), `AudioFrame`, `Segment`, `format_timestamp`, `parse_timestamp`. Serde derives. Unit tests for the timestamp functions, including hours above 99 and malformed input.
3. **Empty area crates**: `mneme-audio`, `mneme-transcribe`, `mneme-store`, `mneme-pipeline` each with a `lib.rs` holding a one-line crate doc and a dependency on `mneme-core`. No feature code.
4. **Tauri 2 app** in `src-tauri`: bundle identifier `app.mneme.desktop`, product name `Mneme`, minimum macOS 14.4, one main window (title `Mneme`, sensible default and minimum size), a strict content security policy with no remote sources. No commands yet.
5. **Frontend**: Vite + React + TypeScript with pnpm. Vitest and React Testing Library set up with one passing test. ESLint and Prettier with default rules.
6. **API module** at `src/lib/api/`:
   - `types.ts`: the contract from PRD 05, verbatim.
   - `mock.ts`: an in-memory implementation with a few seeded meetings (one interrupted), seeded transcript lines, a small model catalog with none or one installed (switchable), simulated download progress, and a working recording state with a ticking timer.
   - `tauri.ts`: a stub where every method throws "not implemented" (issue 021 fills it in).
   - `index.ts`: exports `api`, choosing Tauri when running inside Tauri and the mock otherwise.
7. **Placeholder UI**: one page that lists the mock meetings, to prove the wiring.
8. **Repo files**: `LICENSE` (MIT, Francesco Hayes, 2026), `README.md` (what Mneme is, how to build and run, pointer to PRDs and issues), `.gitignore` (target, node_modules, dist, model files, `.DS_Store`, WAV files).

## Acceptance criteria

- [ ] `cargo test --workspace` passes.
- [ ] `cargo build -p <crate>` works for each crate alone.
- [ ] `pnpm install && pnpm build && pnpm test && pnpm lint` pass.
- [ ] `pnpm tauri build --debug` (or the documented equivalent) compiles the app.
- [ ] The API types match PRD 05 exactly.
- [ ] No dependency that phones home; no analytics or updater plugin.

## Manual checks

None.

## Result

_Filled in by the implementer._
