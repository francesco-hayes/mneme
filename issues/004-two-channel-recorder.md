# 004: Two-channel recorder and `mneme-record` CLI

Status: todo · PRD: [01](../prds/01-audio-capture.md) · Lane: audio · Depends on: 003 · Owns: `crates/mneme-audio/`

## Goal

Run the mic and system sources together on one clock, and prove build step 1 with a command-line recorder that writes two WAV files.

## Scope

1. **`Recorder`**: takes two `AudioSource`s, starts both with one shared start instant, exposes one stream of `AudioFrame`s (both speakers), stops both. If one source fails to start or dies, the other keeps going and the failure is reported.
2. **`mneme-record` binary**: `--out <dir>` and optional `--seconds N`. Records until Ctrl-C or the limit, writes `mic.wav` and `system.wav`, prints duration and any source errors. Each file is padded with silence where its source had no frames, so both files have the same length and line up in time.
3. Usage notes in the crate README: how to run it, which permissions it asks for, and how to test during a call.

## Acceptance criteria

- [ ] Test with two fake sources: both files written, same length, offsets preserved (a source that starts late is padded at the front).
- [ ] Test: one fake source fails mid-run; the other file is complete and the error is reported.
- [ ] `cargo run -p mneme-audio --bin mneme-record -- --help` works.
- [ ] `cargo test -p mneme-audio` and clippy pass.

## Manual checks

- [ ] During a real Zoom call with headphones: `mic.wav` has only me, `system.wav` has only the other side, both clean.
- [ ] The two files line up when played together.

## Result

_Filled in by the implementer._
