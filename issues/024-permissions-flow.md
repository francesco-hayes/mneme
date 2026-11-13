# 024: Permissions flow

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 023 · Owns: `src-tauri/`

## Goal

Microphone and system audio permissions that work on a fresh Mac and explain themselves when denied.

## Scope

1. **Bundle metadata**: `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription` with plain, honest strings (audio is transcribed on this Mac and then discarded). Entitlements for the hardened runtime that audio input needs. No entitlements beyond what is used.
2. **`getPermissions`** returns real status using the helpers from the audio crate. Where the OS gives no way to query system audio status without prompting, document the behavior and return `undetermined` until a capture attempt settles it.
3. **`requestPermission`**: triggers the system prompt when undetermined. When already denied, opens the matching pane of System Settings and returns `denied`.
4. Status is re-read when the app regains focus, so granting in System Settings is picked up without a restart where the OS allows. If a relaunch is required for a permission to take effect, say so in the UI message returned.
5. **During start**: mic denied blocks recording with a message naming the fix. System audio denied records mic only and reports it once.
6. **Mid-recording revocation**: the affected channel stops, the other continues, and the state reports it.

## Notes from earlier issues

- From 003: macOS has no public API to read or request the system audio permission, and without it nothing fails: the tap delivers silence. `system_audio_permission_status()` returns `None` (unknown) by default. A Cargo feature `tcc-spi` in `mneme-audio` (off by default) uses private TCC calls to give a real status and prompt; it rules out the Mac App Store. Do not turn it on without Francesco's decision; build the flow to work with "unknown".
- From 003: `SystemAudioSource::start` returns `PermissionDenied` when its access check is refused; that is the signal to send the user to Settings. The pane is "Screen & System Audio Recording", section "System Audio Recording Only". A change there may need an app restart. `start` and `request_system_audio_permission()` can block while the prompt is open: call them off the UI thread.
- From 003: Chromium's source warns that a prompt left unanswered for 60 s breaks Core Audio in that process until restart.
- From 003: none of the tap code had met a real tap when it was written. Read issue 003's Result and its manual-check outcomes first.
- From 002: `request_microphone_permission()` blocks until the user answers.

## Acceptance criteria

- [x] Tests for the decision logic with faked permission states: start allowed, mic-only, or refused; messages name the right System Settings pane.
- [x] The built app's `Info.plist` contains both usage strings; entitlements file is present and referenced by the bundle config.
- [x] `cargo test -p mneme` and clippy pass.

## Manual checks

- [ ] On a fresh user account (or after resetting permissions with `tccutil reset` for the bundle id): first run prompts for each permission once, with the right text.
- [ ] Deny the mic, then follow the app's instructions to fix it.
- [ ] Deny system audio: a meeting records only Me lines and says why.

## Result

Implemented 2026-11-12 on macOS 27.0, Apple Silicon. The app was **not launched**, no permission was requested and System Settings was not opened (the user was away). Everything is proven with faked permission states plus a compiling bundle; no prompt has been seen.

### What was built

