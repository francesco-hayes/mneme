# 030: Keep the model loaded so New meeting is fast

Status: done · PRD: [06](../prds/06-integration-and-finish.md) · Lane: app · Depends on: 028 · Owns: `src-tauri/src/app.rs` (start path, models, settings, shutdown; not the `stop` body), `src-tauri/src/app/tests.rs`, `src-tauri/src/host.rs`, `src-tauri/src/lib.rs` (warm-up at launch only)

## Goal

Every meeting loaded its Whisper model from scratch in `start_session`: seconds, plus about 7 s on the first load while Metal compiles its shaders. The engine was dropped at stop. Keep one engine loaded instead, warmed after launch, so New meeting starts at once.

## Acceptance criteria

- [x] After launch, off the main thread, the model a start would pick (the default if installed, else the first installed) is loaded and kept.
- [x] `start_meeting` uses the warm engine when it is the picked model, and otherwise loads as before. The engine is kept after stop for the next meeting.
- [x] It is replaced when it can no longer be right: the default model changes, the model is deleted (deleting a warm model that is not recording works and frees it first), or a download finishes and may change the pick. The new pick is warmed in the background.
- [x] A start that arrives while a load is under way waits for it; the model is loaded once. The `recording` lock is never held while loading, and the lock order is `recording` then `warm`.
- [x] `shutdown` waits for a load under way, which drops its engine because quitting is set, then frees the warm engine before returning. No thread but the core holds the engine at exit (issue 005).
- [x] One cached engine, no LRU. The memory cost (0.1 to 1.6 GB by model) is noted in the code.
- [x] `cargo test -p mneme`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all` pass.

## Manual checks

- [ ] Launch the app, wait a few seconds, press New meeting: it records at once (no multi-second wait, none of the first-load shader delay).
- [ ] Change the default model in Settings, wait, start a meeting: no wait, and the meeting uses the new model.
- [ ] Delete the default model in Settings while idle: it is deleted, and the next meeting starts with the other installed model.
- [ ] Quit Mneme from the menu bar right after launch (while the warm-up loads) and after a meeting: the process exits normally, with no abort report.
- [ ] Activity Monitor: memory stays at the size of one model after several meetings.

## Result

**Built**

- `app.rs`: a `Warm` cache (`engine: Option<(model id, Arc<dyn Transcriber>)>`, `loading`, `discard`) behind its own mutex and condvar. `warm_engine` returns the cached engine for a model, waits for a load under way, or loads it itself (freeing a different cached model first, so only one is in memory) and keeps it. `start_session` gets its engine from there; `Recording::Active` still holds a clone, so the `stop` body is unchanged and the cache keeps the engine after it.
- `AppCore::warm_up`: on a thread of its own, waits for the recording to be idle (so a load does not compete with a meeting), picks the model and warms it. Called after launch (`lib.rs`), after `update_settings` changes the default, after `delete_model`, and when a download ends `done` (through a `Weak` to the core).
- `delete_model`: under the `recording` and `warm` locks, frees the warm engine of that model, marks a load of it under way to be discarded, then deletes the file. `warm_engine` checks the model is installed under the same lock, so a warm-up cannot begin loading a model being deleted.
- `shutdown`: after the recording loop, waits for `loading` to clear and drops the cached engine. A load that ends after quitting began drops its engine before it clears `loading`, and the warm-up never takes a reference of its own, so nothing outlives `shutdown`. New loads are refused once quitting.

**Checks**

- Six new tests in `app/tests.rs`: two meetings after a warm-up load the model once; a new default replaces the warm engine (the old one is freed); deleting the warm model frees it before `delete_model` returns and warms the other one; a start during a gated warm-up waits (state readable, a second start refused) and loads once; shutdown frees the warm engine and later warm-ups do nothing; shutdown during a gated warm-up waits for it and nothing is left alive.
- Two existing tests changed: after stop the engine is now kept warm instead of gone.
- `cargo test -p mneme` (65 tests), clippy and fmt pass.

**Known gaps**

- None of it has been seen in the real app; the manual checks are open. Whether the Metal shader delay is paid once per process (so warm-up hides it) is assumed, not measured.
- The download path (`done` triggers a warm-up) has no test: downloads need the network.
- A warm-up asked for during a meeting runs after it ends, so a default changed mid-meeting is loaded then, not straight away.
- The comment in the `stop` body ("this frees the model") is now out of date: the warm cache keeps the model. Left for whoever owns that body.
- If `stop` gains a timeout and returns while the session still holds the engine, `shutdown` freeing the cache would not free that last reference.
