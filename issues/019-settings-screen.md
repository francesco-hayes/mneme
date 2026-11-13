# 019: Settings screen

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 018 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

MVP feature 5 and the model manager: pick the default model, manage downloads, set language and the global shortcut.

## Scope

1. **Models**: every catalog model from `api.listModels` with name, size, note and an English-only tag. Installed models show a Delete action (with a confirm) and can be chosen as the default. Others show Download.
2. **Downloads**: progress bar with received and total size from `api.onDownloadProgress`; a verifying state for the checksum; Cancel; errors shown on the row with Retry. Progress survives leaving and returning to Settings while the download runs.
3. **Default model**: one installed model is the default, saved with `api.updateSettings`. The first installed model becomes the default automatically. Deleting the default moves the default to another installed model, or to none.
4. The model in use by a running recording cannot be deleted.
5. **Language**: `Auto` or a language from a short list of common ones plus a searchable full list. If the default model is English-only and the language is not English or Auto, show a clear warning next to both controls.
6. **Global shortcut**: a recorder field. Unset by default, shown as "None". Click, press a combination, saved through `api.updateSettings`; a Clear button. Requires at least one modifier. A registration error from the backend is shown and the previous value restored.
7. Reusable pieces (model row, download progress) are exported for First run (issue 020).

## Notes from earlier issues

- From 007: the real catalog has ten models: `tiny`, `tiny.en`, `base`, `base.en`, `small`, `small.en`, `medium`, `medium.en`, `large-v3-turbo`, `large-v3` (77 MB to 3.1 GB). The mock's five ids are all valid; align the mock's catalog with the real one (ids, names, sizes, notes are in issue 007's Result and `crates/mneme-transcribe/src/models/catalog.rs`, which you may read but not edit) so the list is designed for ten rows.
- From 007: with the real backend, `cancelled` arrives shortly after `cancelDownload` returns, not at once, and can take up to about 35 s if the connection was still being set up. Show a cancelling state. The first `downloading` event may arrive just after `downloadModel` resolves. `verifying` is brief.
- From 007: deleting a model that is not installed is an error ("Base is not installed."). Disk space is not checked before a download; a full disk arrives as a download error.
- From 005: the engine validates language codes but exports no list of languages; the UI carries its own list.

## Acceptance criteria

- [ ] Tests: list rendering; download progress through to installed; cancel; error and retry; first install becomes default; deleting the default reassigns; the English-only warning; shortcut record, clear, modifier rule and error restore.
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
