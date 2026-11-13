# 017: Meeting live view and editor

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 016 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

MVP feature 3: type notes in a Markdown editor while the meeting records, with nothing else competing for attention.

## Scope

1. **Meeting view** for a meeting id: back button, title field, editor filling the rest of the window. While this meeting is recording, the recording bar from issue 015 is the only other element. No transcript, no badges, no popups.
2. **Editor**: CodeMirror 6 with Markdown highlighting as styled raw Markdown: markers stay visible but dimmed; headings, bold, italic, inline code and lists styled in place. Line wrapping on, no line numbers, system font, readable measure. Follows light and dark. Standard Markdown list continuation on Enter.
3. The document in the editor is exactly the string saved; nothing is transformed.
4. **Autosave** through `api.saveNotes`: shortly after typing pauses, on blur, on leaving the view, and before stop. A failed save shows a quiet inline warning and retries; unsaved text is never discarded.
5. **Title**: shows the default date-time title; editable any time; saved with `api.renameMeeting` on Enter or blur; never prompts. An error (for example a name clash) shows inline and restores the previous title.
6. Opening a new meeting puts the cursor in the editor.
7. **Stop** from the recording bar: flush notes, call `api.stopMeeting`, stay on the same view (issue 018 adds the after state).

## Acceptance criteria

- [x] Tests: typing triggers a debounced save with the exact text; leaving the view flushes a pending save; a failed save keeps the text and retries; rename saves on Enter and restores on error; no transcript UI is present while recording; focus lands in the editor for a new meeting.
- [x] The editor renders headings, bold and dimmed markers in both themes (checked in the browser).
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. `src/lib/api/` (types and mock) is unchanged; no mock control had to be added.

### What was built

```
src/
  screens/
    MeetingScreen.tsx        the meeting view (replaces the placeholder)
    MeetingScreen.module.css
    MeetingScreen.test.tsx   28 tests
    meeting/
      TitleField.tsx         the title in the top bar, renamed in place
      MeetingNotes.tsx       the editor wired to autosave, and the save warning
      notesAutosave.ts       one Autosave per meeting, kept across visits
      notices.module.css     the status lines above the notes
  editor/
    NotesEditor.tsx          CodeMirror in a shadow root; the React component
    extensions.ts            the explicit extension set
    theme.ts                 layout and Markdown styling, on the design tokens
    lineBreaks.ts            keeping CRLF files CRLF
  lib/
    autosave.ts              Autosave: debounce, flush, one save at a time, retry
  test/
    editor.ts                test helpers that drive the editor
    jsdomGaps.ts             stubs for what jsdom lacks and CodeMirror calls
    setup.ts                 + loads the stubs, drops pending autosaves between tests
  styles/tokens.css          + --text-editor (15px), --line-editor (1.6)
```

