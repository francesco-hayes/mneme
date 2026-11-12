# 013: Meeting list and crash recovery

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 012 · Owns: `crates/mneme-store/`

## Goal

List meetings from the folders alone, find one by id, and repair meetings a crash left open.

## Scope

1. **List**: scan the root's direct subfolders, read each `notes.md` frontmatter, return summaries (id, title, date, duration, status, folder path) newest first by `date`. Skip folders with no `notes.md` or unreadable frontmatter, and report them as warnings rather than failing the list.
2. **Find by id**: resolve an id to its folder. Works after the folder was renamed outside Mneme. A simple scan is fine; an in-memory cache refreshed on list is optional.
3. **Load meeting**: summary plus notes body and transcript line count.
4. **Recovery**: for every meeting with `status: recording` (none should exist at launch): finalize the transcript (sort), set `duration` from the last transcript timestamp (zero if empty), set `status: interrupted`, apply any pending folder rename. Returns what it repaired.
5. Edits made outside Mneme (title changed in frontmatter, notes edited in Obsidian) show up on the next list or load; nothing is cached on disk.

## Acceptance criteria

- [ ] Tests: ordering by date not by folder name; skip and warn on junk folders and files in the root; find by id after an external folder rename; duplicate ids are reported, not silently merged.
- [ ] Recovery tests: a meeting left as `recording` with out-of-order transcript lines and a truncated last line becomes `interrupted`, sorted, with the right duration; running recovery twice changes nothing the second time.
- [ ] Listing 500 generated meetings completes fast enough for a launch (state the measured time in the Result).
- [ ] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
