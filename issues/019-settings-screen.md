# 019: Settings screen

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 018 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

MVP feature 5 and the model manager: pick the default model, manage downloads, set the global shortcut.

## Scope

1. **Models**: every catalog model from `api.listModels` with name, size, note and an English-only tag. Installed models show a Delete action (with a confirm) and can be chosen as the default. Others show Download.
2. **Downloads**: progress bar with received and total size from `api.onDownloadProgress`; a verifying state for the checksum; Cancel; errors shown on the row with Retry. Progress survives leaving and returning to Settings while the download runs.
3. **Default model**: one installed model is the default, saved with `api.updateSettings`. The first installed model becomes the default automatically. Deleting the default moves the default to another installed model, or to none.
4. The model in use by a running recording cannot be deleted.
5. ~~**Language**~~: removed 2026-11-12. v1 is English only; Settings has no language control.
6. **Global shortcut**: a recorder field. Unset by default, shown as "None". Click, press a combination, saved through `api.updateSettings`; a Clear button. Requires at least one modifier. A registration error from the backend is shown and the previous value restored.
7. Reusable pieces (model row, download progress) are exported for First run (issue 020).

## Notes from earlier issues

- From 007: the real catalog has ten models: `tiny`, `tiny.en`, `base`, `base.en`, `small`, `small.en`, `medium`, `medium.en`, `large-v3-turbo`, `large-v3` (77 MB to 3.1 GB). The mock's five ids are all valid; align the mock's catalog with the real one (ids, names, sizes, notes are in issue 007's Result and `crates/mneme-transcribe/src/models/catalog.rs`, which you may read but not edit) so the list is designed for ten rows.
- From 007: with the real backend, `cancelled` arrives shortly after `cancelDownload` returns, not at once, and can take up to about 35 s if the connection was still being set up. Show a cancelling state. The first `downloading` event may arrive just after `downloadModel` resolves. `verifying` is brief.
- From 007: deleting a model that is not installed is an error ("Base is not installed."). Disk space is not checked before a download; a full disk arrives as a download error.
- From 005: the engine validates language codes but exports no list of languages; the UI carries its own list.

## Acceptance criteria

- [x] Tests: list rendering; download progress through to installed; cancel; error and retry; first install becomes default; deleting the default reassigns; shortcut record, clear, modifier rule and error restore.
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. No dependency was added; `package.json`, the lockfile, the config files, `src/lib/api/types.ts` and `src/lib/api/tauri.ts` are unchanged.

### What was built

```
src/
  screens/
    SettingsScreen.tsx           the screen (replaces the placeholder): loading, load error, the two sections
    SettingsScreen.test.tsx      37 tests
    settings/
      ModelsSection.tsx          Installed (default radio, Delete with the inline question) and Available to download
      ShortcutRecorder.tsx       the recorder field and Clear
      useSettingSaver.ts         one settings field for a control that saves on change: shown at once, restored on error
  state/
    modelLibrary.ts              ModelLibraryStore, ModelLibraryContext, useModelLibrary   (+ 32 tests)
    ModelLibraryProvider.tsx     creates the store once, above the screens
  lib/
    models.ts                    DownloadStatus, applyDownloadEvent, pickDefaultModel, modelNote, isDownloadActive (+ 15 tests)
    shortcut.ts                  captureShortcut, describeShortcut, the string format   (+ 14 tests)
    format.ts                    + formatBytes ("77.7 MB", "1.53 GB", "3.1 GB")          (+ 3 tests)
  components/                    exported from "../components"
    ModelRow.tsx                 ModelRow, ModelList                                       (+ 12 tests)
    DownloadProgress.tsx         the bar and its line of text
    Group.tsx                    Section, Group, GroupRow: the Mac settings group box
    icons.tsx                    + CheckIcon
  styles/tokens.css              + --color-group, --color-group-border
  App.tsx, test/render.tsx       ModelLibraryProvider added inside RecordingProvider
  lib/api/mockSeed.ts, mock.ts   the ten-model catalog; a cancel delay (see "Mock changes")
```

- **Layout.** A centered column of two sections in grouped boxes: "Transcription models" and "Shortcut". Models are split into **Installed** (each row: a radio button that makes it the default, the name, an "English only" tag where it applies, a "Default" tag on the chosen one, the catalog's note, the size, Delete) and **Available to download** (the same row with Download), both in catalog order, smallest first. A model moves from the second box to the first when its download finishes. Sizes are in decimal units with three significant digits, as Finder shows them. The catalog's notes are shown as written, minus the closing "English only." on the `.en` models, which the tag says.
- **Row states** (`ModelRow`, driven only by props): not installed (Download); starting (bar with no value, "Starting…", Cancel disabled until the backend has answered `downloadModel`); downloading (bar, "244 MB of 488 MB", Cancel); verifying (full bar, "Verifying…"); cancelling (bar kept and grayed, "Cancelling…", Cancel disabled); failed ("Could not download Small: <reason>", Retry and Dismiss); installed (Delete); installed and default (checked radio and the tag); locked while a recording runs (Delete disabled).
- **Delete** asks inside the row: "Delete Small from this Mac?" with Keep and Delete, and when it is the default also says what happens ("Base becomes the default." or "No model will be left, and Mneme needs one to record."). Focus opens on Keep; Escape or Keep closes it and returns focus to Delete. No browser dialog. A failed delete is shown on the row.
- **Global shortcut.** A field showing the shortcut in Mac symbols (`⇧⌘R`) or "None". Click it (or Space/Return on it), press a combination, and it is saved with `api.updateSettings`; held modifiers show as they go down. Clear removes it. Escape, clicking again or moving the focus away cancels. Tab is never taken. A key with no modifier, or with Shift alone (function keys excepted), is refused with a one-line reason and the field keeps listening. A change the backend refuses shows "Could not set the shortcut: <reason>" and the field shows the previous value again.
- **States of the screen.** "Loading settings…" only after 400 ms (`.appears-late`); a failed read shows the reason with Retry. Returning to Settings shows what was read before at once and reads again in the background.

### The shared download store: `src/state/modelLibrary.ts`

`ModelLibraryStore` (a plain class, no React in it) holds the app's **one** subscription to `api.onDownloadProgress`, the model list, the settings and a `downloads` map (model id to `{ phase, receivedBytes, totalBytes, error }`). `ModelLibraryProvider` creates it once in `App`, above the navigation, exactly as `RecordingProvider` does for the recording state, so a download keeps its progress on any screen. Screens read it with `useModelLibrary()` (`useSyncExternalStore`), which returns the state plus `load`, `download`, `cancelDownload`, `dismissDownloadError`, `deleteModel`, `updateSettings` and `defaultModel`. The store reads nothing until a screen calls `load()`, so screens that do not show models cost no API call.

The state machine is the pure function `applyDownloadEvent` in `src/lib/models.ts`. Points taken from 007: once Cancel is pressed the row stays `cancelling` through any further `downloading`/`verifying` events until `cancelled` arrives, or `done` if the cancel came too late (the model is then installed); an `error` while cancelling counts as the stop; a `downloading` event that arrives after `downloadModel` resolves is handled; an event for a download this window did not start (a reload mid-download) creates the row's status.

### Default-model rules: one function, applied in one place

`pickDefaultModel(models, currentDefaultId, preferredId)` in `src/lib/models.ts`: the current default stays while it is installed; otherwise the model that has just finished downloading; otherwise the first installed model in catalog order (the same fallback the mock uses for a recording with no default); null when nothing is installed. `ModelLibraryStore` calls it after every read of models and settings (`settleDefault`) and saves a difference with `api.updateSettings`. A finished download and a delete each trigger that read. So: the first install becomes the default, a later install does not move it, deleting the default moves it to another installed model or to none, and installed models with no saved default get one. If saving fails, Settings shows "Could not set the default model: <reason>" with Retry. First run gets all of this by using the store; it has no rule of its own to write.

### Shortcut string format (for issue 023)

`[Control+][Alt+][Shift+][Command+]<KeyboardEvent.code>`: modifiers always in that order, joined with `+`, then the W3C `code` of the physical key. Examples: `Shift+Command+KeyR`, `Control+Alt+Space`, `Alt+F5`, `Command+Digit1`, `Control+Comma`, `Command+ArrowUp`. Null means none.

This is what Tauri's global-shortcut plugin parses: its `Shortcut` is `global_hotkey::hotkey::HotKey`, and `parse_hotkey`/`parse_key` in global-hotkey 0.10.0 `src/hotkey.rs` (read from the repository on 2026-11-12; the Tauri docs page only shows `CommandOrControl+Shift+C`) split on `+`, match case-insensitively, accept `Control`, `Alt`, `Shift`, `Command` as modifiers and the `code` names (`KeyR`, `Digit1`, `F5`, `Space`, `Comma`, `ArrowUp`) as keys. So `shortcut.parse::<Shortcut>()` should work with no conversion. The recorder only offers keys that the plugin's macOS backend maps (`key_to_scancode`): letters, digits, F1 to F20, the eleven punctuation keys, Space, Return, Delete, Forward Delete, the arrows, Home, End, Page Up, Page Down. `describeShortcut` also reads the parser's other spellings (`Cmd`, `Ctrl`, `Option`, `CommandOrControl`, `R` for `KeyR`), in case the backend ever returns a normalized string.

### Which model a recording uses

`RecordingState` has no model id, so the UI does not guess: **while a recording is running or stopping, every Delete is disabled**, with one line under the list saying why, and an open delete question is withdrawn if a recording starts. Marking one row "in use" from the default at start time could mark the wrong row (the backend falls back to another model when the default is missing). The backend's own refusal is still shown on the row if it ever arrives. See follow-ups for the contract field that would let only one row be locked.

### Mock changes

- `mockSeed.ts`: `MOCK_MODEL_CATALOG` is now the real catalog's ten models, copied from `crates/mneme-transcribe/src/models/catalog.rs` (ids, names, sizes, notes; `englishOnly` for the `.en` ids), in its order. `tiny.en`'s note changed with it.
- `mock.ts`: new option `download.cancelMs` (default 0, which is the old behavior: `cancelled` is sent before `cancelDownload` returns). With a value, the call returns at once, the download stops, `cancelled` arrives that many ms later, and until then the model cannot be started again. `&mockCancelDelay=3000` in the URL sets it in the browser.
- **Existing tests: none had to change for the catalog.** The five old ids are all in the new catalog and the old tests use `MOCK_MODEL_CATALOG.length`. In `src/lib/api/mock.test.ts` two tests were added (the ten-model catalog; the cancel delay) and one existing test, "the query string picks a scenario and a latency", gained one assertion for `mockCancelDelay`.

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm build` | Pass. JS 583.23 kB (gzip 187.54 kB), CSS 19.27 kB (gzip 4.27 kB); before: 564.27 kB and 13.16 kB. |
| `pnpm test` | Pass: 22 files, 340 tests (115 new). |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| Tests fail when the code is broken | 22 single changes, applied one at a time by a script that restored each file. 21 made tests fail: default moved by a later install; no fallback default; rules not applied after a read; cancelling forgotten on progress; error while cancelling kept; `done` leaving a status; no cancelling state; `done` not installing; a refused download swallowed; default not moved after delete; shortcut with no modifier accepted; Shift alone accepted; Tab swallowed; Escape not cancelling; a refused shortcut kept; its error hidden; Clear dead; modifier order; delete without the question; delete allowed while recording; download state dropped when Settings is reopened. The 22nd was a change that turned out to alter nothing, and was replaced by the last one in the list. |

Tests per acceptance item (`src/screens/SettingsScreen.test.tsx` unless noted): list rendering ("shows every model…", "marks the English-only models…", nothing installed, loading, load error); download through to installed ("shows progress with received and total, then verifying, then the model is installed", which also asserts no `style` attribute and no `<style>` element); cancel ("Cancel stops the download…", "until the backend confirms a cancel, the row says it is cancelling" with the mock's cancel delay); error and retry ("a failed download says why on its row, and Retry downloads it", Dismiss, a refused start); survives leaving Settings ("progress carries on while Settings is closed, and is there on return", and the same for a pending cancel); first install becomes default; deleting the default reassigns, and deleting the only model leaves none; delete question, Escape, a failed delete, locked while recording; shortcut record ("click, press a combination…", from the keyboard), clear, modifier rule ("a key without a modifier is not accepted, and says why"), error restore ("a shortcut the backend refuses…", "a failed Clear keeps the shortcut"), Escape, Tab. The store and the pure rules have their own tests in `src/state/modelLibrary.test.ts`, `src/lib/models.test.ts` and `src/lib/shortcut.test.ts`.

Browser check, in the Chromium-based browser pane of the coding tool, with `prefers-color-scheme` emulated. Both servers were stopped afterwards.

Against `pnpm dev` (http://localhost:1420), in screenshots:

- Light, 960×680: the list with one model installed; two downloads running with their bars; a cancelling row (mock cancel delay) beside a running one; three installed models with the default on an English-only one; the delete question on the default model ("Base becomes the default."), closed with Escape, focus back on Delete (read from the DOM); the shortcut field recording, the no-modifier reason after pressing K alone, and Control-Option-K recorded with real key events from the browser tool and saved as `Control+Alt+KeyK` (read back from the mock).
- Dark, 960×680: a verifying row (full bar), a cancelling row (gray bar), a failed row with Dismiss and Retry, and the installed list after a download finished.
- 640×480, light and dark: the installed list, a failed row, the available list and the Shortcut section with `⇧⌘R` and Clear; and in light the locked state during a recording (both Delete buttons disabled, the note, the recording bar under the screen).
- A fresh load ended with no console errors and no `style` attributes.
- Order of events: the 960×680 screenshots on the dev server, and the 640×480 ones of the installed list and the failed row, were taken while the screen still had the language section, which was then removed. After the removal the screen was looked at again at 640×480 in light and dark on the dev server (locked state, available list, Shortcut section) and at 960×680 in light and dark on the built `dist/` below. The model rows and the recorder did not change in between.

Against the built `dist/` (`index-CijKeiHQ.js`, `index-x113hzQQ.css`, the final build), served by a throwaway Node server in the scratch folder that reads `app.security.csp` from `src-tauri/tauri.conf.json` and sends it as the `Content-Security-Policy` header (`curl -I` showed `default-src 'self'; script-src 'self'; style-src 'self'; …`):

- With one download in progress (bar filled to 10 %, then 15 % in the screenshot) and another cancelling: a `securitypolicyviolation` listener recorded **0 violations**; `style` attributes: 0; `<style>` elements: 0; one stylesheet; no console messages. After the download finished and the cancel completed: still 0.
- Control on the same page: `setAttribute("style", …)` from script was blocked with a `style-src-attr` violation, so the policy was being enforced.
- The bar is a `<progress>` element filled from its `value` and `max` attributes; its look is set in the stylesheet through `::-webkit-progress-bar`, `::-webkit-progress-value` and `::-moz-progress-bar`.

### Known gaps

Nothing below was chased further; each is something not verified or left weak.

- **WKWebView (the real app) and Safari.** Nothing ran in WebKit; `pnpm tauri dev` was not run. Unknown there: how the styled `<progress>` and the accent-colored radio buttons render; whether a click focuses the recorder field (WebKit does not focus buttons on click, so the field focuses itself; untested there); and which Command combinations reach the page at all (the app menu takes its own key equivalents first).
- **The real backend.** Everything ran against the mock: no real download, no real cancel delay, no real shortcut registration, and no check that the plugin accepts the strings (the parser's source was read, not run).
- **The starting state (bar with no value) was not looked at in a screenshot**; it is covered by tests only. The pulsing of that bar was not watched.
- **Not looked at in a screenshot:** the load error; the delete question in dark or at 640×480; the recorder's listening state in dark; the locked state in dark.
- **Timing in the browser pane.** The pane was hidden, so the mock's timers were throttled; durations seen there mean nothing.
- **A screen reader** was not used. Names and roles are asserted in tests only.
- Displayed key names assume a US layout: the saved key is the physical key (`KeyR`), which is what a global hotkey registers, so on another layout the letter shown can differ from the keycap.

### Decisions and deviations

- **Language setting removed** (scope change, 2026-11-12): v1 is English only, so Settings has no language control, list or English-only warning; the code written for it was deleted. `Settings.language` stays in the contract and the mock, unused by the UI. The "English only" tag on models remains as plain information.
- **Every Delete is disabled during a recording**, instead of only the model in use (see above).
- **Shift alone is not accepted as the modifier**, except with a function key. The scope says "at least one modifier"; a global shortcut on Shift-R would take capital R away from every app.
- **Installed and Available are two boxes**, not one list of ten with states, following the screens table ("installed models with size and delete, download more").
- **Additions beyond the scope list**, all small: Dismiss on a failed download; the starting state; what a delete does to the default, said in the question; installed models with no saved default getting one on read; the mock's cancel delay.
- A failed read after a download or delete replaces the screen with the error and Retry, as `useLoad` does elsewhere.

### For 020 (First run)

- Import from `"../components"`: `ModelRow`, `ModelList`, `DownloadProgress`, `Section`, `Group`, `GroupRow`. `ModelRow` takes `model`, `download`, `leading` (a radio for the preselected model), `badge` ("Recommended"), `onDownload`, `onCancelDownload`, `onDismissError`, `installedActions` and children; without `installedActions` an installed model says "Installed". `DownloadProgress` can be used by itself for one large progress display.
- State: `const library = useModelLibrary()`, call `library.load()` in an effect when the screen is shown, pass `library.downloads.get(model.id)` to the row and `library.download(id)` / `cancelDownload(id)` / `dismissDownloadError(id)` to its handlers. `renderWithProviders` and `App` already include the provider.
- "The downloaded model is the default" needs no code: the store sets it when the download finishes. Wait for `library.defaultModel` to be set (or the model to be `installed`) before moving to the permissions step.
- "Quitting mid-download and reopening returns to first run cleanly": a fresh store has no downloads, and the gate already opens First run when nothing is installed.
- `formatBytes` (`src/lib/format.ts`) and `modelNote` (`src/lib/models.ts`) for any text outside the row. For tests: `mockApi.mock.reset({ download: { cancelMs } })`, `failNextDownload`, fake timers with `advance()`.

### Follow-ups

- **021 (contract):** add the model in use to the contract (`RecordingState.modelId`, or `inUse` on `ModelInfo`) if only that model's Delete should be locked; until then Settings locks all of them during a recording. The change goes in `types.ts` first.
- **021:** there is no call that lists downloads already running. After a window reload mid-download the row reappears with the next progress event; if events can be far apart (verifying, a stalled connection), an "active downloads" read at startup would close the gap.
- **021:** `defaultModelId` is the UI's to maintain: the backend should keep leaving it alone on install and clearing it when the default is deleted, and `updateSettings({ defaultModelId })` is called by the store without any user action (after a download, after a delete, and on read when it is missing). `deleteModel` errors and download `error` texts are shown as given, after "Could not delete <name>: " and "Could not download <name>: ".
- **023:** the format above. `updateSettings({ globalShortcut })` must reject with a readable sentence when registration fails and leave the old shortcut registered and saved; Settings then shows that sentence and the old value. `globalShortcut: null` unregisters. If the backend normalizes the string, keep it within what `describeShortcut` reads, or Settings shows the raw string.
- **021 / 022:** Command combinations bound to app menu items never reach the recorder; that is the wanted behavior, but worth one look in the app window.
- **Whoever first runs the app window:** look at the progress bar, the radio buttons and the recorder field in WKWebView, in light and dark.
- Root `README.md` (not owned by this lane) still says the mock has "a catalog of five models", and does not mention `mockCancelDelay` or `download.cancelMs`.
- Later: show key names for the active keyboard layout; Cmd-, to open Settings belongs with the app menu.
- `issues/README.md` still lists 019 as in progress; this file's status line is the source of truth.
