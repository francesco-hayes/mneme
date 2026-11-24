# 028: Spellcheck setting, and four small recording fixes

Status: done · PRD: [05](../prds/05-ui.md), [06](../prds/06-integration-and-finish.md) · Lane: several (store, app, ui) · Depends on: 024, 027 · Owns: `crates/mneme-store/src/settings.rs`, `src-tauri/`, `src/`, the API contract block in `prds/05-ui.md`

## Goal

Settle the spellcheck decision with a setting, and fix four weak spots from [known-gaps.md](../docs/known-gaps.md) that would be noticed in daily use:

1. Spellcheck in the notes editor is off and cannot be turned on.
2. "Mic only" (system audio refused) is not shown in the UI; it only goes to the log.
3. Open Mneme in the menu bar does not go to the live meeting.
4. Every Delete in Settings is locked during any recording, because the UI does not know which model is in use.
5. While a meeting is still loading its model, the menu still offers Start, and choosing it is refused.

Asked for by Francesco on 2026-11-24. Run in one session rather than per lane, because every fix touches the contract.

## Contract changes

In `src/lib/api/types.ts` and the PRD 05 block, which stay identical:

- `Settings.spellcheck: boolean`, off by default.
- `RecordingState.modelId: string | null`: the model the recording uses, while recording or stopping.
- `RecordingState.warning: string | null`: one sentence saying a channel is not being recorded.
- `Api.onOpenMeeting(cb: (meetingId: string) => void)`: the event `open-meeting`.

The status values are unchanged. A start that is loading its model still reads `idle` to the window, because "idle" also means "the meeting is closed on disk" (021), and the tests rely on that.

## Acceptance criteria

- [x] Spellcheck: a "Check spelling" switch in Settings (a Notes section) saves `spellcheck` at once and shows a refusal. The notes editor marks misspelled words when it is on, follows a change while open, and keeps autocorrect and autocapitalize off either way. The settings file stores `"spellcheck": true/false`; a value of the wrong type is a warning and falls back to off.
- [x] Mic only: the live meeting view shows the backend's sentence with a warning icon for as long as the meeting records. Set at the start when system audio is already known to be refused, and by the watcher when a source fails. A start warning is taken back if the system source delivers audio after all.
- [x] Open Mneme during a recording or stop shows the window and goes to the live meeting, unless it is already on screen.
- [x] Settings locks Delete only for the model the recording uses, with a caption naming it. A Delete question open on that model is withdrawn when a recording starts with it; questions on other models stay.
- [x] Menu bar: while a start loads its model the first line reads "Starting…" and is disabled, and the shortcut does nothing (it used to try a second start and show "already being recorded").
- [x] `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm build`, `pnpm test`, `pnpm lint` and `scripts/audit-local-only.sh` pass.

## Manual checks

- [ ] In the app: turn spellcheck on in Settings, open a meeting, type a misspelled word: it is underlined. Turn it off: the underline goes.
- [ ] Turn off System Audio Recording for Mneme, start a meeting: the warning shows above the notes. Turn it back on afterwards.
- [ ] Start a meeting, go to Settings, choose Open Mneme from the menu bar: the window shows the live meeting.
- [ ] During a recording, Settings: only the model in use has Delete dimmed.
- [ ] Start a meeting from the window and open the menu bar menu while the model loads: it reads "Starting…" and cannot be clicked.

## Result

**Built**

- Store (`crates/mneme-store/src/settings.rs`): the `spellcheck` key, `Settings.spellcheck`, `SettingsPatch.spellcheck: Patch<bool>` (clear removes the key; off is written as `false`). Tests in `tests/settings.rs`: a new test for the key, and the wrong-type and contract-name tests extended.
- Backend (`src-tauri/src/app.rs`, `dto.rs`): `RecordingState` gains `model_id` and `warning`. `Live` holds the warning; the start decision's mic-only sentence goes there as well as to the log, and the watcher adds the first channel-lost sentence. `AppCore::is_starting`, and a `Host::starting` hook (default no-op) called when a start begins loading.
- Menu bar (`src-tauri/src/tray.rs`, `host.rs`): a `Starting` phase from the core's state; "Starting…" disabled, idle icon, no title, no toggle. Open Mneme emits `open-meeting` with the live meeting's id after showing the window.
- UI: `tauri.ts` subscribes to `open-meeting`. `AppShell` navigates on it. `MeetingScreen` shows `warning` in the notices style. `ModelPicker` takes `inUseId` instead of `locked`. `NotesEditor` takes `spellcheck` (a CodeMirror compartment, so it changes in place); `MeetingNotes` reads it from the model library and asks it to load once if nothing has. Settings has a Notes section with the switch.
- Mock: the new fields, `MOCK_MIC_ONLY_WARNING` when system audio is denied at start, `onOpenMeeting`, and `mock.emitOpenMeeting(meetingId?)` (`__mnemeMock.emitOpenMeeting()` in the console).

**Checks**

- Rust: workspace tests pass; new `app::tests::the_state_says_why_system_audio_is_missing_until_it_is_heard`, `is_starting` asserts in the model-loading test, two new tray tests. Clippy and fmt clean. Local-only audit passes.
- UI: 24 files, 388 tests pass. New tests for the editor's spellcheck attribute, the Settings switch (save and refusal), the live warning (shown, gone after stop, absent when both channels record), spellcheck reaching the editor, Open Mneme navigation, and the in-use Delete rule (replacing the "nothing can be deleted" test).
- Browser, on the mock: the warning above the notes in dark mode; Settings during a recording with only Base's Delete dimmed and its caption; the switch on; `emitOpenMeeting` from Settings landing on the live meeting with `spellcheck="true"` in the editor. No console errors.

**Known gaps**

- None of it has been seen in the real app; the manual checks above are open. The `open-meeting` event and the "Starting…" menu line are covered only by tests of their parts.
- Whether WKWebView underlines words with `spellcheck="true"` inside the editor's shadow root was not seen. If it does not, the attribute is right and the cause is the webview.
- The warning is shown on the live meeting only. On Home or Settings during a meeting, nothing says the recording is mic only.
- Clicking the Dock icon (`RunEvent::Reopen`) still only shows the window; only the menu's Open Mneme goes to the meeting.
- During a model load started from the menu bar or shortcut, the window's New meeting button is still enabled, and pressing it is refused with "already being recorded".
