# 016: Home screen

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 015 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

MVP feature 4: a list of meetings, newest first, and one button to start a new one.

## Scope

1. **List** from `api.listMeetings`: title, date (friendly: "Today 2:30 PM", "Oct 27"), duration ("32 min"). Newest first. Clicking a row opens the meeting full window.
2. **States**: loading, empty ("No meetings yet" with the New meeting button), error with retry. A meeting with status `interrupted` is marked as such; one that is `recording` shows it is live.
3. **New meeting button**: calls `api.startMeeting` and navigates to the live view. Disabled with a short explanation and a link to Settings when no model is installed. Disabled while a recording is already running (the recording bar is the way back to it). Errors from start (for example a denied permission) are shown inline, not as a popup.
4. The list refreshes when returning to Home and when recording stops.
5. Keyboard: up/down moves through rows, Enter opens, Cmd-N starts a new meeting.

## Acceptance criteria

- [ ] Tests: ordering; date and duration formatting (including a meeting with no duration); empty, loading and error states; New meeting disabled without a model and while recording; clicking a row navigates; a start error is shown.
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
