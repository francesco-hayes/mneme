# 021: Tauri commands and events

Status: todo · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 009, 007, 014 · Owns: `src-tauri/`, `src/lib/api/tauri.ts`

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

## Acceptance criteria

- [ ] Tests with fake sources and a fake transcriber: start then stop produces a complete meeting on disk with transcript and duration; double start is refused; start with no model is refused with a clear message; notes saved during recording produce timeline entries; recovery runs on launch.
- [ ] `src/lib/api/tauri.ts` implements the whole interface with no remaining stubs; `pnpm build` passes.
- [ ] The app builds with `pnpm tauri build --debug`.
- [ ] `cargo test -p mneme` (the app crate) and clippy pass.

## Manual checks

- [ ] In the built app: first run downloads a model; New meeting records; notes save; Stop gives a transcript; the meeting appears on Home; relaunch keeps everything.
- [ ] Force-quit mid-meeting, relaunch: the meeting is listed as interrupted with its transcript.

## Result

_Filled in by the implementer._
