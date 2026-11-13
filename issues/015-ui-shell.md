# 015: UI shell: routing, theme, recording bar

Status: done · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 001 · Owns: `src/` (except `src/lib/api/tauri.ts`), frontend config files

## Goal

The frame every screen lives in: navigation between views, Mac-like theme, and a recording bar that is visible everywhere.

## Scope

1. **Routing** between Home, Meeting (by id), Settings and First run. Simple in-app routing with back navigation; no URL bar is visible in the app. Placeholder components for screens later issues build.
2. **Theme**: design tokens as CSS variables. System font stack, light and dark following the system, system accent color (`AccentColor` with a fallback), comfortable Mac-like spacing and control sizes. No UI framework dependency beyond what is needed; plain CSS or CSS modules.
3. **Window chrome**: content laid out so the native title bar looks right; a top bar area with the screen title and navigation (back, Settings).
4. **Recording state store**: one app-wide subscription to `api.getRecordingState` / `api.onRecordingState`.
5. **Recording bar**: shown on every screen while recording or stopping. Timer, a clear "recording" indicator, Stop button, and (when not already on that meeting) a way to jump to the live meeting. Hidden when idle. No animation beyond a subtle indicator.
6. **First-run gate**: if no model is installed, route to First run instead of Home.
7. Shared small components later screens need: button, icon button, list row, empty state, inline error.

## Acceptance criteria

- [x] Works in the browser dev server against the mock API.
- [x] Tests: navigation between views; recording bar hidden when idle, shows the timer while recording, shows a stopping state; the gate routes to First run when the mock has no installed model.
- [x] Light and dark both render correctly (check each in the browser and say how it was checked).
- [x] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

Done 2026-11-12. No dependency was added; `package.json`, the lockfile and the other config files are unchanged, and so is everything under `src/lib/api/` and `src/test/setup.ts`.

### What was built

```
src/
  App.tsx                 RecordingProvider, the first-run gate, then the shell
  main.tsx                loads styles/tokens.css and styles/base.css, then App
  styles/
    tokens.css            design tokens (CSS variables), light and dark
    base.css              page rules, focus ring, .selectable, .visually-hidden
  state/
    navigation.ts         Route, useNavigation, the stack reducer
    NavigationProvider.tsx
    recording.ts          useRecording, useBeforeStop
    RecordingProvider.tsx the one subscription to the recording state
  shell/
    AppShell.tsx          current screen + recording bar; the route-to-screen switch
    RecordingBar.tsx
    startup.ts            startupRoute(models): the first-run gate rule
  components/             Button, IconButton, List/ListRow, EmptyState, InlineError,
                          Screen (top bar + content), icons; import from "../components"
  screens/                HomeScreen, MeetingScreen, SettingsScreen, FirstRunScreen
                          (placeholders for 016 to 020)
  lib/
    useLoad.ts            loading / ready / error for an API read, with reload
    format.ts             formatElapsed; errors.ts: errorMessage; cx.ts: class names
  test/render.tsx         renderShell, renderWithProviders, advance (fake timers)
```