- **Meeting view.** `<Screen layout="fill">` with the title field as its title element. Under the top bar is a row (`.body`) holding the notes column, so 018 adds the transcript pane as a second child of the row and its buttons through `Screen`'s `actions`; nothing here has to move. While this meeting is being recorded the top bar holds Back and the title only (the Settings button is hidden for the live meeting and back once it has stopped), and the shell's recording bar is the only other thing on screen. A meeting that cannot be loaded shows the error in place with Retry.
- **Editor.** CodeMirror 6 with GitHub-flavored Markdown parsed for highlighting only. Every marker stays in the text and visible: `#`, `**`, `_`, backticks, `>`, link brackets and rules are dimmed (tertiary text color); list bullets and numbers are dimmed less (secondary), because they are read with the text. Headings are larger and bold, bold is bold, italic is italic, strikethrough is struck, inline code and code blocks are monospace, link text takes the accent color. Line wrapping on, no gutters, system font at 15px, lines kept to the 680px content width and centered (done with padding, so a click beside the text still lands in the editor). Enter continues a bullet, numbered, task list or quote and ends it on an empty item; Tab and Shift-Tab indent and outdent; Escape then Tab leaves the editor (built into CodeMirror). Undo and redo have their own history. Colors are the token CSS variables, so the editor follows light and dark with no theme of its own.
- **Exact text.** The editor reports `state.sliceDoc()` after each change and that string goes to `api.saveNotes` untouched. Nothing is saved until the text changes, so opening a meeting never rewrites its file. See "Line endings" below.
- **Autosave** (`Autosave` in `src/lib/autosave.ts`, no React in it): saves 800 ms after typing pauses, and at least every 5 s while typing never pauses. `flush()` saves at once: on editor blur (which includes the window losing focus), when the view unmounts (Back, any navigation), on `pagehide`, and through `useBeforeStop` before `api.stopMeeting`. Only one save runs at a time and each sends the latest text, so saves cannot arrive out of order; text typed while a save runs waits for its own pause, or goes out right after if a flush asked for it. A failed save keeps the text, shows "Notes not saved yet, still trying: <reason>" above the editor (`role="status"`, warning color, a "Try now" button) and retries after 2 s, 4 s, 8 s and so on up to 30 s until it works.
- **Never losing text across visits.** The autosave outlives the view. Leaving starts the last save without waiting, and a failed one keeps retrying in the background. If the same meeting is opened again before the backend has the text, the view gets the same autosave back and the editor opens with the unsaved text instead of the backend's older copy.
- **Stop.** The recording bar's Stop waits for the notes to be saved, then stops; the view stays where it is with the notes still editable. Stop is held up for at most 3 s: a save that hangs or fails does not keep the microphone on, and carries on after the stop.
- **Title.** An input in the top bar inside the page's `h1`, showing the meeting's title (the date-time default for a new one). Enter saves with `api.renameMeeting` and moves focus to the notes; leaving the field saves too. The title is sent trimmed and the field then shows the title the backend returned. Escape and an empty title put the previous title back without calling the backend. A failed rename shows "Could not rename the meeting: <reason>" above the notes and restores the previous title; the message goes when the field is focused again. Renames run one at a time in order. Nothing ever asks for a title.
- **Focus.** A meeting that is being recorded opens with the cursor in the editor, at the end of the notes: that covers New meeting, and coming back through "Open meeting". A finished meeting does not take focus.

### Content Security Policy: how CodeMirror runs under `style-src 'self'`

What CodeMirror does (read in the installed sources, `style-mod` 4.1.4 `src/style-mod.js` and `@codemirror/view` 6.43.15):

- All of its CSS goes through `StyleModule.mount(root, …)`. For a **document** root, style-mod creates a `<style>` element (with a `nonce` attribute if the `EditorView.cspNonce` facet is set). For a **shadow root**, it creates a constructed stylesheet (`new CSSStyleSheet()`, filled with `insertRule`) and adds it to the root's `adoptedStyleSheets`. The test in style-mod is `!root.head && root.adoptedStyleSheets && win.CSSStyleSheet`.
- Its inline styles (the `tab-size` on the content element, measuring and layer elements) are all set through `element.style.cssText = …`. The view never calls `setAttribute("style", …)` and creates no other `<style>` element.

So in the document, CodeMirror needs either `'unsafe-inline'` or a nonce in `style-src`; the nonce route would mean changing the policy to `style-src 'self' 'nonce-<value>'` and getting a per-load nonce into the page, which is not this lane's to change. **The editor is mounted in an open shadow root instead** (`NotesEditor.tsx`: `host.attachShadow`, then `new EditorView({ parent: root, root })`). CodeMirror supports shadow roots as a `root`, the stylesheet becomes a constructed one, and no policy change is needed. The design tokens are CSS variables and inherit through the shadow boundary, which is why the theme can use them. The app's global rules do not reach inside, so the theme sets `user-select: text` and `cursor: text` itself (the page turns both off for the window).

Evidence, in the Chromium-based browser pane of the coding tool. `pnpm build`, then `dist/` served by a throwaway Node server (kept outside the repo) that reads `app.security.csp` from `src-tauri/tauri.conf.json` and sends it as the `Content-Security-Policy` response header on every response. `curl -I` showed the header: `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self' data:; connect-src ipc: http://ipc.localhost; object-src 'none'; frame-src 'none'; base-uri 'self'; form-action 'none'`.

