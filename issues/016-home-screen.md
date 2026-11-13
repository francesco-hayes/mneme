# 016: Home screen

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 015 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

MVP feature 4: a list of meetings, newest first, and one button to start a new one.

## Scope

1. **List** from `api.listMeetings`: title, date (friendly: "Today 2:30 PM", "Oct 27"), duration ("32 min"). Newest first. Clicking a row opens the meeting full window.
2. **States**: loading, empty ("No meetings yet" with the New meeting button), error with retry. A meeting with status `interrupted` is marked as such; one that is `recording` shows it is live.
3. **New meeting button**: calls `api.startMeeting` and navigates to the live view. Disabled with a short explanation and a link to Settings when no model is installed. Disabled while a recording is already running (the recording bar is the way back to it). Errors from start (for example a denied permission) are shown inline, not as a popup.
4. The list refreshes when returning to Home and when recording stops.
5. Keyboard: up/down moves through rows, Enter opens, Cmd-N starts a new meeting.

## Acceptance criteria

- [x] Tests: ordering; date and duration formatting (including a meeting with no duration); empty, loading and error states; New meeting disabled without a model and while recording; clicking a row navigates; a start error is shown.
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. No dependency was added; `package.json`, the lockfile, the config files, `src/lib/api/` (types and mock) and `src/test/` are unchanged.

### What was built

```
src/
  screens/
    HomeScreen.tsx          the screen (replaces the placeholder)
    HomeScreen.module.css
    HomeScreen.test.tsx     30 tests
    placeholder.module.css  deleted: Home was its only user
  lib/
    format.ts               + formatMeetingDate, formatFullDate, formatDuration, DateContext
    meetings.ts             newestFirst
    useWindowEvent.ts       window listener hook (keyboard shortcuts, focus)
  components/
    List.tsx                up/down arrow keys move between rows that open something
    listFocus.ts            focusListRow (exported from "../components"), moveListFocus
    List.module.css         the keyboard-focused row is tinted, ring drawn inside the row
    Button.tsx, .module.css new variant "link": underlined text inside a sentence
```

- **List.** One row per meeting from `api.listMeetings`, sorted newest first in the UI as well (`newestFirst`), so the order does not depend on the backend. First line: the title. Second line: the date, as `Today 2:30 PM`, `Yesterday 11:00 AM`, `Oct 27`, or `Oct 27, 2025` for another year, with the full date and time as a tooltip. At the end of the row: the duration to the nearest minute (`32 min`, `1 hr 5 min`, `45 sec` under a minute), or nothing when `durationSecs` is null. An interrupted meeting has "Interrupted" in the warning color after its date. A meeting with status `recording` has a red dot and "Recording" in place of the duration; it does not show a timer, the recording bar already does. Clicking a row opens the meeting.
- **Formatting** uses `Intl.DateTimeFormat` and `Intl.NumberFormat` (unit style) only. The app passes no locale or time zone, so the system's are used (a 24-hour locale gets `Today 14:30`); "Today" and "Yesterday" are English like the rest of the UI. `HomeScreen` takes optional `now`, `locale` and `timeZone` props, which only tests pass. WebKit puts a narrow no-break space before AM/PM and Chromium and Node a plain one; the formatters turn both into a plain space so the text is the same everywhere. A `startedAt` that cannot be parsed gives an empty date and sorts last instead of breaking the screen.
- **States.** Loading: "Loading meetings…", made visible only after 400 ms (a CSS delay) so it does not flash on every visit. Empty: "No meetings yet" with a New meeting button under it. Error: the message inline with Retry.
- **New meeting.** In the top bar on every state, and again in the empty state (standard style there, so the accent-colored one in the top bar stays the single primary button). It calls `api.startMeeting` and navigates to the meeting. Disabled when no model is installed, with the reason shown and a "Download one in Settings" link: above the list, or as the empty state's text. Disabled while a recording is running or stopping, and while a start is in flight (a second press or Cmd-N cannot start twice). A failed start is shown inline above the list ("Could not start the meeting: …") and cleared by the next try. Both disabled reasons are also the button's tooltip.
- **Refresh.** Home is mounted fresh each time it is shown, so returning to it reads the list again. While it is showing, the list is read again when a recording starts or stops (including from outside the window) and when the window gets focus back. The timer ticking does not cause a read. "Today" is judged against the moment the list was last read.
- **Keyboard.** Up/down move through the rows and stop at the ends; with focus outside the list, down goes to the first row and up to the last. Enter opens the focused row (rows are buttons). Cmd-N starts a new meeting when the button is enabled, on Home only.

