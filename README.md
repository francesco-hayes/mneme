# Mneme

Local-only meeting notes for macOS. Mneme listens to a meeting, transcribes it on your Mac with a Whisper model, and keeps your notes and the transcript as plain Markdown files. Nothing leaves the machine: no accounts, no telemetry, and no network use apart from downloading the model you pick.

Mneme is under construction. This repository currently holds the skeleton: a Tauri 2 app, a Cargo workspace with one crate per area, and a typed frontend API with a mock. The window shows a placeholder list of mock meetings.

- What it will do and why: [mneme-scope.md](mneme-scope.md)
- Requirements per area: [prds/](prds/README.md)
- Work items and their status: [issues/](issues/README.md)

## Requirements

- macOS 14.4 or later on Apple Silicon
- Rust (stable) and the Xcode Command Line Tools
- Node 24 or later and pnpm 11

The Tauri CLI is a dev dependency, so nothing is installed globally.

## Build and run

```sh
pnpm install

pnpm dev            # UI in the browser at http://localhost:1420, on the mock API
pnpm tauri dev      # UI in the app window, on the mock API for now (see below)
pnpm tauri build --debug   # debug app at target/debug/bundle/macos/Mneme.app
```

## Checks

```sh
pnpm build          # type-check and bundle the UI
pnpm test           # UI tests (Vitest, React Testing Library)
pnpm lint           # ESLint, then Prettier in check mode
pnpm format         # apply Prettier

cargo test --workspace
cargo test -p mneme-core    # one crate; the app crate is `mneme`
```

## Layout

```
Cargo.toml            workspace root: members, shared version and dependencies
crates/
  mneme-core/         shared types: Speaker, AudioFrame, Segment, timestamps
  mneme-audio/        microphone and system audio capture
  mneme-transcribe/   Whisper engine, silence chunker, model downloads
  mneme-store/        meeting folders, transcript, notes, settings
  mneme-pipeline/     live session: capture to transcript
src-tauri/            the Tauri app (crate `mneme`): window, config, icons
src/                  React and TypeScript UI
  lib/api/            the only way the UI talks to the backend
prds/  issues/  mneme-scope.md
```

Each crate builds and tests alone with `cargo build -p <crate>` and `cargo test -p <crate>`. The area crates depend on `mneme-core` and not on each other.

## The API module

The UI imports one object, `api`, from `src/lib/api`. Its shape is the contract in `src/lib/api/types.ts`, which is copied from [PRD 05](prds/05-ui.md#api-contract). There are two implementations:

- `mock.ts`: in-memory data, used in the browser dev server and in tests.
- `tauri.ts`: calls the Rust backend. It is a stub until issue 021; every method fails with "not implemented".

`index.ts` picks Tauri inside the app and the mock everywhere else. Setting `VITE_MNEME_API=mock` or `VITE_MNEME_API=tauri` when starting Vite overrides that. `pnpm tauri dev` sets it to `mock` (see `beforeDevCommand` in `src-tauri/tauri.conf.json`), so the app window shows working screens before the backend exists. Issue 021 removes that override. A built app always uses the Tauri implementation.

### The mock API

The mock starts with four meetings (one of them interrupted, one with an empty transcript), a catalog of five models with one installed, and both permissions granted. Recording runs a timer that ticks once a second and adds a transcript line every five seconds. Downloads report progress through `downloading`, `verifying` and `done`, and can be cancelled.

In the browser, pick a starting point with the query string:

| URL                      | Starts with                                             |
| ------------------------ | ------------------------------------------------------- |
| `/`                      | One model installed, four meetings                      |
| `/?mock=first-run`       | No model, no meetings, permissions not asked yet        |
| `/?mock=empty`           | One model installed, no meetings                        |
| `/?mock=long-transcript` | The default plus a meeting with a 2,000-line transcript |
| `&mockLatency=300`       | Any of the above, with every call delayed by 300 ms     |

While the dev server runs, the controls are also on `__mnemeMock` in the browser console, for example `__mnemeMock.setInstalledModels([])` or `__mnemeMock.failNextDownload()`.

In tests, import the same instance the app uses and drive it through `mock`. The test setup resets it after every test.

```ts
import { mockApi, mockScenarios } from "./lib/api/mock";

mockApi.mock.reset({ installedModelIds: [] }); // no model installed
mockApi.mock.reset(mockScenarios["first-run"]); // or a named scenario
mockApi.mock.setInstalledModels(["base"]); // switch without resetting
mockApi.mock.setPermission("microphone", "denied");
mockApi.mock.failNext("saveNotes", "Disk full."); // the next call rejects
mockApi.mock.failNextDownload(); // the next download ends in `error`
mockApi.mock.putMeeting({ ... }); // add or replace a meeting
```

For a private instance with its own timing, call `createMockApi(options)`. `MockOptions` in `mock.ts` lists every option: the clock, the seed, tick length, stop delay, download speed and latency. With Vitest fake timers, `vi.advanceTimersByTimeAsync(3000)` moves a recording forward three seconds.

## Local-only rules

- No analytics, crash reporting, update checks or updater plugin.
- The only code allowed to open a network connection is the model downloader in `mneme-transcribe`.
- The webview's content security policy allows the app's own files and its channel to the Rust side, and nothing remote. A test in `src-tauri` fails if a remote source is added.

## License

[MIT](LICENSE)
