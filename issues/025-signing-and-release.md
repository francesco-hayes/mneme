# 025: Signing, notarization, local-only audit

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: release · Depends on: 021 · Owns: `scripts/`, `docs/`, root `README.md`, new overlay config files in `src-tauri/`

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

- [x] The audit check runs and passes; adding a network crate to a non-allowlisted crate would fail it (show this was tried and reverted).
- [x] Scripts pass a shell linter or a dry run; nothing in the repo contains credentials or identities.
- [x] `docs/release.md` lists every manual check from issues 002–024.
- [ ] `pnpm tauri build --debug` still works unsigned. (Not run: see Result. No existing build input was changed.)

## Manual checks

- [ ] Run the dev-signing script; rebuild twice; macOS does not re-ask for mic or system audio permission.
- [ ] With Wi-Fi off and a model installed: record, stop, read the transcript.
- [ ] Once a Developer ID exists: the release script produces a notarized app that opens on another Mac.

## Result

Implemented 2026-11-12. Nothing was signed, no keychain item was created, Apple was not contacted, and no existing file under `src-tauri/` was edited.

### What was built

- `scripts/audit-local-only.sh`: the local-only check. Read-only and offline (`cargo tree --locked --offline`, `jq`, `grep`). Five checks: network-capable crates reachable outside `mneme-transcribe`; such crates inside `mneme-transcribe` but not behind `ureq`; updater, analytics and crash-reporting crates and npm packages; the Tauri config (CSP has no remote source and has a `default-src`, no updater, no remote window or frontend URL, each overlay checked merged over the main config, capabilities grant nothing remote); socket types from the standard library named in workspace sources outside the downloader. `--deny NAME` adds a crate to the denylist, `--list` prints it.
- `scripts/dev-signing.sh`: prints its plan by default; with `--apply` creates a self-signed code-signing certificate `Mneme Dev Signing` in the login keychain. Exits without changing anything if an identity or certificate of that name exists.
- `src-tauri/tauri.dev-signing.conf.json`: overlay for dev builds (`signingIdentity`, hardened runtime off). Used as `pnpm tauri build --debug --config src-tauri/tauri.dev-signing.conf.json`; `APPLE_SIGNING_IDENTITY` is the alternative.
- `scripts/release.sh` and `src-tauri/tauri.release.conf.json`: audit, build and sign with `APPLE_SIGNING_IDENTITY` (Developer ID, hardened runtime), verify, notarize with `xcrun notarytool submit --wait`, staple, Gatekeeper check, zip. Notary credentials from a keychain profile (`MNEME_NOTARY_PROFILE`) or an App Store Connect API key (`APPLE_API_KEY_PATH`, `APPLE_API_KEY`, `APPLE_API_ISSUER`). Stops before building when anything is missing. `--dry-run` prints every step.
- `docs/release.md`: build, dev signing, release, the audit, the Wi-Fi-off checklist, a pre-release list, and the manual test list from the issue files.
- Root `README.md`: requirements now name cmake and the Xcode Command Line Tools; build and run; the three dev tools; states that the backend commands are not wired (issue 021), since `src/lib/api/tauri.ts` was still a stub.

### What was run

- `shellcheck scripts/*.sh` (it was installed): clean. `bash -n` on each: ok.
- `scripts/audit-local-only.sh`: passed, exit 0. Network-capable crates in the tree: `ureq`, `ureq-proto`, `rustls`, `rustls-webpki`, `webpki-roots`, all behind `mneme-transcribe` and `ureq`. `tokio` is present without `net`. `mneme-pipeline` already depends on `mneme-transcribe`, so the "only through the downloader" path was exercised for real.
- Failure shown twice, then reverted. `scripts/audit-local-only.sh --deny hound` (a crate that is present, treated as a network crate): exit 1, with "hound is reachable without going through mneme-transcribe" and "used by mneme-transcribe directly, not through ureq". A temporary overlay `src-tauri/tauri.audit-selftest.conf.json` with `https://example.com` and `*` in the CSP and a `plugins.updater` key: exit 1 with all three reported. The file was deleted and the audit passed again. No `Cargo.toml` was edited for this: the crates belong to other lanes.
- `scripts/dev-signing.sh` (no flag): printed the plan, exit 0; no identity of that name exists. Its two `openssl` steps were run by hand in a scratch folder (no keychain involved) to confirm the system LibreSSL accepts the config and produces a certificate with the Code Signing usage; the files were deleted.
- `scripts/release.sh` and `scripts/release.sh --dry-run` without credentials: both exit 1 after listing what is missing (signing identity, notary credentials, entitlements file); the dry run also prints all six steps.
- Credentials scan of the repository (private-key headers, identities, app-specific passwords, key and certificate files): only the placeholders `Your Name (ABCDE12345)` and `Jane Doe (ABCDE12345)` in the doc and script header.
- `prettier --check` on the README, the doc and the two overlay files: clean.

### Known gaps

- `pnpm tauri build --debug` was not run. Other agents were running cargo and editing `src/`. No existing build input changed; the two new overlay files are only read when passed with `--config`. The overlays themselves have therefore never been through a Tauri build; their keys were checked against the installed CLI's config schema (2.12.4).
- `scripts/dev-signing.sh --apply` has never run. Unverified: that `security import` accepts the file, that `codesign` and Tauri sign with an untrusted self-signed identity without extra trust settings (the doc gives the fallback), and that permissions then persist across rebuilds. That is the first manual check.
- `scripts/release.sh` has never run past its prerequisite checks. Unverified: the shape of `notarytool`'s JSON output (`.status`, `.id`), stapling, and the Gatekeeper result.
- The statement that `pnpm tauri dev` is outside dev signing is from how Tauri runs the dev binary, not from a test.
- The audit works on crate names. It cannot see a dependency that opens sockets through the standard library, or a network crate missing from the denylist. It audits the host target only. The Wi-Fi-off test covers run-time behavior and has not been done (no working app yet).
- The release produces a zip of the app, not a DMG.
- The manual test list was gathered while 009, 020, 021, 022, 023 and 024 were `todo` and 008 was `paused`; their checks may change.

### Follow-ups for the orchestrator

- **After 024:** `bundle.macOS.entitlements` must be set in `src-tauri/tauri.conf.json` (or a `tauri.macos.conf.json`). `scripts/release.sh` refuses to run without it. Then decide whether to delete `"hardenedRuntime": false` from `src-tauri/tauri.dev-signing.conf.json` so dev builds match the release.
- Run `pnpm tauri build --debug` once when cargo is free, to tick the last criterion.
- When 021 lands: update the README paragraph that says the backend is not wired, and re-run the audit against the new `src-tauri` dependencies and capabilities.
- When 009 and 020 to 024 are done: re-gather their manual checks into `docs/release.md`.
- From issue 007, before a release: re-check `ureq` 3.5.2 (published the day it was adopted), and decide on limiting redirects to Hugging Face hosts and on proxy or system trust store support. Listed in `docs/release.md`, "Before a release".
- Optional: a `package.json` script or CI step that runs `scripts/audit-local-only.sh`.