### Decisions worth knowing

- The model list is read by Home itself each time it is shown (Settings can change it). Until it arrives, and if it cannot be read, New meeting stays enabled; a start that cannot work reports why inline.
- A refresh that fails replaces the list with the error and Retry (that is how `useLoad` works).
- Two tests from issue 015 found a row with `name: "Standup"` exactly. Rows now also carry the date and duration, so those queries became `/^Standup/` and `/^Design review/` (`src/App.test.tsx`, `src/shell/RecordingBar.test.tsx`). Nothing else in them changed.

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm test` | Pass: 11 files, 124 tests (48 new). Also passes with `TZ=Asia/Tokyo LANG=de_DE.UTF-8` and `TZ=Pacific/Honolulu LANG=ja_JP.UTF-8`. |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| `pnpm build` | Pass. |
| Tests fail when the code is broken | 25 single changes were applied one at a time and reverted (no sort, reversed sort, Today off by one, time zone ignored, year dropped, null duration shown, floor instead of round, model check off, recording check off, start error dropped, row click dead, no navigation after start, loading text gone, empty state gone, empty state without button, list error hidden, no refresh on recording change, no refresh on focus, interrupted not marked, live not marked, Cmd-N off, arrows not entering the list, list arrows off, double-start guard off, model notice gone). Each made between 1 and 12 tests fail. |

Tests per acceptance item: ordering in `src/lib/meetings.test.ts` and "orders by start time itself" in `src/screens/HomeScreen.test.tsx` (the screen is handed an unsorted list); date and duration formatting, including no duration, in `src/lib/format.test.ts` and "shows title, date and duration"; empty, loading and error under "states"; disabled without a model, disabled while recording, and the start error under "New meeting"; "clicking a row opens that meeting" under "the list". Beyond the list: refresh on return, on stop, on a recording started outside and on window focus; the keyboard; a double start.

Browser check, in the Chromium-based browser pane of the coding tool against `pnpm dev` on http://localhost:1420, with `prefers-color-scheme` emulated. The dev server was stopped afterwards.

- Looked at in light and dark, at 960×680: the list (with the interrupted mark), the list while recording (live row, disabled button, recording bar), the start error (mock microphone permission set to denied), the list error with Retry (which brought the list back), the empty state (`/?mock=empty`), and the keyboard-focused row.
- No model installed (`__mnemeMock.setInstalledModels([])`): with a list, the notice and disabled button in light at 960×680 and in dark at the minimum size 640×480; the link opened Settings. With no meetings, the empty state's explanation in light only.
- Loading text (`/?mockLatency=4000`): in light only.
- Flow: arrow keys entered and moved through the list and Enter opened "Design review"; Stop from Home removed the bar and the row changed from "Recording" to "11 sec" without leaving the screen.
- No console errors, and no `style` attributes in the page.

### Not verified

- **The 400 ms delay of the loading text in real time.** The browser pane was hidden while testing, so CSS animations did not advance. The keyframes were checked by setting the animation's time by script (hidden at 0 and 399 ms, visible from 400 ms), and the text was seen once made visible.
- **A real Cmd-N keypress.** A browser keeps Cmd-N for itself. The handler was exercised with a synthetic `keydown` in the browser and by the tests. In the app, a native menu item bound to Cmd-N would take the key before the page sees it.
- **The real app window (WKWebView) and Safari.** `pnpm tauri dev` was not run. Not seen there: the system font rendering of the rows, the tooltips, and whether the window `focus` event fires when the app becomes active.
- **The real backend.** Everything ran against the mock.

### Follow-ups

- Issue 017: New meeting only navigates to `{ name: "meeting", meetingId }` right after `api.startMeeting` resolves; the meeting view has to put the cursor in the editor itself. `useWindowEvent` is there for its shortcuts.
- Issue 021 and 022: if the app menu gets a "New Meeting" item on Cmd-N, it should start the meeting from any screen and Home's own handler can go. Until then Cmd-N works on Home only.
- Issue 021: Home trusts `status: recording` from `listMeetings` to mark the live row, and re-reads the list when `RecordingState.meetingId` changes between a meeting id and null. The backend must send the idle state only after the stopped meeting's duration and status are written, as the mock does.
- Issue 019: after the last model is deleted, Home shows the reason and the Settings link. Settings does not need to tell Home; Home reads the model list each time it is shown.
- Later: no virtualization and no paging; every meeting is a row. Fine for hundreds of meetings, to revisit with search.
- Root `README.md` (not owned by this lane) still says the window shows a placeholder list.
