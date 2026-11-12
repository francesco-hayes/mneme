# 011: Transcript append, parse, sort

Status: todo · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 010 · Owns: `crates/mneme-store/`

## Goal

Write the transcript one line at a time so a crash loses seconds, and read it back reliably.

## Scope

1. **Line format**: `[hh:mm:ss] Me: text` / `[hh:mm:ss] Them: text`, using `mneme-core`'s timestamp functions. Newlines and runs of whitespace inside text become single spaces; empty text is not written.
2. **`TranscriptWriter`** for a meeting: `append(segment)` writes one line and flushes to disk before returning. Safe to call from the pipeline's worker thread.
3. **`finalize()`**: sorts the file by timestamp, stable for equal timestamps, through the safe write helper. Malformed lines are kept at the end rather than dropped.
4. **Parser**: returns lines in file order as `(timestamp, speaker, text)`; skips malformed lines and reports how many were skipped. A cheap line count for the UI.
5. A half-written last line (crash mid-append) is handled: the parser skips it; the next append starts on a new line.

## Acceptance criteria

- [ ] Tests: format and parse round-trip, including text containing brackets and colons; out-of-order appends are sorted by `finalize`; equal timestamps keep arrival order; multi-line text is flattened; a truncated last line is skipped and the next append is well-formed; line count matches.
- [ ] Test: lines appended before a simulated crash (writer dropped without finalize) are all on disk.
- [ ] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
