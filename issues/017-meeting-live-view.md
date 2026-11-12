# 017: Meeting live view and editor

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 016 · Owns: `src/` (except `src/lib/api/tauri.ts`)

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

- [ ] Tests: typing triggers a debounced save with the exact text; leaving the view flushes a pending save; a failed save keeps the text and retries; rename saves on Enter and restores on error; no transcript UI is present while recording; focus lands in the editor for a new meeting.
- [ ] The editor renders headings, bold and dimmed markers in both themes (checked in the browser).
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
