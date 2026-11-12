# 020: First run

Status: todo · PRD: [05](../prds/05-ui.md) · Lane: ui · Depends on: 019 · Owns: `src/` (except `src/lib/api/tauri.ts`)

## Goal

Get one model installed and permissions granted before the first meeting, in as few steps as possible.

## Scope

1. Shown by the gate from issue 015 whenever no model is installed.
2. **Step 1, model**: a short explanation that transcription runs on this Mac and needs a model. A recommended model is preselected (take the recommendation from the benchmark doc if issue 008 has produced one; otherwise a small multilingual model, marked as a placeholder choice). The full list is available with size and speed notes. Download with progress, checksum verification state, cancel and retry, reusing the Settings pieces.
3. **Step 2, permissions**: microphone and system audio, each with one sentence on why, current status from `api.getPermissions`, and a button calling `api.requestPermission`. A denied permission explains how to fix it in System Settings. System audio can be skipped (recording then captures only me); the microphone cannot.
4. A one-line recommendation to use headphones, with the reason (without them the mic hears the other side).
5. **Finish** goes to Home. The downloaded model is the default.
6. Quitting mid-download and reopening returns to first run cleanly.
7. One sentence stating that nothing leaves this Mac and the download is the only network use.

## Acceptance criteria

- [ ] Tests: gate shows first run with no models; download flow reaches the permissions step; denied and granted states render; system audio skip is allowed and mic skip is not; finishing lands on Home with the default model set; a failed download can be retried.
- [ ] `pnpm build`, `pnpm test` and `pnpm lint` pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
