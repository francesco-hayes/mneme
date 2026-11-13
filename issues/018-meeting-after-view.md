# 018: Meeting after view and transcript pane

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 017 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

After a meeting, keep the notes first and make the full transcript one click away.

## Scope

1. **After state** of the meeting view (status `complete` or `interrupted`): same layout, notes full width, still editable with autosave.
2. **Transcript button** in the top bar: shows a ready state with the line count ("Transcript · 214 lines"). With zero lines it says so and stays usable.
3. **Transcript pane**: a side pane beside the notes, closed by default, including right after stop. Toggles with the button and a keyboard shortcut. Resizable is optional; a sensible fixed width is fine. Loaded from `api.getTranscript` on first open.
4. **Lines**: timestamp, speaker, text, with two distinct speaker styles for Me and Them that work in both themes and do not rely on color alone. Consecutive lines from the same speaker group visually. Text is selectable and copyable. Long transcripts scroll smoothly (hundreds to a few thousand lines).
5. **Stopping state**: between Stop and the transcript being complete, the button shows it is finishing; the notes stay editable.
6. **Interrupted meetings**: a quiet note that the recording ended unexpectedly and the transcript may be incomplete.
7. **Show in Finder** action calling `api.showInFinder`. This covers export: the meeting already is Markdown on disk.

## Acceptance criteria