- `src-tauri/src/permissions.rs` (new): the decisions as pure functions. `start_decision` (record, microphone only, or refuse with the sentence), `system_audio_evidence` (what a source's phase, errors and frame count say), `system_audio_status` (what `getPermissions` reports), `request_action` (nothing, prompt, or open Settings), `channel_lost`, and `PermissionKind::pane` / `settings_url`. Six unit tests.
- `src-tauri/src/app.rs`: `AppCore` uses them. It remembers, for this run, what the system source of the latest recording showed: refused with `PermissionDenied` reads `denied`, delivered frames reads `granted`, otherwise `undetermined`. The microphone is read from macOS on every call. The memory is updated on every state tick and again when a meeting closes.
- `src-tauri/Entitlements.plist` (new), named by `bundle.macOS.entitlements` in `tauri.conf.json`: only `com.apple.security.device.audio-input`. Apple's page for it: "may record audio using the built-in microphone and access audio input using Core Audio". No sandbox, JIT or library exceptions. `scripts/release.sh` finds it (same `jq` merge as the script: `Entitlements.plist`).
- `src-tauri/src/host.rs`: opens the URL from `PermissionKind::settings_url` with `/usr/bin/open`.

### Decisions

- **Usage strings:** reviewed, left as they were. Both say what is recorded, why, and that the audio is transcribed on this Mac and then discarded.
- **Documents folder:** no usage string added. Apple's page for `NSDocumentsFolderUsageDescription` says a file the app creates in Documents needs no consent, that the system prompts on other access, and that the string is "optional, but highly recommended". Mneme creates `~/Documents/Mneme` itself. Whether macOS still asks when a later launch reads that folder was not tried.
- **System audio, undetermined request:** `requestPermission` triggers the prompt through `request_system_audio_permission` (the public route: a tap opened for a moment). Reason: the First run button would otherwise do nothing, and the prompt would first appear at the start of the first meeting, where it costs the other side's audio while it is open. It runs the same tap code a recording runs, so it adds no new kind of risk. Only a yes is kept; a refusal read there may be a prompt still open, so the status stays `undetermined` until a recording is refused.
- **Known denied at start:** the meeting starts and the system source is still tried. That attempt is the only way to notice access given in System Settings; the next meeting that delivers flips the status to `granted`.
- **Start:** microphone `denied` refuses with "Microphone access is denied. Allow Mneme in System Settings > Privacy & Security > Microphone. If it still cannot record after that, quit and reopen Mneme." `Restricted` gets a sentence about policy and names no pane. System audio `denied` records the microphone only and logs the sentence naming Privacy & Security > Screen & System Audio Recording, under System Audio Recording Only, "then quit and reopen Mneme".
- **During a meeting:** a channel that fails to start or stops is logged once with its reason (and its pane when the reason is the permission). When no source is left the watcher stops the meeting as soon as capture is over, so the window gets `stopping` then `idle` instead of `recording`.
- **Settings URLs:** `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone` and `...?Privacy_AudioCapture`. Both anchor names are strings in this Mac's Privacy & Security settings extension binary; neither URL was opened.
- **Config tests:** the three existing ones pass unchanged. Two were added in `lib.rs` for this change: the entitlements file is referenced and holds only audio input; `Info.plist` has both usage strings.

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme` | 56 passed |
| `cargo clippy -p mneme --all-targets -- -D warnings` | clean |
| `cargo fmt -p mneme -- --check` | clean |
| `pnpm tauri build --debug` | `target/debug/bundle/macos/Mneme.app` built |
| `PlistBuddy` on the built `Info.plist` | both usage strings present, text as in `src-tauri/Info.plist` |
| `jq` on the config | `bundle.macOS.entitlements` is `Entitlements.plist`; the file exists and passes `plutil -lint` |

Tests for the criteria: in `permissions.rs`, `a_start_is_allowed_mic_only_or_refused`, `each_permission_names_and_opens_its_own_pane`, `a_request_prompts_when_undetermined_and_opens_settings_when_denied`, `a_recording_start_settles_system_audio`, `get_permissions_reports_what_the_recordings_learned`, `a_lost_channel_is_named_with_its_reason_and_its_pane`. In `app/tests.rs`, with a fake system source that returns `PermissionDenied`: `a_refused_system_source_records_the_microphone_only_and_reads_denied_from_then_on`, `a_meeting_that_loses_every_source_does_not_stay_recording`, and the extended `permissions_report_what_is_known_and_a_request_agrees_with_the_next_read`.

### Manual checks still open

None of the three under "Manual checks" was done. Also:

- A signed hardened-runtime build: `codesign -d --entitlements - Mneme.app` shows audio input, the microphone records, and the tap delivers with that entitlement alone.
- First run, system audio button: the prompt appears with the usage text; whether the call waits for the answer; what the status reads after Allow and after Don't Allow.
- Both Settings URLs open the right pane, the system audio one at System Audio Recording Only.
- Microphone denied, then allowed in Settings: the First run screen turns to granted on focus without a restart.
- System audio allowed in Settings after a refusal: whether the next meeting records it without a restart.
- Whether macOS asks for the Documents folder, at first launch or a later one.
- Access taken away during a meeting: the other channel carries on.

### Known gaps

- **The debug bundle does not carry the entitlements.** It is signed ad hoc by the linker; Tauri applies the file only when it signs with an identity. That the entitlement is enough under the hardened runtime is unproven.
- **"Microphone only" never reaches the UI.** The contract has no field for it; the sentence goes to stderr. The same holds for a channel lost during a meeting.
- **A meeting whose sources all fail at once** is still returned by `startMeeting` as started; it goes to `stopping` and `idle` within a tick, and the reason is only in the log.
- **`denied` rests on `SystemAudioSource::start`.** If macOS shows the prompt without holding that call, the first meeting marks system audio `denied` while the prompt is still open. A later meeting that delivers corrects it.
- **A refused system-audio prompt at First run stays `undetermined`** until a meeting is refused, so the screen cannot show "Open System Settings" before that.
- **`granted` means frames arrived**, not that they carried sound; it trusts the access check in `start`.
- **The memory lasts one run.** After a restart system audio is `undetermined` again.
- **Neither request function has ever been called**, here or in a test; `host.rs` is untested.
- The "quit and reopen" advice for the microphone is a hedge; whether a restart is needed was not observed.

