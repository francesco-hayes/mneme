# 032: Choose the microphone in Settings

Status: done · PRD: [05](../prds/05-ui.md), [01](../prds/01-audio-capture.md) · Lane: several (audio, store, app, ui) · Depends on: 031 · Owns: `crates/mneme-audio/src/mic.rs`, `crates/mneme-store/src/settings.rs`, `src-tauri/`, `src/`, the API contract block in `prds/05-ui.md`

## Goal

Let the user pick which input device records their side of a meeting, instead of always the system default. Asked for by Francesco on 2026-11-24.

## Contract changes

In `src/lib/api/types.ts` and the PRD 05 block, which stay identical:

- `interface Microphone { id: string; name: string; isDefault: boolean }`
- `Settings.microphoneId: string | null`: null is the system default.
- `Api.listMicrophones(): Promise<Microphone[]>`: the devices connected now (command `list_microphones`).

## Acceptance criteria

- [x] `mneme-audio` lists input devices (id, name, which is the default) without opening anything or asking for permission, leaving out Mneme's own system audio device.
- [x] `MicSource::with_device(id)` records that device while it is connected and the system default otherwise. It moves to the chosen device when it is plugged in, and back to the default when it is unplugged, through the existing twice-a-second device check.
- [x] The settings file stores `microphoneId`; null or missing is the system default. Any text is accepted: a device that is not connected is not an error.
- [x] Each meeting's sources are made with the microphone in the settings at its start.
- [x] Settings has a Microphone section: a menu with "System default (name)" first, then each device by name. It saves on change. The list is read again when the menu is about to open, so a device plugged in meanwhile appears. A chosen device that is not connected stays chosen, shown as "Unplugged microphone", with a note that meetings record the system default until it is back. A list that cannot be read says so, with Retry.
- [x] `cargo test --workspace`, clippy, fmt, `pnpm test`, `pnpm lint`, `pnpm build` and the local-only audit pass.

## Manual checks

- [ ] In Settings, the menu lists the Mac's real microphones by name, with the current default named in the first entry.
- [ ] Choose a USB or headset mic, start a meeting, speak: Me lines come from that mic (check `mic.wav` in the meeting folder if kept).
- [ ] Unplug the chosen mic during a meeting: recording carries on with the default. Plug it back in: it moves back.
- [ ] Plug a mic in while Settings is open, then open the menu: it is there.

## Result

**Built**

- Audio (`crates/mneme-audio/src/mic.rs`): `InputDevice`, `input_devices()`, `MicSource::with_device`. The cpal backend picks the device to use (the chosen one if connected, else the default) each time it checks for a change, so the existing reopen-on-change logic handles plugging in and out. A small pure function decides the choice and is unit-tested. `system::OBJECT_NAME` is shared so the private aggregate device is never listed.
- Store: `microphoneId`, stored as text like the shortcut (trimmed, empty means not set).
- Backend: `Host::sources` takes the chosen microphone; `Host::microphones` lists devices; `AppCore::list_microphones` and the `list_microphones` command. `dto::Microphone`.
- UI: `src/screens/settings/MicrophoneSection.tsx` (a native `<select>` styled like the model picker's button, with the up-down chevron). Mock: `MOCK_MICROPHONES`, the `microphones` option, `mock.setMicrophones(list)`.

**Checks**

- Rust: 831 tests pass. New: the device-choice rule, a listing of the real devices on this Mac (five, built-in marked default; Mneme's own device absent), the store key, and a core test that each meeting is made with the microphone in the settings at its start.
- UI: 400 tests pass. New Settings tests: names and order, saving an id and null, an unplugged choice with its note, a device plugged in while open appearing on the next open, and a failed list with Retry.
- Browser, on the mock: the section between Transcription and Notes in dark mode, matching the model menu; the unplugged state with its note. No console errors.

**Known gaps**

- Not tried in the real app: whether recording actually switches to the chosen device has been checked only through the backend choosing it (`current_device` returns the chosen id); the manual checks are open.
- A change during a meeting applies from the next meeting, not the current one.
- An unplugged choice is shown as "Unplugged microphone", not by its name: only the id is stored.
- Choosing which device to check is a Core Audio device listing twice a second while a chosen device is set (once for the default before). Not measured, expected to be negligible.
- Virtual devices (for example "Microsoft Teams Audio") are listed like any other input.
