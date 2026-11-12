# 012: Notes safe writes and note timeline

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 011 · Owns: `crates/mneme-store/`

## Goal

Save notes without ever leaving a half-written file, and record when each note line was typed.

## Scope

1. **Read notes**: return the body of `notes.md` without frontmatter.
2. **Save notes**: replace the body, keep frontmatter byte-for-byte, through the safe write helper.
3. **Note timeline**: `save_notes_at(body, meeting_time_ms)` used while recording. Compare the new body with the previously saved body line by line and append one JSON line to `notes-timeline.jsonl` for each line that is new or changed: `{"t":"hh:mm:ss","text":"<line>"}`.
   - Blank lines and lines that only moved are not recorded.
   - Deleted lines are not recorded.
   - Append-only and flushed on each save.
   - The first save diffs against the body on disk, so restarting the app mid-meeting does not re-log every line.
4. Saves when not recording do not touch the timeline.

## Acceptance criteria

- [ ] Tests: body save keeps frontmatter exactly, including unknown keys; a body containing `---` survives; typing three lines over three saves yields three entries with the right times; editing one line yields one entry with the new text; reordering lines yields none; saving the same body twice yields none; saves while not recording yield none.
- [ ] Every timeline line is valid JSON on its own line; text with quotes and unicode round-trips.
- [ ] Test: a crash between temp write and rename leaves the old `notes.md` intact.
- [ ] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