- Opened the app from that server, opened a meeting, typed with real key events (a heading, bold, inline code, a list continued with Enter, Tab to nest, Cmd-A). The editor was fully styled, in light and dark (screenshots looked at; computed styles read: `#` at 24px, weight 700, `rgba(0,0,0,0.3)` in light and `rgba(255,255,255,0.3)` in dark; body text at 0.85 and 0.87; list marker at 0.56 and 0.58).
- A `securitypolicyviolation` listener installed before the editor was created recorded **0 violations**, and the console had no messages.
- `document.querySelectorAll("style")`: 0. `<style>` elements inside the shadow root: 0. `shadowRoot.adoptedStyleSheets`: 1 sheet with 91 rules. `style` attributes outside the shadow root: 0; inside it: 1 (`tab-size: 4` on the content element, written by CodeMirror through `style.cssText`, present and not reported).
- Control, on the same page: adding a `<style>` element and calling `setAttribute("style", …)` from script were both blocked, each with a `securitypolicyviolation` event (`style-src-elem`, `style-src-attr`) and a console error naming `style-src 'self'`. So the policy was being enforced, and the document-root way CodeMirror mounts styles would have failed there.
- The check was repeated on the final build (`index-CvtiCPhS.js`) with a new meeting: 0 violations, 0 style elements, focus in the editor.

A test (`NotesEditor.test.tsx`, "lives in a shadow root and adds no style element to the page") fails if the editor is ever mounted in the document.

### Line endings

CodeMirror stores lines, not one string. By default it splits a document on `\n`, `\r\n` and `\r` alike and joins with `\n`, so a CRLF file would come back LF. Given `EditorState.lineSeparator`, it splits and joins on exactly that separator. `notesExtensions` therefore looks at the text it opens with:

- only `\n`, or one line: nothing configured; the text round-trips as it is.
- only `\r\n` (or only `\r`): that separator is configured, the text round-trips byte for byte, Enter inserts the same separator, and pasted or dropped text is converted to it (`EditorView.clipboardInputFilter`), because with a separator configured CodeMirror no longer splits on the others.
- mixed line endings: cannot be kept. The editor shows the text normally and it is saved with `\n` throughout, but only once the user changes something.

Nothing else is normalized: no trimming, no final newline added or removed, tabs and trailing spaces kept (tested with awkward text).

### Dependencies and bundle size

Added to `dependencies` (caret ranges like the rest of `package.json`; the lockfile resolves to exactly these, the current releases on 2026-11-12):

| Package | Version | Used for |
| --- | --- | --- |
| `@codemirror/state` | 6.7.6 | editor state, line separator |
| `@codemirror/view` | 6.43.15 | the editor view, theme, key bindings |
| `@codemirror/commands` | 6.11.1 | `standardKeymap`, history, Tab indent |
| `@codemirror/language` | 6.13.1 | `Language`, `HighlightStyle` |
| `@codemirror/lang-markdown` | 6.5.2 | `markdownLanguage`, `markdownKeymap` |
| `@lezer/highlight` | 1.2.5 | highlight tags |
| `@lezer/markdown` | 1.8.1 | the `MarkdownParser` type only (already installed as a dependency of lang-markdown) |

Not used: the `codemirror` package (`basicSetup`) and the `markdown()` helper. The helper pulls in the HTML, CSS and JavaScript parsers and autocomplete; those packages are installed as dependencies of `@codemirror/lang-markdown` but are not in the bundle (checked: no HTML-parser, completion, search, lint, fold or tooltip code in `dist`). `defaultKeymap` was left out in favor of `standardKeymap` because its extras are for code (Cmd-/ comments, Cmd-[ and Cmd-] indent, Cmd-I select syntax).

| `pnpm build` output | Before | After |
| --- | --- | --- |
| JS | 246.87 kB (gzip 77.89 kB) | 558.00 kB (gzip 179.63 kB) |
| CSS | 9.51 kB (gzip 2.57 kB) | 10.60 kB (gzip 2.80 kB) |