- [x] Tests: pane closed by default and after stop; button shows the line count; toggle opens and loads once; speaker styles applied per line; zero-line and interrupted states; Show in Finder calls the API; notes stay editable in the after state.
- [x] A 2,000-line mock transcript scrolls without visible lag (say how it was checked).
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. No dependency was added; `package.json`, the lockfile and the config files are unchanged, and so are `src/lib/api/` (types and mock; no mock control was needed) and `src/screens/MeetingScreen.test.tsx` (017's tests pass as they were).

### What was built

```
src/
  screens/
    MeetingScreen.tsx            loads the meeting, then MeetingView: top-bar actions, notices, notes, pane
    MeetingAfterView.test.tsx    38 tests (new file; 017's test file is untouched)
    meeting/
      useMeetingPhase.ts         live / finishing / after, and reading the meeting again when its recording ends
      useTranscript.ts           the transcript, read on first use and kept while the view is open
      TranscriptButton.tsx       "Transcript · 214 lines", the pane's toggle (+ .module.css)
      transcriptShortcut.ts      isTranscriptShortcut: Shift-Command-T
      TranscriptPane.tsx         the side pane: header, loading, error with Retry, empty, the lines (+ .module.css)
      notices.module.css         + .quiet, for the interrupted note
  lib/
    transcript.ts                groupBySpeaker, timestampSecs, shortTimestamp, transcriptLineText (+ 5 tests)
    format.ts                    + formatLineCount ("no lines", "1 line", "2,000 lines") (+ 1 test)
  components/icons.tsx           + TranscriptIcon (exported from "../components")
  styles/base.css                + .appears-late: the 400 ms delay for "Loading…" text, now shared
  screens/HomeScreen.tsx, .module.css   use .appears-late instead of Home's own copy of that animation
```

- **After state.** Same layout as the live view. With the pane closed the notes column is the only child of the row and has the full width. The editor is the one from 017 with the same autosave; nothing about it changes after a meeting. The top bar gains two actions before Settings: **Show in Finder** (plain button) and the **Transcript button**.
- **Transcript button.** "Transcript · 214 lines", "· 1 line", "· no lines" (still enabled; the pane then says "Nothing was transcribed in this meeting."). The count is the ready sign. It comes from `meeting.transcriptLineCount`, and from the loaded lines once the pane has been opened. `aria-expanded` follows the pane, and while the pane is open the button is drawn held down (accent tint). Tooltip: "Show transcript (⇧⌘T)" / "Hide transcript (⇧⌘T)".
- **Pane.** An `<aside aria-label="Transcript">`, the second child of `.body`, to the right of the notes. Closed every time a meeting is opened: the open flag is plain component state in the meeting view, which is keyed by meeting id, and nothing is persisted, so it cannot carry over between meetings or visits (the decision log says closed by default; nothing justified more). Nothing but the button and the shortcut opens it, so it is closed after a stop too. Header: "Transcript", and once the lines are loaded "214 lines · 52 min" (the duration is the meeting's `durationSecs`).
- **Loading.** `api.getTranscript` is called the first time the pane opens, not before (opening a meeting does not read the transcript). Closing and reopening shows the same lines without another call, scrolled to where they were left. "Loading transcript…" appears only if the read takes longer than 400 ms. A failed read shows "Could not load the transcript: <reason>" with Retry inside the pane; reopening the pane after a failure also tries again.
- **Lines.** Consecutive lines from one speaker form a turn (`<li data-speaker>`). Each turn starts with the speaker's name written out ("Me" / "Them"), which is what tells the two apart without color. On top of that, what I said is a tinted block (accent at 16 %) with a solid accent bar on its left, and what they said is plain text beside a faint gray bar; "Me" is in the primary text color and "Them" in the secondary one. All of it is built from the tokens, so it follows light and dark. Every line has its timestamp in a narrow column on the left: caption size, tertiary color, tabular digits, written short (`6:40`, `1:02:03`); the column widens when the transcript reaches an hour.
- **Selecting and copying.** The list is `selectable`. Copying a selection that covers more than part of one line puts whole lines on the clipboard in the format of `transcript.md`: `[00:06:40] Them: The export work is merged.`, one per line. That is done in a `copy` handler, because what the browser would copy by itself has the time, the name and the text on separate lines and names the speaker only on the first line of a turn. A few words selected inside one line are copied as they are. A copy made while the focus is in a text field or the notes editor is left alone.
- **Stopping state.** `useMeetingPhase` gives the view one of three phases. `live`: this meeting is being recorded; nothing about the transcript is rendered (017's rule). `finishing`: from the press of Stop until the finished meeting has been read; the top bar shows a disabled button "Finishing transcript…", there is no count and no way into the pane, and the shortcut does nothing. `after`: the two actions above.
- **How the stop updates the view.** The hook watches `useRecording()`. When this meeting stops being the live one (Stop in the bar, or a stop from anywhere else), it calls `api.getMeeting` once more and keeps the result in its own state: status, duration and line count come from that copy. `useLoad` is not reloaded and no key changes, so `MeetingNotes` and the editor are the same instances, with their text, cursor, undo history and any unsaved typing (tested: text typed while stopping is still there and still unsaved after the transition, and is then saved by the normal autosave). If that second read fails, the view still moves to `after`: the button says "Transcript" with no count it cannot vouch for, and takes the count from the lines once the pane is opened; there is no error screen in place of the notes.
- **Interrupted meetings.** A line above the notes in the secondary text color, `role="note"`: "The recording ended unexpectedly. The transcript may be incomplete." Not an alert, not dismissible, and everything else works as for any meeting.
- **Show in Finder** calls `api.showInFinder(id)`. A failure shows "Could not show the meeting in Finder: <reason>" above the notes and goes away with the next try.
- **Keyboard.** Shift-Command-T opens and closes the pane, wherever the focus is. See below.

### Layout and narrow widths

The pane's width is `clamp(260px, 36%, 400px)` of the row, set in the stylesheet (no inline style, nothing resizable). Measured in the browser:

| Window | Notes | Pane |
| --- | --- | --- |
| 960×680 (default) | 614 px | 346 px |
| 640×480 (minimum) | 380 px | 260 px |
| from 1112 px wide | the rest | 400 px |

The notes are the larger part at every size the window can have. At the minimum size the pane stays beside the notes instead of covering them: side by side is the point of the pane (the decision log: "notes can still be checked against the transcript side by side"), the editor keeps a 332 px text column there, and the transcript's text a column of about 180 px beside its timestamps. There is no overlay mode and no breakpoint. The top bar fits at 640 px with a 250 px title field next to both actions. The pane is not resizable: a fixed proportion was accepted by the scope, and dragging would need widths set from script.

### The keyboard shortcut: Shift-Command-T

- Not bound by the editor. Read in the installed `@codemirror/commands` 6.11.1: the T bindings are `Ctrl-t` (transpose) only; `standardKeymap`, `historyKeymap`, `markdownKeymap` and `indentWithTab` have nothing on Shift-Command-T.
- Not one of the shortcuts macOS gives every app. Apple's list (support.apple.com/en-us/102650) has Command-T for Show Fonts and Option-Command-T for Show Toolbar; Shift-Command-T appears only as Finder's own "show or hide the tab bar". Browsers use it to reopen a closed tab, which matters only in the dev server.
- Considered and dropped: Option-Command-I (the standard "inspector" shortcut, but it opens the Web Inspector in a debug WKWebView), Option-Command-S and Control-Command-S (sidebar shortcuts; this pane is on the right and is not navigation), Command-\ (commonly a password manager's autofill key).
- It is a `keydown` listener on the window (`useWindowEvent`). Keyboard events are composed, so a key pressed inside the editor's shadow root reaches it; CodeMirror does not handle that key and nothing is typed. Tested by firing the key on the editor's content element inside the shadow root, and in the browser with a real key event while the cursor was in the notes (the pane toggled, the text was unchanged, the focus stayed in the editor). The key is matched by `event.key` in either case, so it is the key labeled T on any layout. Closing the pane while the focus is inside it moves the focus to the Transcript button.

### Long transcripts: every line in the page, no windowing

Decision: plain rendering. All lines are real elements in one scrolling list (2,000 lines are 9,332 elements), the pane has `contain: content` so its layout and the editor's do not touch each other, and the list component is memoized. No windowing, no `content-visibility`, no dependency.

Why, in order:

1. **What the target engine has.** `content-visibility` is in Safari from 18.0 (MDN: Baseline 2024; caniuse: Safari 18.0, Chrome 85, Firefox 125). The app's minimum is macOS 14.4, whose WKWebView is Safari 17.4, which ignores the property. So the plain path has to be good enough by itself on the oldest supported system, whatever is added on top. `contain` is in Safari from 15.4.
2. **The plain path, measured.** Built `dist/` served with the app's CSP header, Chromium-based browser pane, viewport 960×680, 120 Hz display, Apple M2 Pro, the `long-transcript` scenario (2,000 lines):

   | Measurement | Result |
   | --- | --- |
   | Click on the button to lines in the DOM and laid out (5 openings) | 45 to 90 ms (90 ms was the first, which also reads the transcript); one main-thread task of 50 to 81 ms |
   | Click to the second animation frame after it | 93 to 112 ms |
   | Scrolling by script, 60 px per frame for 240 frames | frame interval: median 8.3 ms, 95th percentile 9.3 ms, longest 9.5 ms; no frame over 17 ms; no long task or long animation frame |
   | Scrolling by script, 400 px per frame for 240 frames (96,000 px) | median 8.3 ms, 95th percentile 8.6 ms, longest 9.3 ms; none over 17 ms |
   | Wheel scrolling injected by the browser tool (5 gestures, 1,000 px each) | 54 frames watched: median 8.3 ms, longest 14.8 ms; no long frame |
   | Laying every line out again at a new pane width (what a window resize costs per step), on the previous build | 19 to 27 ms |
   | Typing 80 characters in the notes with the 2,000 lines open beside them, on the previous build | no long task or long animation frame; 2 input events reported over 16 ms, the longest 32 ms |
   | Select all in the list and copy (2,000 lines to the clipboard event) | 1.9 ms (dev server) |

   Scrolling runs with no script at all (the only scroll listener stores a number), so once the lines are laid out there is nothing to lag. The cost is the one-time 50 to 90 ms when the pane opens.
3. **`content-visibility: auto` on each turn, measured for comparison** (the property added to the stylesheet rule through the CSSOM, on the build before the last small edits, where plain rendering measured 52 to 61 ms and 94 to 110 ms): opening 17 to 26 ms to the DOM and 31 to 49 ms to the second frame, re-layout 7 ms, scrolling the same as plain apart from one 216 ms frame in one run that was not explained. So it saves about 35 ms once per opening, and only on macOS 15 or later. Against that: it would be a second behavior that only exists in an engine nobody here can test; off-screen turns get estimated heights (the list's height was 119,984 px estimated against 120,217 px real), which breaks returning to the remembered scroll position when the pane reopens and moves the scrollbar while reading; and WebKit has no scroll anchoring to hide those corrections when scrolling upward. Not worth it for 35 ms.
4. **Windowing** would keep opening fast at any length, but needs positions set from script on every scroll (against this codebase's rule of no `style` attributes under `style-src 'self'`, and the reason the usual libraries are out), and it breaks selecting and copying across the whole transcript and the browser's own find. At "hundreds to a few thousand lines" the measurement does not call for it.

Where it bends (dev server, which is slower than the build: React development mode and StrictMode's double render): 2,000 lines open in 92 to 236 ms and scroll with no frame over 9.4 ms; 5,000 lines (23,332 elements) open in 189 to 330 ms and scroll with no frame over 9.1 ms; 10,000 lines (46,666 elements, about 14 hours at a line every 5 seconds) open in about 480 ms and scripted scrolling drops to a median of 14 to 16 ms per frame. So plain rendering is comfortable to at least 5,000 lines; a meeting of 10,000 lines would be the reason to revisit.

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm build` | Pass. JS 564.27 kB (gzip 181.75 kB), CSS 13.16 kB (gzip 3.30 kB); before: 558.00 kB and 10.60 kB. |
| `pnpm test` | Pass: 17 files, 225 tests (44 new: 38 after view, 5 transcript helpers, 1 line count). Also passes with `TZ=Asia/Tokyo LANG=de_DE.UTF-8` and `TZ=Pacific/Honolulu LANG=ja_JP.UTF-8`. |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| Tests fail when the code is broken | 61 single changes were applied one at a time by a script, the whole suite run, and the file restored. 58 made at least one test fail on the first run. 3 survived (ready before the finished meeting had been read; transcript UI for a meeting marked `recording` that nothing is recording; a copy in a text field taken over); a test was added for each, and all 3 then failed as they should. The list includes: pane open by default, pane opening at stop, no count, wrong wording for 0 and 1, transcript read on every opening or never, no retry on reopening, toggle dead, pane not closable, every turn styled as Me, name not written, no grouping, one big turn, raw or missing timestamps, empty state silent, interrupted note missing or on every meeting, Show in Finder not calling the API, its error dropped or never cleared, the editor rebuilt at the phase change, no finishing state, finishing button usable, no re-read after stop, a failed re-read stuck in finishing, stale count, stale duration, each modifier of the shortcut, the shortcut while recording, focus lost on close, copy format, partial line copied whole, scroll position not kept, loading text gone, error hidden, Retry dead, lines not selectable, pane not an `aside`. |

Tests per acceptance item, all in `src/screens/MeetingAfterView.test.tsx`:

- Pane closed by default: "shows the line count, and the pane is closed" (also asserts `getTranscript` was not called), "the pane is closed again the next time the meeting is opened". After stop: "when the stop completes the button is ready with the count, and the pane stays closed", "a recording stopped from outside the window ends the same way".
- Line count: the same first test (214 lines), "says one line as one line", and after a stop (6 lines from the mock's recording).
- Toggle opens and loads once: "opens and closes with the button, and reads the transcript once"; the shortcut tests.
- Speaker styles per line: "shows every line with its time, under the speaker of its turn" (each line's turn carries `data-speaker`, the attribute the CSS styles on), "groups consecutive lines of one speaker into a turn that names the speaker".
- Zero lines: "with no lines it says so and still opens the pane". Interrupted: "says quietly that the transcript may be incomplete", "a meeting that ended normally has no such note".
- Show in Finder: "asks the backend to show this meeting's folder", "a failure is said in place and goes with the next try".
- Notes editable after: "can still be edited, and are saved as before", "can be edited with the transcript open beside them", "the editor and unsaved text survive the stop".
- Beyond the list: the finishing state and its end, a failed re-read, a failed stop, loading, error and Retry, the shortcut (from the body, from inside the editor's shadow root, from inside the pane, wrong modifiers, while recording), copying (across lines, within a line, nothing selected, focus in a text field), scroll position kept across close and open.
- 017's "shows Back, the title, the editor and the recording bar, and no transcript" passes unchanged, and one of the 61 changes (transcript UI in the live phase) makes it fail.

Browser check, in the Chromium-based browser pane of the coding tool, with `prefers-color-scheme` emulated. Both servers were stopped afterwards.

Against `pnpm dev` (http://localhost:1420, `/?mock=long-transcript`), looked at in screenshots unless said otherwise:

- Light, 960×680: a finished meeting with the pane closed (notes full width) and open (214 lines); the 2,000-line transcript past the one-hour mark (wider timestamp column); the interrupted meeting with its note, pane open; a new recording stopped from the bar: the stopping state ("Stopping…" in the bar, "Finishing transcript…" disabled in the top bar) and the state after it ("Transcript · 3 lines", pane closed). The transition was also read from the DOM at 150 ms and 1,050 ms after the click: Back only, then Back and the disabled finishing button, then Back, Show in Finder, "Transcript · 3 lines" and Settings, with the same editor element and text throughout.
- Dark, 960×680: the zero-line meeting with the pane open; a failed transcript read (mock `failNext("getTranscript")`) with Retry, together with a failed Show in Finder.
- Dark, 640×480: the 2,000-line transcript with the pane open, at the start and past the one-hour mark.
- Light, 640×480: both errors, and after Retry the 12 lines; no horizontal overflow.
- The shortcut, with key events from the browser tool (they arrive as `key: "t"` with Command and Shift): from the list (the pane closed and focus went to the button), from the button, and twice from inside the notes (toggled each time, the notes text unchanged, focus still in the editor, typing carried on).
- Copying, with a real `copy` event (`document.execCommand("copy")` after a real click) and a listener reading what the handler put on the event: three whole lines for a selection from mid-line to mid-line; the browser's own copy for words inside one line; all 2,000 lines after Select All with the focus in the list (Chromium's Select All stayed inside the list).
- Scroll position: closed at 5,000 px, reopened at 5,000 px.

Against the built `dist/` served by a throwaway Node server (in the scratch folder, outside the repo) that reads `app.security.csp` from `src-tauri/tauri.conf.json` and sends it as the `Content-Security-Policy` header (`curl -I` showed `default-src 'self'; script-src 'self'; style-src 'self'; …`), final build `index-vUh5DMJ3.js` / `index-DV08ljeq.css`:

- The 2,000-line transcript with the pane open in light and dark at 960×680 and at 640×480 (four screenshots), opened and closed five times, scrolled by script and by wheel.
- A `securitypolicyviolation` listener recorded **0 violations** through all of it, and the console had no messages. `<style>` elements in the page: 0. `style` attributes outside the editor's shadow root: 0 (inside it: 2, CodeMirror's own, written through the CSSOM as described in 017).
- Computed styles read in both themes: Me text on `color(srgb 0 0.46 1 / 0.16)` with an `rgb(0, 117, 255)` bar; Them text on transparent with a `rgba(255,255,255,0.12)` bar in dark; body text 0.87 white in dark and 0.85 black in light; timestamps 0.3 white in dark (not read in light).
- "Loading meetings…" on Home and "Loading transcript…" in the pane with `&mockLatency=1500`: computed `visibility` hidden 100 ms after appearing and visible 600 ms later, in real time. Not looked at in a screenshot.

### Not verified

- **WKWebView (the real app).** Nothing was run in WebKit; `pnpm tauri dev` was not run. Specifically unknown there: (1) how 2,000 lines open and scroll (the numbers above are Chromium's; the approach uses nothing newer than Safari 15.4, but WebKit's layout speed was not measured); (2) that Shift-Command-T reaches the page as a `keydown` (it should, since no menu item has it; a menu item that takes it later would have to call the same toggle); (3) that the `copy` handler runs for a native Edit > Copy and that `clipboardData.setData` is honored there; (4) that Select All in the list stays inside the list as it did in Chromium (if it selects more, the handler still copies only transcript lines); (5) selection across the tinted blocks and the look of the selection color on the tint.
- **Nobody watched it scroll.** "Without visible lag" rests on measured frame intervals and long-frame observers, not on a person's eyes. The injected wheel gestures are jumps of 1,000 px, not a trackpad's stream of small deltas.
- **A physical Shift-Command-T.** The key events came from the browser tool. In a normal browser window the browser keeps that key for itself.
- **The system clipboard.** What the handler put on the copy event was read; the clipboard itself was not read back.
- **Not looked at in a screenshot:** the stopping state in dark; the zero-line pane in light; the interrupted note in dark; the pane's focus ring; the long title case at 640 px (widths were measured: title field 250 px beside 322 px of actions).
- **The re-layout and typing measurements** were taken on the build before the last two small edits (a simplified copy guard and tests); the opening and scrolling numbers are from the final build.
- **The real backend.** Everything ran against the mock, where `stopping` lasts 600 ms.

### Decisions and deviations

- **Duration lives in the pane's header**, beside the line count, not in the top bar. The screens table gives the top bar only the Transcript button and Show in Finder, and a date and duration there would compete with the title at 640 px.
- **The count on the button comes from the meeting; the count in the pane comes from the lines.** If they ever differ, the button switches to the loaded number.
- **A meeting whose status is `recording` while nothing is recording it** (the recording state has not arrived yet, or a crash that recovery has not tidied) is shown as the quiet live view, with no transcript UI. Guessing "after" would flash transcript UI in the live view when the state arrives late.
- **The pane's header is a `div`, not a `<header>`**: Testing Library counts every `<header>` as a `banner`, and 017's tests find the top bar by that role.
- **Additions beyond the scope list**, all small: copying in the transcript file's format, the scroll position kept across close and open, focus returned to the button when the pane closes around it, the count taken from the lines after a failed re-read, the shared `.appears-late` class.
- **Home changed in one place**: its loading text uses the shared `.appears-late` class instead of its own copy of the same animation. Checked in the browser as above; Home's tests pass unchanged.
- **Not done:** a close button inside the pane and Escape to close (the top-bar button and the shortcut are always in reach); a resizable pane; search in the transcript; jumping from a note to a line.

### For 019 and 020

- `.appears-late` (global, `src/styles/base.css`): put it on any "Loading…" text so it shows only after 400 ms.
- `formatLineCount` in `src/lib/format.ts` shows the pattern for a counted noun with `Intl.NumberFormat`; `TranscriptIcon` is exported from `../components`.
- `notices.module.css` now has `.quiet` next to `.warning`, for a line of information above content.
- `useTranscript` is the pattern for "read on first use, keep, retry on demand", where `useLoad` reads on mount.
- `Button` passes `ref` and every `aria-*` attribute through, so a toggle (`aria-expanded`) needs no new component; `TranscriptButton.module.css` has the held-down look.
- Testing Library: any `<header>` is a `banner` and any `<aside>` a `complementary` to it, wherever it sits. 017's live-view test asserts there is no `complementary` while recording.

### Follow-ups

- **Whoever first runs the app window (021):** the five WKWebView points above; open `/?mock=long-transcript` there and scroll it. If opening the pane feels slow in WebKit, the cheap step is `content-visibility: auto` plus `contain-intrinsic-size` on `.turn` (measured above, effective from macOS 15), at the cost of the exact scroll restore; the full step is windowing.
- **021:** `getMeeting` right after the recording state goes idle must return the final status, `durationSecs` and `transcriptLineCount`: the view reads the meeting once at that moment (the mock sets them before it emits idle). `showInFinder` errors are shown as given. `getTranscript` is called at most once per visit to a meeting, and only when the pane is opened.
- **013 / 021:** a meeting left with status `recording` after a crash stays in the quiet live view with no transcript until recovery marks it `interrupted`.
- **021 / 022:** if the app menu gets a View > Show Transcript item on Shift-Command-T, it must drive the same toggle, since the menu will take the key before the page sees it.
- **Later:** search in the transcript; click a timestamp to copy or to jump; a note-to-line link from the notes timeline; windowing if meetings of 10,000 lines turn up.
- Root `README.md` (not owned by this lane) still says the window shows a placeholder list.
- `issues/README.md` still lists 018 as in progress; this file's status line is the source of truth.
