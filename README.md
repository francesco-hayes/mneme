# Mneme

Local-only meeting notes for macOS. Mneme listens to a meeting, transcribes it on your Mac with a Whisper model, and keeps your notes and the transcript as plain Markdown files. Nothing leaves the machine: no accounts, no telemetry, and no network use apart from downloading the model you pick.

Mneme is under construction. The parts exist and are tested on their own: audio capture, the Whisper engine and model downloads, storage, and the UI screens running on a mock API. They are not joined yet. **The app's backend commands are not wired (issue 021)**: `src/lib/api/tauri.ts` is a stub, so a built app opens its window but every call to the backend fails with "not implemented". Use `pnpm dev` or `pnpm tauri dev`, which run the UI on the mock, to see the screens.

- What it will do and why: [mneme-scope.md](mneme-scope.md)
- Requirements per area: [prds/](prds/README.md)
- Work items and their status: [issues/](issues/README.md)

## Requirements

- macOS 14.4 or later on Apple Silicon
- Rust (stable)
- The Xcode Command Line Tools (`xcode-select --install`); full Xcode is not needed
- cmake (`brew install cmake`), to build whisper.cpp
- Node 24 or later and pnpm 11

The Tauri CLI is a dev dependency, so nothing is installed globally.

## Build and run

```sh
pnpm install

pnpm dev            # UI in the browser at http://localhost:1420, on the mock API
pnpm tauri dev      # UI in the app window, on the mock API for now (see below)
pnpm tauri build --debug   # debug app at target/debug/bundle/macos/Mneme.app
pnpm tauri build           # release app at target/release/bundle/macos/Mneme.app
```

The first Rust build compiles whisper.cpp and takes several minutes. Builds are unsigned unless you ask otherwise, which is fine on the Mac that built them. An unsigned app is asked for its microphone and system-audio permission again after every rebuild; [docs/release.md](docs/release.md#dev-signing) describes the one-time dev certificate that stops that, and the Developer ID signing and notarization for a release (which needs an Apple Developer account and has not been run).

### Development tools

Command-line tools that exercise one crate without the app:

- `mneme-record` (in `crates/mneme-audio`): records the microphone and system audio to `mic.wav` and `system.wav`. `cargo run -p mneme-audio --bin mneme-record -- --help`; see [crates/mneme-audio/README.md](crates/mneme-audio/README.md).
- `mneme-bench` (in `crates/mneme-transcribe`): times a Whisper model on a WAV recording and reports whether it keeps up with a live meeting. `cargo run --release -p mneme-transcribe --bin mneme-bench -- --help`.
- `mneme-models` (in `crates/mneme-transcribe`): lists the model catalog and downloads a model for tests and `mneme-bench`. `cargo run -p mneme-transcribe --bin mneme-models -- --help`.

## Checks

```sh
pnpm build          # type-check and bundle the UI
pnpm test           # UI tests (Vitest, React Testing Library)
pnpm lint           # ESLint, then Prettier in check mode
pnpm format         # apply Prettier

cargo test --workspace
cargo test -p mneme-core    # one crate; the app crate is `mneme`

scripts/audit-local-only.sh # fails if network code appears outside the model downloader
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
scripts/              dev signing, release, local-only audit
docs/release.md       build, sign, notarize, and the manual test list
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
mockApi.mock.setInstalledModels(["base.en"]); // switch without resetting
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

`scripts/audit-local-only.sh` checks all three against the dependency tree and the Tauri configuration; [docs/release.md](docs/release.md#local-only-audit) says what it covers and what it cannot see.

## License

[MIT](LICENSE)
