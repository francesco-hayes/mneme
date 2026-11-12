# 014: Settings file

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 013 · Owns: `crates/mneme-store/`

## Goal

Persist the few app settings across restarts.

## Scope

1. **`Settings`**: `default_model_id` (none until first download), `language` (default `auto`), `global_shortcut` (none by default), `meetings_root` (default `~/Documents/Mneme`).
2. **`SettingsStore`** over a file path (`settings.json` in the app data folder; the path is passed in, this crate does not know about Tauri). Load, update with a partial patch, save through the safe write helper.
3. Missing file gives defaults. A corrupt file gives defaults and is kept aside as `settings.json.bad` rather than overwritten silently. Unknown keys are kept on save.
4. Serialized field names match the frontend contract in PRD 05 (`defaultModelId`, `language`, `globalShortcut`).

## Acceptance criteria

- [ ] Tests: defaults when missing; patch updates only named fields; values survive reopen; corrupt file falls back and is preserved; unknown keys survive.
- [ ] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
