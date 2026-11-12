# 010: Meeting folders, frontmatter, naming

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 001 · Owns: `crates/mneme-store/`

## Goal

Create a meeting as a folder with a `notes.md` whose frontmatter holds its identity, and rename it safely.

## Scope

1. **`Store`** opened on a root folder (created if missing).
2. **Create meeting**: given a start time, makes `YYYY-MM-DD HHMM Meeting/` (local time) with `notes.md` (frontmatter, empty body), an empty `transcript.md` and an empty `notes-timeline.jsonl`. Frontmatter keys as in PRD 04: `id`, `title` (default display title like `Meeting, Oct 27 2:30 PM`), `title_edited: false`, `date` with UTC offset, `duration` empty, `status: recording`, `model`, `language`.
3. **Frontmatter read/write**: parse and rewrite without touching the body; keep unknown keys and their order where practical; tolerate a file with no frontmatter or broken frontmatter by returning an error for that meeting only.
4. **Safe write helper** used by everything in the crate: write to a temp file in the same folder, sync, rename over the target.
5. **Folder name rules**: sanitize the title part (remove path separators, control characters, leading dots, trailing spaces and dots; collapse whitespace; cap the length); empty result becomes `Meeting`.
6. **Rename**: sets `title` and `title_edited: true`, then renames the folder to match. Refused while `status` is `recording`: the title is saved to frontmatter and the folder rename is applied by a `finish` call at stop. If the target folder exists, return an error and change nothing.
7. **Finish**: sets `duration` and `status: complete`, applies any pending folder rename.

## Acceptance criteria

- [ ] Tests in temp folders: create; frontmatter round-trip with unknown keys and a body containing `---` lines; sanitizing nasty titles; rename after completion; rename during recording is deferred and applied at finish; rename onto an existing folder fails cleanly; the id never changes across renames.
- [ ] Created files open as plain Markdown with valid YAML frontmatter.
- [ ] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
