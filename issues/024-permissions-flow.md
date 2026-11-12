# 024: Permissions flow

Status: todo · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 023 · Owns: `src-tauri/`

## Goal

Microphone and system audio permissions that work on a fresh Mac and explain themselves when denied.

## Scope

1. **Bundle metadata**: `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription` with plain, honest strings (audio is transcribed on this Mac and then discarded). Entitlements for the hardened runtime that audio input needs. No entitlements beyond what is used.
2. **`getPermissions`** returns real status using the helpers from the audio crate. Where the OS gives no way to query system audio status without prompting, document the behavior and return `undetermined` until a capture attempt settles it.
3. **`requestPermission`**: triggers the system prompt when undetermined. When already denied, opens the matching pane of System Settings and returns `denied`.
4. Status is re-read when the app regains focus, so granting in System Settings is picked up without a restart where the OS allows. If a relaunch is required for a permission to take effect, say so in the UI message returned.
5. **During start**: mic denied blocks recording with a message naming the fix. System audio denied records mic only and reports it once.
6. **Mid-recording revocation**: the affected channel stops, the other continues, and the state reports it.

## Acceptance criteria

- [ ] Tests for the decision logic with faked permission states: start allowed, mic-only, or refused; messages name the right System Settings pane.
- [ ] The built app's `Info.plist` contains both usage strings; entitlements file is present and referenced by the bundle config.
- [ ] `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] On a fresh user account (or after resetting permissions with `tccutil reset` for the bundle id): first run prompts for each permission once, with the right text.
- [ ] Deny the mic, then follow the app's instructions to fix it.
- [ ] Deny system audio: a meeting records only Me lines and says why.

## Result

_Filled in by the implementer._
