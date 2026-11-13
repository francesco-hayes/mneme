# 025: Signing, notarization, local-only audit

Status: todo · PRD: [06](../prds/06-integration-and-finish.md) · Lane: release · Depends on: 021 · Owns: `scripts/`, `docs/`, root `README.md`, new overlay config files in `src-tauri/`

## Goal

Dev builds that keep their permissions between rebuilds, a scripted path to a notarized release, and proof the app is local-only.

## Running alongside 022–024

Order changed 2026-11-12: this issue runs at the same time as 022, 023 and 024, which own `src-tauri/`. So it must not edit any existing file under `src-tauri/` (`tauri.conf.json`, `Cargo.toml`, sources, capabilities). Signing and release settings go in separate overlay config files that the Tauri CLI merges with `--config` (for example `src-tauri/tauri.release.conf.json`), plus scripts. Anything that truly needs a change to an existing `src-tauri/` file is written under Follow-ups for the orchestrator to apply after 024. The entitlements file and `Info.plist` usage strings belong to 024; reference them, do not create them. The manual-check list in `docs/release.md` is assembled from the issue files as they stand; note which of 022–024 were not finished when it was written.

## Scope

1. **Dev signing**: a script that creates (once) a self-signed code-signing certificate in the login keychain and a documented way to make dev and debug builds sign with it, so macOS ties permissions to one stable identity. The script only prints what it would do unless run with an explicit flag, and never touches an existing identity. Do not run it in this issue; creating keychain identities is Francesco's step.
2. **Release signing**: Tauri bundle config and a script for Developer ID signing and notarization with `notarytool`, reading credentials from the environment or a keychain profile, never from the repo. Documented step by step. It cannot be run until an Apple Developer account exists; say so at the top of the doc.
3. **Local-only audit**:
   - A check (script or test) that lists every crate in the dependency tree that can open network connections and fails if one appears outside the allowlist for the model downloader.
   - Confirm no updater, analytics or crash-reporting plugin is present and the CSP blocks remote content.
   - A short checklist for the Wi-Fi-off test.
4. **`docs/release.md`**: build, sign, notarize, and the manual test list gathered from every issue's "Manual checks".
5. Update the root README with current build and run instructions.

## Acceptance criteria

- [ ] The audit check runs and passes; adding a network crate to a non-allowlisted crate would fail it (show this was tried and reverted).
- [ ] Scripts pass a shell linter or a dry run; nothing in the repo contains credentials or identities.
- [ ] `docs/release.md` lists every manual check from issues 002–024.
- [ ] `pnpm tauri build --debug` still works unsigned.

## Manual checks

- [ ] Run the dev-signing script; rebuild twice; macOS does not re-ask for mic or system audio permission.
- [ ] With Wi-Fi off and a model installed: record, stop, read the transcript.
- [ ] Once a Developer ID exists: the release script produces a notarized app that opens on another Mac.

## Result

_Filled in by the implementer._
