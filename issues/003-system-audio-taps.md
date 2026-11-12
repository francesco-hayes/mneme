# 003: System audio capture with Core Audio taps

Status: todo · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 002 · Owns: `crates/mneme-audio/`

## Goal

Capture what the Mac is playing (the other side of a call) as `Speaker::Them` frames, using Core Audio process taps.

## Scope

1. **`SystemAudioSource`** implementing the `AudioSource` trait from issue 002. A global tap over all processes, excluding Mneme's own process, wrapped in an aggregate device and read with an IO proc. Tap is unmuted: the user still hears the call.
2. Read the tap's stream format and run it through the conversion module from issue 002 (downmix, resample to 16 kHz).
3. Clean teardown: destroying the tap and aggregate device on stop and on drop, including after errors. No orphan aggregate devices left in the system.
4. Follow output device changes (headphones in or out, AirPods) without ending capture and without a jump in the meeting clock.
5. **Permission helpers** for system audio capture: status and request, as far as the OS allows checking. Document what the OS actually exposes.
6. The API choice is behind the trait. Note in the module docs what a ScreenCaptureKit source would need to implement.

## Notes

- Minimum macOS is 14.4. Check current Apple documentation and the current state of Rust bindings before choosing how to call the API (existing crate versus direct bindings); record the choice and why in the Result.
- The binary that captures needs `NSAudioCaptureUsageDescription`. For a command-line binary, document how that is supplied for manual testing.
- This is the highest-risk issue in v1. If taps cannot be made to work, stop and write up exactly what failed, rather than switching API without saying so.

## Acceptance criteria

- [ ] `SystemAudioSource` compiles and implements the trait; teardown is covered by a test that needs no permission where possible.
- [ ] Unit tests for format handling using synthetic buffers in the formats taps deliver.
- [ ] A live capture test exists, marked ignored.
- [ ] `cargo test -p mneme-audio` and clippy pass.

## Manual checks

- [ ] Play audio from another app; the ignored live test or example records it cleanly.
- [ ] Mneme's own sounds are not captured.
- [ ] Switch output device mid-capture; capture continues.
- [ ] After the process exits, Audio MIDI Setup shows no leftover aggregate device.

## Result

_Filled in by the implementer._
