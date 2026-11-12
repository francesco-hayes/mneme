# PRD 06: Integration and finish

Covers build step 6, the gap "Recording indicator", and the wiring that joins the backend crates to the UI. Issues: 021–025.

## Problem

The crates and the UI exist separately. The app needs them connected, a recording state visible from anywhere on the Mac, a permissions flow that works on a fresh machine, and a build other people can open.

## Goals

- Tauri commands and events that implement the UI's API contract with the real crates.
- A menu bar icon that shows recording state and can start and stop.
- One global shortcut that toggles recording, off until set.
- A clear path through microphone and system audio permissions.
- Signed dev builds so permissions stick; a notarized release build once a Developer ID exists.

## Non-goals

- Auto-update.
- Calendar-triggered start.
- Windows or Linux packaging.

## Requirements

### Commands and state

- One app state owns the current `Session` (PRD 03), the store root and the settings.
- Every method in the API contract (PRD 05) has a command; `onRecordingState` and `onDownloadProgress` are events.
- Only one recording at a time. Starting while recording returns the current meeting's error, not a second session.
- `startMeeting` fails with a clear message when no model is installed or a permission is denied.
- On launch: run crash recovery (PRD 04) before the first list is served.
- Quitting while recording stops the session cleanly first.
- The UI switches from the mock to the Tauri implementation when running inside Tauri.

### Menu bar icon

- Idle and recording states with distinct icons (template images, so they follow the menu bar's appearance).
- While recording, the timer shows next to the icon.
- Menu: Start or Stop (whichever applies), Open Mneme, Quit.
- Starting from the menu bar opens no window; it just records.

### Global shortcut

- One system-wide shortcut toggles recording. No default; nothing is registered until the user sets one in Settings.
- Changing it re-registers at once. A combination that cannot be registered reports an error and the old one stays.

### Permissions

- `Info.plist` usage strings for the microphone and for system audio capture; entitlements for the hardened runtime.
- `getPermissions` reports real status. `requestPermission` triggers the system prompt, or opens the right pane of System Settings when the user has already denied.
- A denied permission during recording shows what is missing and how to fix it; mic-only recording still works if system audio is denied.

### Signing and release

- Dev builds are signed with one stable local certificate so macOS does not re-ask for permissions after each rebuild. A script sets it up; the steps are documented.
- Release: Developer ID signing and notarization are scripted and documented, and run once the Apple Developer account exists (open question in the scope doc).
- Local-only audit: a check that the shipped app has no network code paths other than the model downloader, and a test run with Wi-Fi off.

## Done when

- Tests cover the command layer with fake sessions: start, stop, double start, start without a model, recovery on launch.
- Manual: core flow works in the built app; menu bar icon and timer track recording; shortcut toggles from another app; permissions flow works on a fresh user account; app works with Wi-Fi off once a model is downloaded.
