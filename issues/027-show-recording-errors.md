# 027: Show start and stop errors that come from outside the window

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: ui · Depends on: 022, 023 · Owns: `src/`, the API contract block in `prds/05-ui.md`

## Goal

A recording that fails to start or stop from the menu bar or the global shortcut must say why. Today the window opens and shows nothing.

## Background

Issue 022 requires that a start error from the menu bar "opens the window showing the error; it never fails silently". The backend half exists: `src-tauri/src/tray.rs` shows and focuses the window, then emits the event `recording-error` with the payload `{ action: "start" | "stop", message: string }`. The global shortcut (023) goes through the same function. The UI has no listener for that event, because the API contract has no method for it.

## Scope

1. **Contract**: add one subscription to `Api` in `src/lib/api/types.ts`, in the style of `onRecordingState`: `onRecordingError(cb: (e: RecordingError) => void): () => void`, with `interface RecordingError { action: 'start' | 'stop'; message: string }`. Make the same change to the contract block in `prds/05-ui.md` so the two stay identical.
2. **Tauri implementation** (`src/lib/api/tauri.ts`): listen to `recording-error` and pass the payload on.
3. **Mock** (`src/lib/api/mock.ts`): implement the subscription and add a control that emits an error, so the UI can be tested and tried in the browser.
4. **UI**: one app-wide subscription (beside the recording-state one). When an error arrives, show it inline and quietly, visible on whatever screen is open: the sentence, prefixed so it reads as a failed start or a failed stop, with a way to dismiss it. No popup, no toast. It clears when dismissed and when a recording next starts. A second error replaces the first.
5. Errors from a start or stop pressed inside the window are already shown where they happen (Home, the recording bar). Do not show those twice.

## Acceptance criteria

- [x] Tests: an emitted start error is shown on Home, on a meeting and on Settings; a stop error reads as a stop error; Dismiss removes it; a recording starting removes it; a second error replaces the first; an in-window start failure is not shown twice.
- [x] The Tauri implementation subscribes to `recording-error` (covered in `src/lib/api/index.test.ts` beside the existing subscription test; the method count there goes from 19 to 20).
- [x] `types.ts` and the contract block in `prds/05-ui.md` are identical apart from the `export` keywords.
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

- [ ] In the app: turn off Mneme's microphone access in System Settings, choose Start Recording from the menu bar icon. The window opens and says the microphone is not allowed. Turn access back on afterwards.

## Result

**Built**

- Contract: `RecordingError` and `Api.onRecordingError` in `src/lib/api/types.ts`, and the same two lines in the contract block of `prds/05-ui.md`.
- Tauri (`src/lib/api/tauri.ts`): `onRecordingError` listens to `recording-error` through the same `subscribe` helper as `onRecordingState`.
- Mock (`src/lib/api/mock.ts`): `onRecordingError`, and the control `mock.emitRecordingError(action, message?)` (in the browser console: `__mnemeMock.emitRecordingError("start")`). A start or stop made through the mock API still reports its failure only by rejecting, as the real backend does.
- State (`src/state/recording.ts`, `RecordingProvider.tsx`): `outsideError` and `dismissOutsideError` on the recording context, fed by the app's one `onRecordingError` subscription. A new error replaces the old one. It is cleared when a recording state arrives for a meeting other than the last one seen, which is a recording starting and not the once-a-second tick of one going on.
- UI (`src/shell/RecordingErrorNotice.tsx` and its CSS module, placed in `AppShell`): **one line at the bottom of the window, directly above the recording bar** (at the very bottom when nothing is being recorded). It reads "Could not start recording: …" or "Could not stop recording: …" in the `InlineError` style (`role="alert"`), with a plain Dismiss button. It is outside the screen area, so it shows on every screen and takes a line from the bottom of the editor instead of covering it. No animation.
- No duplicates: only the backend event feeds the notice. Home keeps showing its own failed start and the recording bar its own failed stop; neither reaches the notice.

**Checks**

- `pnpm test`: 24 files, 372 tests pass. New: `src/shell/RecordingErrorNotice.test.tsx` (11 tests: shown on Home, a meeting and Settings; stays across navigation; stop wording; Dismiss; cleared by a recording starting; second replaces first; in-window start and stop failures shown once) and one test in `src/lib/api/index.test.ts` (method count 20, `recording-error` subscribed and unsubscribed).
- `pnpm build` and `pnpm lint` pass, after `pnpm format`.
- Contract parity: a script stripped `export ` from `types.ts` and compared it with the PRD block: identical, 35 lines.
- Rules broken once each, then restored: never clearing on a new recording failed "a recording starting removes it"; clearing on every recording state failed the stop-error test; feeding in-window failures into the notice failed both shown-once tests.
- Browser, `pnpm dev` on the mock: start error on Home in light, stop error on a live meeting in dark, above the recording bar, editor not covered, no console errors. Server stopped afterwards.

**Known gaps**

- Not run in the real app: the manual check above is open. That the backend event reaches this listener in the built app is covered only by the mocked `listen` test.
- If the notice is showing and a start pressed inside the window then fails, Home shows its own error and the older notice stays until dismissed: two lines, for two attempts.
- If the window loads while a recording is running and an error arrives before the first state is known, the first tick clears it. Narrow, and not tested.
- A long sentence wraps and the line grows; this was not looked at in the browser at the minimum window size.
- The mock section of the top-level `README.md` does not list `emitRecordingError`; that file is outside this issue's lane.