The app is one chunk read from disk, so `vite.config.ts` raises `build.chunkSizeWarningLimit` to 700 kB to keep the build output free of a warning that does not apply; anything past that still warns.

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm build` | Pass. |
| `pnpm test` | Pass: 15 files, 181 tests (57 new: 28 meeting view, 14 autosave, 11 editor, 4 line breaks). Also passes with `TZ=Asia/Tokyo LANG=de_DE.UTF-8`. |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| Tests fail when the code is broken | 34 single changes were applied one at a time by a script and reverted; every one made at least one test fail. They include: text trimmed on save, no debounce, debounce never firing, no flush on leaving, on blur, before stop, stop not waiting, stop waiting without limit, no retry, failed save marking the text saved, no warning, overlapping saves, undo during a save not re-saved, no maximum wait, reopen ignoring unsaved text, Enter or blur not renaming, failed rename keeping the new title, rename error hidden, empty title sent, Escape not restoring, no autofocus for a new meeting, autofocus for every meeting, cursor at the start, Settings shown while recording, a Transcript button while recording, list continuation off, Tab off, CRLF not kept, editor outside a shadow root, load error hidden. |

Tests per acceptance item, all in `src/screens/MeetingScreen.test.tsx` unless noted: debounced save with the exact text ("saves the exact text shortly after typing pauses", "waits for the pause"); leaving flushes ("leaving the view saves what was still waiting", "closing the view saves too"); failed save keeps the text and retries ("a failed save keeps the text, warns quietly and retries until it works", and `src/lib/autosave.test.ts`); rename on Enter and restore on error ("Enter saves the new title…", "a rename that fails says why in place and puts the old title back"); no transcript UI while recording ("shows Back, the title, the editor and the recording bar, and no transcript": no text matching "transcript", `api.getTranscript` never called while the mock produced transcript lines, the only buttons are Back and Stop); focus for a new meeting ("a new meeting opens with the cursor in the editor"). Also covered: blur, stop order (`saveNotes` before `stopMeeting`), no overlapping saves, CRLF, reopening with unsaved text, list continuation, Tab.

How the editor is tested: CodeMirror's author does not support the view under jsdom, and the practice he and others point to is to stub the missing geometry and selection methods and drive the editor through its own API (`EditorView.findFromDOM`, then transactions). `src/test/jsdomGaps.ts` stubs `Range.getClientRects`, `Range.getBoundingClientRect`, `Selection.getComposedRanges` and `document.execCommand` (each only if missing). `src/test/editor.ts` has `notesEditor()`, `notesText()`, `typeInNotes(text)`, `pressInNotes(key)` (a real `keydown` on the editor, so key bindings run), `blurNotes()` and `notesHaveFocus()`. Testing Library queries do not see into the shadow root, which is why these exist. Tests assert on what reaches `api.saveNotes` and on the mock's stored notes. jsdom reports its vendor as Apple, so CodeMirror takes its Safari code path for the selection in a shadow root there.

Three tests from 015 and 016 read the meeting title from the `h1`'s text; the title is now a field, so they read the field's value instead (`src/shell/RecordingBar.test.tsx`, `src/screens/HomeScreen.test.tsx`). Tests that find the meeting by `heading` with its name still pass unchanged, because the field sits inside the `h1`.

Browser check, in the Chromium-based browser pane against `pnpm dev` on http://localhost:1420, with `prefers-color-scheme` emulated. Both servers were stopped afterwards.

- Light and dark at 960×680: a document with every kind of markup (three heading levels, bold, italic, strikethrough, inline code, link, autolink, bullets, nested bullet, numbered list, tasks, quote, code block), looked at in screenshots and through computed styles. Headings, bold and dimmed markers render in both.
- The live view: New meeting opened it with the cursor already in the editor (typed without clicking); only Back, the title and the recording bar with Stop were on screen. Dark at 960×680, light at 640×480 (the minimum window size).
- Failed save (mock `failNext("saveNotes")`): the warning appeared above the notes and went away after the retry. Dark only.
- Failed rename (mock `failNext("renameMeeting")`): the message appeared and the title went back. Dark at 960×680 and light at 640×480.
- Successful rename with Enter: the title stayed, and the next keys went into the notes. Light.
- Stop: "Stopping…" seen in a screenshot, then the bar went, the notes stayed, Settings came back. Light at 640×480.
- Empty meeting: the dimmed "Notes" placeholder. A click in the left margin put the cursor in the editor. Light.
- No console errors in any of it.

### Not verified

- **WKWebView (the real app).** Nothing was run in WebKit; `pnpm tauri dev` was not run. Specifically unverified there: (1) that WebKit also leaves constructed stylesheets alone under `style-src 'self'` (`adoptedStyleSheets` exists since Safari 16.4; the policy applying only to style elements and attributes is how the spec and Chromium behave); (2) that `element.style.cssText` is not reported there (if it were, the only effect is the default tab size and a console line); (3) typing, caret, selection, IME and copy/paste inside a shadow root (CodeMirror uses `Selection.getComposedRanges` on Safari 17 and has Safari-specific shadow DOM fixes as recent as 2026, but this was only exercised in Chromium and, for the Safari code path, in jsdom); (4) that `-webkit-user-select: none` on `body` does not affect the editor (the theme sets it back to `text` on the content element).
- **The effective policy inside Tauri.** Tauri may add nonces or hashes to the configured policy at build or load time. That can only allow more, but the header the app really gets was not looked at.
- **`pagehide` and window blur as save triggers in the app.** Blur was tested as a DOM event; whether WKWebView fires `blur` on the editor when the app is deactivated, and `pagehide` when the window closes, was not checked.
- **The real backend.** Everything ran against the mock.
- **Failed-save warning in light, successful rename in dark:** looked at in one theme each, as listed above.

### Decisions and deviations

- **Shadow DOM** for the editor: forced by the policy, see above. The cost is that page CSS and Testing Library do not reach inside; the theme and the test helpers account for it.
- **Settings is hidden only while this meeting is live**, not on every meeting view, so the path Home → meeting → Settings from 015 still exists for finished meetings.
- **Additions beyond the scope list**, all small: Tab/Shift-Tab indent, the 5 s maximum wait, the 3 s cap on how long Stop waits, the "Try now" button, Escape in the title, focus moving to the notes after Enter in the title, the "Notes" placeholder, CRLF preservation, keeping unsaved text across visits.
- **Spellcheck and autocorrect are off** (CodeMirror's default for its content element). Turning them on is a product call, see follow-ups.
- **Dependency versions are caret ranges** (`^6.43.15`), matching the existing entries; the lockfile pins the exact versions listed.
- Wrapped lines of a list item are not hang-indented under the item's text. That needs per-line decorations and was left out.

### For 018 to 020

- `MeetingScreen` already loads the meeting with `useLoad` and knows `isLive` from `useRecording`. `MeetingNotes` reads `meeting.id` and `meeting.notes` once, so reloading the meeting after stop (for status and line count) does not reset the editor. Add the pane next to `<section className={styles.notes}>` inside `.body`, and the buttons through `actions`.
- `NotesEditor` takes a `ref` with `focus()`, for returning focus after the pane closes.
- `Autosave` (`src/lib/autosave.ts`) is generic: any "save this text a little after it changes, never lose it" need can use it with `useSyncExternalStore(saver.subscribe, () => saver.status)`.
- `notices.module.css` has the layout for a status line above the notes (018's "recording ended unexpectedly" note fits there).
- Test helpers in `src/test/editor.ts`; `setup.ts` resets pending autosaves after each test.

### Follow-ups

- **Whoever first runs the app window (021):** check the four WKWebView points above. If CodeMirror's styles are blocked there after all, the fallback is the nonce route: `EditorView.cspNonce` plus `style-src 'self' 'nonce-<value>'` in `src-tauri/tauri.conf.json` with a per-load nonce the page can read; that is a change to the policy and to the app lane.
- **021, 022, 023:** a stop from the menu bar or the shortcut that does not go through `useRecording().stop()` skips the flush; the notes are then saved by blur or the 800 ms pause instead. Same for quitting the app: the backend should ask the window to flush (or wait a second) before closing, since `pagehide` is best effort.
- **021:** `saveNotes` must be safe to call after `stopMeeting` and for a meeting that is not live (a late or retried save can arrive then), and calls arrive strictly one after another per meeting.
- **021:** `renameMeeting` is sent a trimmed, non-empty title and its returned `title` is what the field shows.
- **Product decision:** spellcheck and macOS autocorrect in the notes editor (one line in `extensions.ts`: `EditorView.contentAttributes.of({ spellcheck: "true", autocorrect: "on" })`), to be judged in the real window.
- **Later:** a failed save that is still retrying after the view was left is not shown anywhere until that meeting is opened again; a global indicator could go with the recording bar. Hanging indent for wrapped list items. Cmd-B and Cmd-I for bold and italic.
- Root `README.md` (not owned by this lane) still says the window shows a placeholder list, and does not mention `src/editor/`.
- `issues/README.md` still lists 017 as in progress; this file's status line is the source of truth.
