# 031: The leftovers from 028

Status: done · PRD: [05](../prds/05-ui.md), [06](../prds/06-integration-and-finish.md) · Lane: several (app, ui) · Depends on: 028 · Owns: `src/`, `src/lib/api/*`, the API contract block in `prds/05-ui.md`, `src-tauri/src/tray.rs`, `src-tauri/src/dto.rs`, `src-tauri/src/lib.rs` (Reopen), the `Recording::state()` and `host.starting()` call site in `src-tauri/src/app.rs` and their tests

## Goal

Close three of the known gaps 028 left:

1. "Mic only" shows on the live meeting only. On Home or Settings during a meeting, nothing says the recording is missing a channel.
2. While a start from the menu bar or shortcut loads its model, the window's New meeting is still enabled, and pressing it is refused with "already being recorded".
3. Clicking the Dock icon only shows the window; the menu's Open Mneme also goes to the live meeting.

## Contract change

In `src/lib/api/types.ts` and the PRD 05 block, which stay identical:

- `RecordingState.starting: boolean`: a start is loading its model. Only with `status: 'idle'` and `meetingId` null; `modelId` names the model.

The status values and what `idle` means are unchanged: an idle state without `starting` still means no meeting is open, and is sent only once the last one is closed on disk. No method was added, so `tauri.ts` (which passes the payload through) and the method count in `index.test.ts` are unchanged.

A separate event was the alternative. It was not chosen because the window would then have to merge two streams and guess when "starting" ends; with a field, the next state (recording, or idle after a refusal) ends it by itself, and `getRecordingState` answers it too for a window that opens mid-load.

## Acceptance criteria

- [x] The recording bar shows a warning icon and "Mic only" when the warning is about system audio, or "Not all audio recorded" for any other channel. The full sentence is the tooltip and the screen-reader text; the bar stays one line. It is left out on the live meeting, which already shows the full sentence above the notes.
- [x] When a start begins loading, the backend sends an idle state with `starting: true` and `modelId` set. While it is set, Home's New meeting (top bar and empty state) reads "Starting…", is disabled with the title "A meeting is starting.", and Cmd-N does nothing. A start from the window shows the same "Starting…" while it loads.
- [x] A start that fails after loading began sends idle (without `starting`), and New meeting comes back.
- [x] Settings locks the Delete of the model being loaded (it was already refused by the backend), through the existing `modelId` rule. This also makes the PRD's sentence on `modelId` ("from the moment it starts loading") true; before, the backend sent null while loading.
- [x] The Dock icon (`RunEvent::Reopen`) calls `tray::open_mneme`, now `pub`: it shows the window and goes to the live meeting during a recording or stop.
- [x] `cargo test -p mneme`, `cargo clippy -p mneme --all-targets -- -D warnings`, `cargo fmt --all`, `pnpm test`, `pnpm lint` and `pnpm build` pass.

## Manual checks

- [ ] Turn off System Audio Recording for Mneme, start a meeting, go Home: the bar shows "Mic only", and hovering it shows the sentence. Turn the permission back on afterwards.
- [ ] With the window on Home, start a meeting from the menu bar: New meeting reads "Starting…" until the model has loaded, then the bar appears.
- [ ] During a recording, go to Settings, click the Dock icon: the window shows the live meeting.

## Result

**Built**

- Backend (`dto.rs`, `app.rs`): `RecordingState.starting`. `Recording::state()` reports `Starting` as idle with `starting` and `model_id`. `start_meeting` sends that state (`emit_state`) right before `host.starting()`; nothing else in the start path changed. The `Host::starting` doc says the window is now told.
- Menu bar (`tray.rs`): `open_mneme` is `pub`. `recording_state_changed` reads `state.starting`, so a starting state only redraws instead of also re-reading the model list.
- Dock (`lib.rs`): `RunEvent::Reopen` calls `tray::open_mneme`.
- UI: `RecordingBar` shows the short warning off the live meeting. `HomeScreen` disables New meeting while its own start or `state.starting` is loading, with the "Starting…" label. `IDLE_RECORDING` gains `starting: false`.
- Mock: `startMeeting` now sends the starting state before the recording state, as the backend does. New control `mock.startFromOutside(loadMs = 1500)`: a starting state, the load, then the recording; a refusal at once (no model, microphone denied) only emits a recording error, and a queued `failNext("startMeeting")` fails it after the load, back to idle with a recording error. `__mnemeMock.startFromOutside()` works from the console.

**Checks**

- Rust: 59 tests pass. The test host now leaves starting states out of "idle" (its `statuses()` names them `starting`, and it records what is on disk only for an idle that is not starting), so the idle-count and "idle only once closed on disk" assertions keep their meaning. New assertions: the full meeting test's first state is `starting` and there is exactly one; the model-loading test sees the starting state with `model_id` from the core and from the host; a microphone refusal sends `starting` then `idle`. Clippy and fmt clean.
- UI: 24 files, 393 tests pass. New: the bar's "Mic only" off the live meeting, with tooltip and screen-reader sentence, and nothing when both channels record; Home disabled with "Starting…" during an outside start, Cmd-N ignored, then the bar; Home back after an outside start fails; the mock's outside start (starting, refusal of the window's own start meanwhile, recording, failure after load, refusal at once). The mock tick test now expects the starting state first.
- Not seen in a browser or in the real app.

**Known gaps**

- None of it has been seen in the real app; the manual checks above are open. The Dock path is covered only by the parts it calls (`open_mneme` needs a running app).
- The Dock icon now goes to the live meeting on every click during a recording, also when the window is already visible on another screen. That matches Open Mneme; if it feels pushy, `Reopen`'s `has_visible_windows` could gate it.
- The short label is picked from the start of the backend's sentence ("System audio…"). If those sentences change, the label falls back to "Not all audio recorded", which is still true.
- Follow-up outside this issue's paths: the root `README.md` section on the mock lists its controls and does not mention `startFromOutside`.
