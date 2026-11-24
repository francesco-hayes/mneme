# 029: Stop that cannot hang, failed chunks retried and marked, disk check before downloads

Status: done · PRD: [03](../prds/03-live-pipeline.md), [02](../prds/02-transcription-and-models.md) · Lane: pipeline, transcribe, app (stop path) · Depends on: 009, 007, 021 · Owns: `crates/mneme-pipeline/`, `crates/mneme-transcribe/src/models/`, the stop path in `src-tauri/src/app.rs`

## Goal

Make a meeting impossible to lose or get stuck. Three gaps from [known-gaps.md](../docs/known-gaps.md):

1. A transcriber call that never returns makes Stop wait forever.
2. A chunk the transcriber fails on is dropped without a trace in the transcript.
3. A model download starts without checking that it fits on the disk.

## Acceptance criteria

- [x] Stop gives up on a transcriber call that runs for 120 s (`CALL_LIMIT` in `app.rs`). It keeps every line written so far, lists the chunk in work (`DropReason::TranscriberStuck`) and the audio still waiting (`DrainAbandoned`), finalizes the transcript, closes the meeting as `complete`, drops the app's reference to the engine and logs what was given up. The same limit applies at quit, after the 10 s `QUIT_GRACE`.
- [x] The limit applies to one call, not to the whole drain. A long backlog that the transcriber is working through is still transcribed to the end.
- [x] A chunk that fails (error or panic) is tried once more. If the second try fails, a line `[hh:mm:ss] Me: [not transcribed]` (or `Them:`) is written at the chunk's start time, and the failure is still in `SessionSummary.transcription_errors`. The line round-trips through the store's parser and sorts by time like any other line.
- [x] A download is refused before anything starts when the models folder's disk has less free space than the model plus 200 MB. The refusal is one sentence naming the model, its size, the margin and the free space, and reaches the UI as the `download_model` command's `Err(String)`, like the other start errors.
- [x] `cargo test` for `mneme-pipeline`, `mneme-transcribe` and `mneme`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all` and `scripts/audit-local-only.sh` pass.

## Manual checks

- [ ] Real app: record a short meeting and stop. It stops as before (no 120 s wait, nothing given up).
- [ ] A transcript with a `[not transcribed]` line opens in the meeting view and reads sensibly there.
- [ ] Download a model onto a nearly full disk (a small disk image works): the Settings row shows the refusal sentence.

## Result

**Built**

- Pipeline: `Session::give_up_transcriber()` and `Session::stop_or_give_up(call_limit)`. The capture thread no longer blocks on joining the transcription thread. It waits until that thread ends or the session gives up on it. On a give-up it takes the sink out of a shared slot, finalizes it, lists the chunk in work as `TranscriberStuck` and publishes the summary. The transcription thread holds the slot's lock while it writes and accounts for a chunk, so a give-up never lands halfway through a chunk. A call that returns after a give-up writes nothing. The worker records when each call starts, and that is what `stop_or_give_up` watches.
- Retry and gap marker in `worker.rs`. There is no retry once the session is giving up or the backlog has been abandoned. `mneme_pipeline::NOT_TRANSCRIBED` is the marker text. The Whisper engine strips short bracketed tags from its own output (`text.rs`), so real speech can never come out as exactly this text. The gap line counts towards `lines`. The store needed no change.
- App (`app.rs`, stop path only): both normal and hurried stops end in `stop_or_give_up(CALL_LIMIT)`. `report_stuck` logs the speaker and time of the stuck chunk, the milliseconds given up and the lines kept. Why 120 s: a chunk holds at most 28 s of audio (`ChunkerConfig::default`). A call still running after 120 s is more than 4x slower than real time, and a model that slow could not keep up with a meeting anyway. No benchmark figures are involved.
- Models: `ModelError::NotEnoughSpace` and `DOWNLOAD_SPACE_MARGIN` (200 MB). Free space is `statvfs` `f_bavail * f_frsize`, read through `rustix::fs::statvfs` (rustix 1.1.6 was already in the lock file through `tempfile`, and only its `fs` feature is used). If the free space cannot be read, the download goes ahead.

**What happens to a stuck thread**

Nothing can interrupt a call into whisper.cpp. After a give-up, the `mneme-transcribe` thread stays inside the call and keeps its `Arc` to the engine. The session and the app have dropped theirs, so the model stays loaded only through that thread. If the call ever returns, the thread writes nothing, drops the engine (which frees the model) and ends. If the app quits first, the engine is still alive at exit. Issue 005 says whisper.cpp's Metal backend then fails its assertion and aborts the process. By that point the meeting is closed and its transcript sorted, so the cost is a crash report, not lost data. A new meeting loads its own engine as usual, so while the old thread is stuck two models can be in memory.

**Checks**

- `mneme-pipeline`: all tests pass. New: `a_chunk_that_fails_once_is_tried_again_and_kept`, `the_gap_line_round_trips_through_the_store_in_time_order` (real `Store` and `TranscriptWriter`, exact file contents), `a_transcriber_call_that_never_returns_is_given_up_and_the_session_ends` (gated call, 300 ms limit, the sink is finalized once and nothing is written after the call is released, the engine `Arc` drops back to one), `a_slow_drain_is_not_cut_short_by_the_call_limit`. Changed for the new rules: the error-on-one-chunk, fails-every-time and panic tests, which now expect two tries and a gap line.
- `mneme`: all 60 pass. New: `stop_gives_up_on_a_transcriber_call_that_never_returns_and_closes_the_meeting` (in tests `CALL_LIMIT` is 300 ms, via `cfg(test)`).
- `mneme-transcribe`: all pass. New: `a_model_larger_than_the_free_space_is_refused_before_anything_starts` (real `statvfs` on the temp folder: no events, no file, nothing active), plus the wording and size formatting of the refusal.
- Clippy, fmt and the local-only audit are clean.

**Known gaps**

- The UI shows `[not transcribed]` as plain text. It is not styled differently, and the reason for the failure is in the log only.
- The space check does not count bytes that another download running at the same time still has to write. On APFS, `f_bavail` leaves out purgeable space, so the check can refuse a download when Finder shows enough space available.
- A sink stuck inside `append` or `finalize` (for example a disk that hangs) still holds Stop up. The give-up waits for the sink lock. Only a stuck transcriber is covered.
- Nothing here has been run in the real app. The manual checks above are open.
