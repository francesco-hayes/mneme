# 026: Folder name suffix for same-minute meetings

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 011 · Owns: `crates/mneme-store/`

## Goal

Starting a second meeting in the same minute as another must never fail. Decided 2026-11-12: the new folder gets a numeric suffix.

## Background

Issue 010 found that start, stop, start again within one minute asks for the same folder name (`2026-10-27 1430 Meeting`), and `create_meeting` returns `FolderExists`. The scope doc had assumed this could not happen.

## Scope

1. **Create**: when the folder name a new meeting wants is taken, use the same name with ` 2` appended, then ` 3`, and so on: `2026-10-27 1430 Meeting 2`. The first meeting keeps the plain name. Creation and the check are atomic with respect to other creates in the same `Store`, and a folder that appears from outside between check and create is handled by trying the next number, not by failing.
2. The suffix belongs to the folder name only. The display title in frontmatter is unchanged (both meetings show "Meeting, Oct 27 2:30 PM").
3. **Later behavior stays consistent** with a suffixed folder:
   - A suffixed, never-retitled folder is not mistaken for a hand-renamed folder, and is not "corrected" back to the plain name by rename, finish or recovery.
   - When the title is edited, the folder moves to the new title's name with no suffix if that name is free.
   - A rename by the user onto a name that is taken still fails with `FolderExists`, as issue 010 built it. The suffix is for automatic names at creation, not for titles the user chose.
4. Decide and document where the suffix sits relative to the length cap from issue 010, so a long title plus a suffix still fits.

## Acceptance criteria

- [ ] Tests: two creates in the same minute give `… Meeting` and `… Meeting 2`; a third gives `… Meeting 3`; ids differ; both list as separate meetings with the same display title.
- [ ] Test: a gap is reused or skipped by a stated rule (for example, with `Meeting` and `Meeting 3` present, say which name the next one gets) and the rule is in the Result.
- [ ] Tests: finish, rename and (if present by then) recovery leave a suffixed default-titled folder where it is; editing the title moves it to the unsuffixed new name; user rename onto a taken name still fails cleanly.
- [ ] Test: concurrent creates from several threads in the same minute all succeed with distinct folders.
- [ ] Existing tests still pass; `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
