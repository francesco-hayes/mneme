# 021: Tauri commands and events

Status: done (all four acceptance criteria met; `pnpm test` has one failing obsolete test outside this issue's files, see Result) · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 009, 007, 014 · Owns: `src-tauri/`, `src/lib/api/tauri.ts`

## Goal

Connect the UI to the real crates: every method in the API contract works in the built app.

## Scope

1. **App state**: store (meetings root from settings), settings store, model store (models folder in the app data dir), and the current session if any. One recording at a time.
2. **Commands** for every method in `src/lib/api/types.ts`, and `src/lib/api/tauri.ts` filled in to call them. Events for recording state (on change and once a second while recording) and download progress.
3. **Start meeting**: check a default model is installed and permissions allow it; create the meeting folder; load the model; build the mic and system sources; start the `Session` with the store's transcript writer. If system audio is denied, record mic only and say so in the returned state. Loading the model must not freeze the UI.
4. **Stop meeting**: stop the session, finish the meeting in the store (duration, status, pending rename), emit the new state.
5. **Save notes**: while that meeting is recording, save with the current meeting time so the note timeline is written; otherwise a plain save.
6. **Launch**: run store recovery before serving the first list.
7. **Quit while recording**: stop the session cleanly before exit.
8. **Show in Finder**: reveal the meeting folder.
9. Core logic lives in plain Rust functions that take the state, so tests run without a Tauri runtime. Errors cross to the UI as readable messages.
10. Network: confirm the app's capabilities and CSP still allow no remote access from the webview; the model download runs in Rust only.

## Notes from earlier issues

- Decided 2026-11-12: v1 is English only. Always start the session with `Language::english()`; ignore the `language` setting for transcription (it stays in the settings file and the contract, fixed at `en`). No language UI anywhere.

- Order changed 2026-11-12: this issue no longer waits for the last UI screen (020). It needs only the API contract in `src/lib/api/types.ts`. The UI lane may still be working in `src/` while this runs; touch nothing there except `src/lib/api/tauri.ts`, and expect Settings and First run to be placeholders or mid-change when you open the app window.

- From 001: `beforeDevCommand` in `tauri.conf.json` is `VITE_MNEME_API=mock pnpm dev`; set it back to `pnpm dev` once `tauri.ts` is real. Add the area crates to `src-tauri/Cargo.toml`. The mock's behavior where the contract is silent is listed in issue 001's Result ("Contract gaps"); match it or change the mock.
- From 005: a `WhisperEngine` still alive at process exit aborts the process (a Metal assert). Do not keep it in a static or in state that outlives quit; quitting mid-recording stops the session and drops the engine first.
- From 005: the first load of a model can take about 7 s while Metal compiles shaders; the UI needs a starting state rather than a frozen button.
- From 015: the recording bar shows `elapsedSecs` exactly as sent and does not count by itself, so emit the recording state about once a second.
- From 015: nobody has opened the real app window yet. Check the top bar against the native title bar, that `AccentColor` follows the system accent in WKWebView, and that styles load under `pnpm tauri dev` (Vite injects `<style>` tags; add a `devCsp` if they are blocked).
- From 015: a stop that bypasses the UI's `useRecording().stop()` (menu bar, shortcut) skips the editor's before-stop flush. Emit an event the UI can flush on before the backend stops, or accept that the editor's autosave covers it, and say which.

- From 010 and 026: two meetings in the same minute no longer clash; the second folder gets a numeric suffix (issue 026). The mock's `folderPath` does not model this and does not need to.
- From 010: store types do not derive `serde`; map them to the contract types in `src-tauri` (`id.as_str()`, `started_at.to_rfc3339()`, `duration_secs`, `status.as_str()`, `folder`).
- From 002: the frame channel is `std::sync::mpsc`; `request_microphone_permission()` blocks until the user answers, so call it off the UI thread.

- From 017: the editor runs in a shadow root so CodeMirror's styles are a constructed stylesheet, which `style-src 'self'` allowed in Chromium. This is unproven in WKWebView. In the app window check: the editor is styled; typing, selection, IME and copy/paste work inside the shadow root; `body`'s `-webkit-user-select: none` does not block the editor. If styles are blocked, the fallback is `EditorView.cspNonce` plus a per-load nonce in the CSP; report it rather than loosening the policy.
- From 017: `saveNotes` must be safe after `stopMeeting` and for meetings that are not recording, because late or retried saves can arrive then. `renameMeeting` receives a trimmed, non-empty title and the field shows the `title` returned.
- From 017: autosave flushes on editor blur, view unmount and `pagehide`. Check that WKWebView fires `blur` when the app is deactivated and `pagehide` when the window closes.

- From 012: if the session is recording this meeting, call `save_notes_at(&id, &notes, elapsed_ms)` on the same clock as segment `start_ms`; otherwise `save_notes`. `Err` means not saved. `SavedNotes::timeline_error` is something to log, not a failed save. Timeline entries are only written while status is `recording`, so the editor's before-stop flush must reach the store before `finish_meeting`.
- From 007: keep one `ModelStore` in app state, opened once at launch (opening deletes `.part` files). `downloadModel` is `start_download(id, |p| emit(p))` and drop the handle; `cancelDownload` is `cancel_download(id)`. The store does not know the default or the model in use: refuse deleting the model in use and clear `defaultModelId` in the command. Tests here cannot point the real downloader at a local server; use `open_with_catalog` with files placed in a temp folder.

- From 018: `getMeeting` called right after the recording state goes idle must already return the final status, `durationSecs` and `transcriptLineCount`; the view reads the meeting once at that moment. Emit idle only after the store has finished the meeting. In the app window, check with a long transcript: open and scroll speed of the pane, that Shift-Command-T reaches the page, that Edit > Copy runs the pane's copy handler, and that Select All stays inside the list.
- From 016: Home marks the live row from `status: recording` in `listMeetings` and re-reads when `RecordingState.meetingId` flips between an id and null.

- From 013: open one `Store` per root and keep it for the life of the process. Call `recover()` once before the first list and log `recovered`, `failed` and `warnings`. A meeting in `failed` is still `recording` on disk with nothing recording it; do not show it as live. `listMeetings` is `list_meetings()?.meetings` (log the warnings; the contract has no place for them). `getMeeting` is `load_meeting(&id)`. Show `Error::DuplicateId`'s message as is. A changed meetings root means a new `Store` plus `recover()` on it. Recovery assumes one running copy of the app per root: make the app single-instance.
- From 004: `Recorder::start` never blocks but `stop()` waits for devices to close; call it off the UI thread. A source that stays `Starting` for more than a second or two most likely has a permission prompt open. `PermissionDenied` as the first error of a `FailedToStart` source is the signal to send the user to System Settings.

- From 019: the UI applies the default-model rule itself through `updateSettings({ defaultModelId })`; the backend keeps leaving `defaultModelId` alone on install and clearing it when the default is deleted. Every Delete is disabled during a recording because `RecordingState` has no model id; adding one is optional. There is no "active downloads" read, so after a window reload a row reappears only with the next progress event. In the app window, look at `<progress>`, the radio buttons and the shortcut recorder.
- From 014: `SettingsStore::open(app_data_dir.join(SETTINGS_FILE), documents_dir.join("Mneme"))` once at launch, both paths absolute from Tauri's path API; log the returned warnings. `updateSettings` can take `SettingsPatch` as the command argument and return `Settings` (it also serializes `meetingsRoot`). The store checks nothing about meaning: refuse a `defaultModelId` that is not installed; a stored default may name a model no longer installed, so check before starting a meeting.

## Acceptance criteria

- [x] Tests with fake sources and a fake transcriber: start then stop produces a complete meeting on disk with transcript and duration; double start is refused; start with no model is refused with a clear message; notes saved during recording produce timeline entries; recovery runs on launch.
- [x] `src/lib/api/tauri.ts` implements the whole interface with no remaining stubs; `pnpm build` passes.
- [x] The app builds with `pnpm tauri build --debug`.
- [x] `cargo test -p mneme` (the app crate) and clippy pass.

## Manual checks

- [ ] In the built app: first run downloads a model; New meeting records; notes save; Stop gives a transcript; the meeting appears on Home; relaunch keeps everything.
- [ ] Force-quit mid-meeting, relaunch: the meeting is listed as interrupted with its transcript.

## Result

Implemented 2026-11-12 on macOS 27.0, Apple Silicon. The app was **not launched** and no recording or permission request was made (the user was away): everything below is proven at the logic level with fakes, plus a compiling bundle. Nothing about the real window or real audio has been seen.

### What was built

- `src-tauri/src/app.rs`: `AppCore`, the whole logic with no Tauri in it. Holds the `SettingsStore`, the `ModelStore`, the `Store` (replaced only when the meetings root changes) and the one recording (`Idle | Starting | Active | Stopping`). Everything that differs between the app and a test comes in through the `Host` trait (events, `load_transcriber`, `sources`, permissions, Finder, System Settings); folders are arguments to `AppCore::open(app_data_dir, documents_dir, host)`.
- `src-tauri/src/dto.rs`: the contract types with `serde`, mapped from the crates' types. A meeting that is `recording` on disk but not recorded by this run is reported as `interrupted`.
- `src-tauri/src/host.rs`: the real `Host`: `MicSource` + `SystemAudioSource`, `WhisperEngine::load`, the mneme-audio permission helpers, events `recording-state` and `download-progress`, `/usr/bin/open -R` for Finder (std only, no plugin).
- `src-tauri/src/commands.rs`: 17 commands, one per promise method of the contract, each run with `spawn_blocking` so the window never waits on a model load, a stop, file access or a prompt. `src-tauri/src/lib.rs`: setup, single-instance lock, exit hook.
- `src/lib/api/tauri.ts`: the whole `Api` with `invoke` and `listen`. A backend refusal (a plain string) is rethrown as an `Error` with that sentence, as the mock throws. Subscriptions return their unsubscribe function at once.
- `src-tauri/Info.plist` (merged into the bundle by Tauri): `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription`; both are in the built `Mneme.app`. `beforeDevCommand` is back to `pnpm dev`. Capabilities and CSP are unchanged: `core:default` already allows `listen`, and app commands need no permission entry. The three config tests pass unchanged.

### Behaviour decided here

- **Start:** refused unless idle (also while a model is still loading). Model: the default if installed, else the first installed one. Microphone `denied` refuses; `undetermined` asks first. Order is model load, then meeting folder, then session, so a failed load leaves no folder; if the session cannot start the meeting is closed, not left `recording`. Always `Language::english()`. The model is loaded per meeting and freed at stop, so no engine exists between meetings.
- **State events:** on every change and every second while recording (a thread per meeting, which also finishes the meeting if the session ends by itself). `idle` is sent only after `finish_meeting`. While the model loads the state is still `idle`; the pending `startMeeting` promise is the UI's "starting".
- **Stop:** `session.stop()`, drop the engine, `finalize_transcript` if the session could not, `finish_meeting(duration)`, then idle.
- **Notes:** `save_notes_at` with `session.elapsed()` while that meeting is recording **or stopping**; otherwise a plain save. On the 015 question (a stop that bypasses the UI's flush): no flush event is sent; the editor's autosave covers it, and a save that arrives while the stop is still draining still gets its timeline entry.
- **Quit:** `RunEvent::ExitRequested` and `RunEvent::Exit` both call `AppCore::shutdown()`: no new starts, wait for a start or stop under way, stop the recording (10 s for the transcriber to catch up, then `abandon_backlog`), finish the meeting, drop the engine.
- **Launch:** `recover()` runs inside `AppCore::open`, before any command is served. If the state cannot be opened the app still starts and every command fails with the reason.
- **Single instance:** a lock file (`mneme.lock` in the app data folder, `File::try_lock`); a second copy exits at once. No plugin.
- **Permissions** (minimal; 024 owns the flow): microphone status and request from mneme-audio. System audio is `undetermined` unless this run saw it work (a granted request, or the system source running in a meeting); a refusal is never reported on a guess. `requestPermission` returns what `getPermissions` then reports; on an already denied permission it opens the Privacy pane and returns `denied`. `tcc-spi` is off.
- **Models and settings:** `downloadModel` resolves when the download starts; `deleteModel` refuses the model in use (also while loading) and clears a default that pointed at it; `updateSettings` refuses a default that is not installed and is on disk before it returns.
- **Differences from the mock:** `startedAt` is RFC 3339 with the local offset, not `...Z` (the UI only parses it with `Date`). `getSettings` also carries `meetingsRoot`. `language` is whatever the file says (default `en`); transcription ignores it.

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme` | 20 passed (15 core, 1 contract JSON, 3 config, 1 instance lock), about 2 s |
| `cargo clippy -p mneme --all-targets -- -D warnings` | clean |
| `cargo fmt -p mneme -- --check` | clean |
| `pnpm build` | passes |
| `pnpm lint` | passes |
| `pnpm tauri build --debug` | `target/debug/bundle/macos/Mneme.app` built, usage strings present |
| `pnpm test` | **357 passed, 1 failed** (see below) |

**The failing test** is `src/lib/api/index.test.ts`, "every method of the Tauri stub reports that it is not implemented". It asserts the stub this issue removes. The file is outside this issue's files, so it was left alone. A replacement (three tests of the real `tauri.ts` with `invoke` and `listen` mocked) is written but **not run** at `/private/tmp/claude-501/-Users-francescohayes--claude-spares-spare-Fmb7kb/8b7c505f-861e-4310-8bb4-a869506fd923/scratchpad/issue-021/index.test.replacement.ts`. Until someone applies it, `tauri.ts` is covered only by the type check.

Acceptance tests (all in `src-tauri/src/app/tests.rs`, fake sources at real-time pace and a fake transcriber):
`start_then_stop_gives_a_complete_meeting_with_transcript_duration_and_note_timeline`, `a_second_start_is_refused_while_recording`, `a_second_start_is_refused_while_the_model_is_still_loading`, `start_without_an_installed_model_is_refused_with_a_clear_message`, `recovery_runs_on_launch_before_the_first_list`. Also: sources ending by themselves, shutdown while recording, microphone denied, model choice, settings, a new meetings root, models, permissions.

Four behaviours were broken once each and the change undone; each made tests fail: idle sent before the meeting is closed; a plain save during recording; no refusal of a second start; recovery not run at launch.

### Manual checks still open

Nothing here has been done. In the built app:

- First run downloads a model (real network, checksum), New meeting records, notes save, Stop gives a transcript, the meeting is on Home, relaunch keeps everything.
- Force-quit mid-meeting, relaunch: listed as interrupted with its transcript.
- Real `MicSource` and `SystemAudioSource` in a session: both prompts appear with the usage text, the app is not killed, "Them" lines appear, the app's own sounds are not captured.
- System audio refused: the meeting records the microphone only (stderr says so; the UI cannot).
- First model load (about 7 s): the button shows a pending state and the window stays responsive.
- Quit from the menu and by closing the window while recording: no Metal abort, the meeting is `complete` on relaunch.
- macOS asks for access to the Documents folder on first launch (the meetings root); the app has no usage string for it.
- Second launch of the app while one runs exits; whether macOS brings the first window forward.
- Show in Finder selects the meeting folder. "Open System Settings" lands on the Microphone pane.
- The window under the CSP in WKWebView: styles load in the built app and under `pnpm tauri dev` (no `devCsp` was added), the editor's shadow root is styled (017), fonts, the top bar against the native title bar, `AccentColor`, `<progress>`, radio buttons, the shortcut recorder.
- 017/018 items: typing, selection, IME, copy/paste in the editor; `blur` on app deactivation and `pagehide` on window close; a long transcript's scroll speed, Shift-Command-T, Edit > Copy, Select All.
- The recording bar counts once a second from the `recording-state` event.

### Known gaps

- **`pnpm test` is red** by the one obsolete test above.
- **"Mic only" is not told to the UI.** The contract has no field for it. A system source that fails to start is written to stderr and the meeting goes on.
- **System audio never reads `denied`** in the real app (no public API, `tcc-spi` off), so "Open System Settings" for system audio is unreachable; its pane anchor (`Privacy_AudioCapture`) is unverified.
- **`request_system_audio_permission` and `request_microphone_permission` were never called**, here or in any test.
- **The model is loaded for every meeting** (seconds each time). Keeping it between meetings would need care at exit.
- **Quit blocks the main thread** while the session stops: up to 10 s plus one chunk. The window shows nothing during it.
- **A stop cannot be cancelled or hurried from the UI**; a very long backlog keeps the state at `stopping`.
- **No "active downloads" read** (019's note stands). A download running at quit is cut off; its `.part` file is removed at the next launch.
- **Errors of a finished session** (failed chunks, dropped audio, lines not written) go to stderr only.
- **Not tested:** the commands and `host.rs` themselves (they need a Tauri runtime and devices); a note saved during `stopping`; the 10 s quit path with a slow transcriber; `finish_meeting` failing at stop; a meetings root that cannot be opened.
- The second copy of the app exits without bringing the first forward.
- Logging is `eprintln!`; there is no log file.

### Follow-ups

- **Orchestrator:** apply the replacement test to `src/lib/api/index.test.ts`. `Cargo.lock` changed (path dependencies, `tempfile`).
- **022 (menu bar), 023 (shortcut):** get the core with `app.state::<commands::AppState>().core` (a `Result<Arc<AppCore>, String>`). Start with `core.start_meeting()` and stop with `core.stop_meeting()`, both **off the main thread** (`tauri::async_runtime::spawn_blocking`); both block (model load, transcript drain) and return `Result<dto::Meeting, String>`. `core.recording_state()` is cheap and says which of the two a toggle should call (`idle` / `recording` / `stopping`). The window follows by itself through the `recording-state` event. A start refused with "A meeting is already being recorded." also covers a model still loading.
- **022:** `lib.rs` calls `core.shutdown()` on `ExitRequested`, which is final (no start afterwards). If the app is to stay in the menu bar when the window closes, call `api.prevent_exit()` there and keep `shutdown()` for `RunEvent::Exit` and the real Quit.
- **024:** entitlements; a usage string for the Documents folder; decide `tcc-spi`; a contract field or event for "recording the microphone only"; verify the System Settings anchors. `AppCore::request_permission` and `permissions` are the two entry points; `Host` is where a different system-audio check goes.
- **UI lane:** `startMeeting` can take seconds before it resolves (state stays `idle` meanwhile).
