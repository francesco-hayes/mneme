# 015: UI shell: routing, theme, recording bar

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 001 · Owns: `src/` (except `src/lib/api/tauri.ts`), frontend config files

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

- [ ] Works in the browser dev server against the mock API.
- [ ] Tests: navigation between views; recording bar hidden when idle, shows the timer while recording, shows a stopping state; the gate routes to First run when the mock has no installed model.
- [ ] Light and dark both render correctly (check each in the browser and say how it was checked).
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
