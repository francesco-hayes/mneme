# 018: Meeting after view and transcript pane

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 017 · Owns: `src/` (except `src/lib/api/tauri.ts`)

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

- [ ] Tests: pane closed by default and after stop; button shows the line count; toggle opens and loads once; speaker styles applied per line; zero-line and interrupted states; Show in Finder calls the API; notes stay editable in the after state.
- [ ] A 2,000-line mock transcript scrolls without visible lag (say how it was checked).
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