- **Routing.** A hand-rolled stack of routes in React state (`useReducer`), not a router library. Four screens, no URL bar and no deep links did not justify a dependency. `useNavigation()` gives `route`, `canGoBack`, `navigate(route)`, `back()` and `reset(route)`. Navigating to a screen that is already further back in the stack returns to it, so Back never loops. The location is not kept in the URL, so a reload in the browser goes back to the start screen.
- **Theme.** Tokens in `src/styles/tokens.css`: type (`--font-ui`, `--font-mono`, `--text-caption|small|body|headline|title`, `--line-*`, `--weight-*`), space (`--space-1` to `--space-10`, 4px grid), radius, control and layout sizes (`--control-height` 28px, `--control-height-small` 22px, `--topbar-height`, `--recording-bar-height`, `--content-width`), and colors (`--color-bg`, `--color-chrome`, `--color-text`, `--color-text-secondary`, `--color-text-tertiary`, `--color-separator`, `--color-hover`, `--color-pressed`, `--color-control*`, `--color-accent`, `--color-on-accent`, `--color-accent-soft`, `--color-focus-ring`, `--color-recording`, `--color-danger`, `--color-danger-soft`, `--color-warning`). Dark values sit under `prefers-color-scheme: dark`. The accent is `AccentColor` / `AccentColorText` inside `@supports (color: AccentColor)`, with macOS blue as the fallback. `light-dark()` is not used because Safari 17.4 (macOS 14.4) does not have it. Components use CSS modules; nothing sets a `style` attribute or injects a `<style>` tag.
- **Window chrome.** Every screen renders one `<Screen title actions back settings layout>`: a 44px top bar (Back when there is somewhere to go back to, the title, the screen's actions, Settings) over the content area. `layout="column"` is a centered scrolling column; `layout="fill"` gives the whole area to an editor or split view. The window itself does not scroll or bounce, and UI text is not selectable unless marked `.selectable`.
- **Recording state.** `RecordingProvider` holds the one subscription (`onRecordingState`, seeded by `getRecordingState`). `useRecording()` gives `state`, `isActive`, `isStopping`, `stop()`, `stopError` and `onBeforeStop`. `stop()` waits for the handlers registered with `useBeforeStop(handler)`, then calls `api.stopMeeting`; a handler that fails does not block the stop. Issue 017 uses `useBeforeStop` to flush notes.
- **Recording bar.** At the bottom of every screen while recording or stopping, absent when idle: a red dot that slowly dims and returns (still under `prefers-reduced-motion`), "Recording", the timer, "Open meeting" when the current screen is not the live meeting, and Stop. While stopping it reads "Stopping…", the red goes gray and Stop is disabled. A failed stop is shown inside the bar.
- **First-run gate.** `App` loads the model list once at startup; `startupRoute` opens First run when no model is installed and Home otherwise. A failed load shows an inline error with Retry. The gate decides the opening screen only: First run leaves with `navigation.reset({ name: "home" })`, and deleting the last model later does not force the user back into First run.
- **Placeholders.** Home lists meeting titles and has a New meeting button, so the shell can be walked through in the browser; Meeting shows the title; Settings and First run show a note. Each says which issue builds it.

### Verification

| Command or check | Outcome |
| --- | --- |
| `pnpm test` | Pass: 8 files, 76 tests (39 new in 6 files, plus the scaffold's API tests). |
| `pnpm lint` | Pass (ESLint, then Prettier check). |
| `pnpm build` | Pass. Output is one JS file and one linked CSS file. |
| Tests fail when the code is broken | Checked by breaking the idle check, the stopping label, the gate rule and `back` in turn: 14 tests failed. Restored afterwards. |

Tests per acceptance criterion: navigation in `src/App.test.tsx` and `src/state/navigation.test.ts`; the recording bar (hidden when idle, running timer, stopping state, failed stop, link to the live meeting, a recording started outside the window, before-stop handlers) in `src/shell/RecordingBar.test.tsx`; the gate in `src/App.test.tsx` (first-run scenario, and models present but none installed).

Browser check, in the Chromium-based browser pane of the coding tool against `pnpm dev` on http://localhost:1420, with `prefers-color-scheme` emulated as light and then dark, at 960×680 and at the minimum window size 640×480:

- Light and dark: Home (idle and while recording), a live meeting and `/?mock=first-run` were each looked at in both; Settings in dark only (it uses the same components as First run). Backgrounds, text, separators, the primary button, the disabled button, the recording bar tint and the inline error all changed with the scheme and stayed readable.
- Flow: New meeting started a recording and opened the meeting; the bar showed the timer counting; Back to Home and on to Settings kept the bar and showed "Open meeting", which returned to the live meeting; Stop showed "Stopping…" with Stop disabled and a gray, still dot (read from the DOM 150 ms after the click), and the bar was gone after the mock's 600 ms.
- Errors: a failed stop appeared inside the bar (looked at in dark only), and a failed start appeared inline on Home (light and dark).
- Keyboard focus ring looked at on the Back button, in light only.
- Content Security Policy: the built `dist/` was served by a throwaway local server that sent the app's policy from `src-tauri/tauri.conf.json` as a real header. The app ran with no console messages, one stylesheet, no `<style>` elements and no `style` attributes.

### Not verified

- **The real app window.** `pnpm tauri dev` was not run (other lanes were mid-change), so nothing was seen in WKWebView: how the top bar sits under the native title bar, the system font rendering, and whether `AccentColor` follows the accent chosen in System Settings there. In Chromium `AccentColor` is supported and resolved to a fixed blue.
- **Safari / WebKit** was not used at all; every browser check was in Chromium.
- **Styles under `pnpm tauri dev`.** Vite's dev server injects `<style>` tags. If Tauri applies `style-src 'self'` to the dev URL, the dev window would be unstyled even though the built app is fine. Not checked.
- The stopping state was confirmed through the DOM and computed styles, not in a screenshot (it lasts 600 ms in the mock).

### Follow-ups

- Issue 021 (or whoever first runs the app window): look at the top bar against the native title bar in light and dark and adjust `--color-chrome` if the seam shows; confirm `AccentColor` follows the system accent in WKWebView; check styles under `pnpm tauri dev` and add a `devCsp` if they are blocked.
- Issue 021: the timer shows `elapsedSecs` exactly as the backend sends it and does not count on its own. The backend has to emit the recording state about once a second while recording, as the mock does.
- Issues 022 and 023: a recording started or stopped from the menu bar or the shortcut is picked up by the bar (tested against the mock), but a stop that does not go through `useRecording().stop()` skips the before-stop handlers, so notes typed in the last moments are saved by the editor's own autosave, not flushed first. If that matters, the backend should ask the window to stop instead of stopping directly.
- Root `README.md` (not owned by this lane) still says the window shows a placeholder list of mock meetings, and its layout section does not describe `src/`.
- In the dev server, React StrictMode runs each load twice, so a single `__mnemeMock.failNext(...)` on a call made while a screen loads is used up by the discarded first call. Use it on calls made from a click, or test load failures in Vitest.
- Mac keyboard shortcuts for navigation (Cmd-[ for Back, Cmd-, for Settings) were left out; they belong with the app menu.
