# 020: First run

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 019 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

Get one model installed and permissions granted before the first meeting, in as few steps as possible.

## Scope

1. Shown by the gate from issue 015 whenever no model is installed.
2. **Step 1, model**: a short explanation that transcription runs on this Mac and needs a model. A recommended model is preselected (take the recommendation from the benchmark doc if issue 008 has produced one; otherwise a small multilingual model, marked as a placeholder choice). The full list is available with size and speed notes. Download with progress, checksum verification state, cancel and retry, reusing the Settings pieces.
3. **Step 2, permissions**: microphone and system audio, each with one sentence on why, current status from `api.getPermissions`, and a button calling `api.requestPermission`. A denied permission explains how to fix it in System Settings. System audio can be skipped (recording then captures only me); the microphone cannot.
4. A one-line recommendation to use headphones, with the reason (without them the mic hears the other side).
5. **Finish** goes to Home. The downloaded model is the default.
6. Quitting mid-download and reopening returns to first run cleanly.
7. One sentence stating that nothing leaves this Mac and the download is the only network use.

## Acceptance criteria

- [x] Tests: gate shows first run with no models; download flow reaches the permissions step; denied and granted states render; system audio skip is allowed and mic skip is not; finishing lands on Home with the default model set; a failed download can be retried.
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. No dependency was added; `src/lib/api/` (types, mock, tauri), the shared components and the model library are unchanged.

### What was built

```
src/screens/
  FirstRunScreen.tsx            the screen (replaces the placeholder): loads the library, picks the step, the privacy sentence
  FirstRunScreen.test.tsx       18 tests
  firstRun/
    ModelStep.tsx               step 1: recommended model preselected, full list on request, one Download button
    PermissionsStep.tsx         step 2: microphone and system audio rows, headphones line, Finish
    FirstRun.module.css         styles on the tokens
```

- **The step is derived, not stored.** It is the model step while `useModelLibrary().defaultModel` is undefined and the permissions step once it is set. The library sets the default when the first download finishes, so first run has no default-model code, and reopening the app mid-download starts at the model step with nothing left over.
- **Step 1.** "Choose a transcription model", with one sentence that transcription runs on this Mac and needs a model. `small` is preselected with a "Recommended" badge (`RECOMMENDED_MODEL_ID` in `ModelStep.tsx`, commented as a placeholder until the benchmark in issue 008 is finished). Only that row is shown until "Show all models" opens the whole catalog with sizes and notes; each row has a radio button. One primary button under the list, "Download <name>", starts the download. The rows are `ModelRow`/`ModelList` from Settings: progress, "Verifying…", Cancel, and after a failure the reason with Retry and Dismiss. While a download runs the radios are disabled. If the default could not be saved, the step stays, with "Could not set the default model: <reason>" and Retry.
- **Step 2.** "Allow recording": a `Group` with a row per permission, each with one sentence on why. Not asked yet: an "Allow…" button calling `api.requestPermission`. Granted: a check and "Allowed". Denied: "Not allowed", the path in System Settings under the row (Privacy & Security > Microphone; Privacy & Security > Screen & System Audio Recording > System Audio Recording Only, plus "quit Mneme and open it again" for system audio, from issue 003), and an "Open System Settings" button that also calls `api.requestPermission` (PRD 06: it opens the right pane once denied). The status is always what `api.getPermissions` says: read on entry, after every request, and when the window gets the focus back.
- **Finish** is disabled until the microphone is granted, with "Mneme cannot record without the microphone. Allow it to finish." With the microphone granted and system audio not, the button reads "Finish without system audio" beside "Without system audio, a recording has only your side." It calls `navigation.reset({ name: "home" })`.
- One line: "Use headphones: without them the microphone also hears the other side of the call." One sentence under both steps: "Nothing you record leaves this Mac. Downloading the model is the only time Mneme uses the network."

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm build` | Pass. JS 588.49 kB (gzip 189.05 kB), CSS 20.62 kB (gzip 4.45 kB). |
| `pnpm test` | Pass: 23 files, 358 tests (18 new, all in `src/screens/FirstRunScreen.test.tsx`). |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| Tests fail when the code is broken | Five single changes, each restored: Finish enabled without the microphone (3 tests failed); Finish also requiring system audio (2); Finish not leaving (3); downloading the recommended model whatever is selected (1); moving to permissions before the default is saved (0 at first, so the test "the permissions step waits until the model has been saved as the default" was added, and then 1). |

Tests per acceptance item: gate with no models ("with no model installed the app opens on First run…", through `<App />`; `src/App.test.tsx` also still covers it); download reaches permissions ("downloading shows progress, then verifying, then the permissions step, with the model as the default", which also asserts no `style` attribute and no `<style>` element); denied and granted states (three tests, plus "answering no to the prompt…"); system audio skip allowed (undetermined and denied) and microphone skip not (undetermined and denied, the latter recovering on window focus); Finish lands on Home with the default set ("the whole of first run…", through `<App />`); failed download retried. Also: the full list and choosing another model, Cancel, reopening mid-download, a failed permissions read.

Browser check in the coding tool's Chromium pane against `pnpm dev` at `http://localhost:1420/?mock=first-run`, 960x680, `prefers-color-scheme` emulated; the server was stopped afterwards. Light: step 1 collapsed and expanded, a download just started (bar, Cancel, radios disabled), step 2 with both permissions not asked, and with the microphone allowed and system audio denied. Dark: step 1 collapsed, step 2 with system audio denied (before and after allowing the microphone), and Home after "Finish without system audio". No console errors, no `style` attributes.

### Known gaps

- **Only the mock.** No real download, no real permission prompt. Nothing ran in WKWebView or Safari; `pnpm tauri dev` was not run.
- **System audio status depends on the backend.** The step shows whatever `getPermissions` returns after `requestPermission` resolves. Issue 003 notes that without `tcc-spi` the real status may be unknown or read "denied" while the prompt is still open; then this step would show "Not allowed" too early. Not handled here.
- **"Open System Settings" is `requestPermission` on a denied permission**, as PRD 06 describes. The mock does nothing there, so the button was only seen to exist.
- **Quitting after the download but before Finish** opens Home next time (the gate only looks at installed models), so the permissions step is not shown again. Home's own handling of a denied microphone then applies.
- **Focus is not moved when the step changes**: the Cancel button that had it is gone, and focus falls to the page. Not announced to a screen reader either; none was used.
- **Not looked at in the browser:** the failed-download row, verifying, the default-save error and the permissions read error on this screen (tests only); 640x480; the all-granted state.
- A download started outside this window (seen only through progress events) shows its row and disables the choice, by reasoning from the store's behavior; not tested here.

### For 021

- First run calls `getPermissions` on entering step 2, after every `requestPermission`, and on every window `focus`. It ignores `requestPermission`'s return value and trusts the following `getPermissions`, so the two must agree by the time `requestPermission` resolves.
- `requestPermission(kind)` on a denied permission is expected to open the matching System Settings pane and resolve (not reject). A rejection is shown as "Could not ask for the microphone: <reason>".
- The default model is saved by the model library through `updateSettings({ defaultModelId })` right after the `done` progress event; first run does not advance until `getSettings` reports it.
- The recommended model id is the literal `"small"`; if the catalog lacks it, the first model in the catalog is preselected instead.
