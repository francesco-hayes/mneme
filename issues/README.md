# Issues

Local issue tracker. One file per issue; the status line at the top of each file is the source of truth, and this table mirrors it.

Statuses: `todo`, `in progress`, `done`, `blocked`. An issue is `done` when its acceptance criteria pass. Manual checks that need a real call, a permission prompt or a signing account are listed in each issue and tracked separately in the last column.

| # | Issue | PRD | Lane | Depends on | Status | Manual checks |
| --- | --- | --- | --- | --- | --- | --- |
| [001](001-scaffold-workspace.md) | Scaffold workspace and app shell | 00 | base | — | todo | — |
| [002](002-mic-capture.md) | Mic capture to 16 kHz mono, WAV writer | 01 | audio | 001 | todo | open |
| [003](003-system-audio-taps.md) | System audio capture with Core Audio taps | 01 | audio | 002 | todo | open |
| [004](004-two-channel-recorder.md) | Two-channel recorder and `mneme-record` CLI | 01 | audio | 003 | todo | open |
| [005](005-whisper-engine.md) | Whisper engine wrapper | 02 | transcribe | 001 | todo | — |
| [006](006-silence-chunker.md) | Silence chunker | 02 | transcribe | 005 | todo | — |
| [007](007-model-catalog-downloads.md) | Model catalog and download manager | 02 | transcribe | 006 | todo | — |
| [008](008-benchmark-cli.md) | Benchmark CLI and speed table | 02 | transcribe | 007 | todo | open |
| [009](009-live-session.md) | Live session: capture to transcript | 03 | pipeline | 004, 006, 011 | todo | open |
| [010](010-meeting-folders.md) | Meeting folders, frontmatter, naming | 04 | store | 001 | todo | — |
| [011](011-transcript-file.md) | Transcript append, parse, sort | 04 | store | 010 | todo | — |
| [012](012-notes-and-timeline.md) | Notes safe writes and note timeline | 04 | store | 011 | todo | — |
| [013](013-listing-and-recovery.md) | Meeting list and crash recovery | 04 | store | 012 | todo | — |
| [014](014-settings-file.md) | Settings file | 04 | store | 013 | todo | — |
| [015](015-ui-shell.md) | UI shell: routing, theme, recording bar | 05 | ui | 001 | todo | — |
| [016](016-home-screen.md) | Home screen | 05 | ui | 015 | todo | — |
| [017](017-meeting-live-view.md) | Meeting live view and editor | 05 | ui | 016 | todo | — |
| [018](018-meeting-after-view.md) | Meeting after view and transcript pane | 05 | ui | 017 | todo | — |
| [019](019-settings-screen.md) | Settings screen | 05 | ui | 018 | todo | — |
| [020](020-first-run.md) | First run | 05 | ui | 019 | todo | — |
| [021](021-tauri-commands.md) | Tauri commands and events | 06 | app | 009, 007, 014, 020 | todo | open |
| [022](022-menu-bar-icon.md) | Menu bar icon | 06 | app | 021 | todo | open |
| [023](023-global-shortcut.md) | Global shortcut | 06 | app | 022 | todo | open |
| [024](024-permissions-flow.md) | Permissions flow | 06 | app | 023 | todo | open |
| [025](025-signing-and-release.md) | Signing, notarization, local-only audit | 06 | app | 024 | todo | open |

## Lanes

Issues in one lane run one after another because they share files. Different lanes run at the same time because they own different folders.

| Lane | Owns | Order |
| --- | --- | --- |
| base | whole repo, once | 001 |
| audio | `crates/mneme-audio/` | 002 → 003 → 004 |
| transcribe | `crates/mneme-transcribe/` | 005 → 006 → 007 → 008 |
| store | `crates/mneme-store/` | 010 → 011 → 012 → 013 → 014 |
| ui | `src/`, frontend config | 015 → 016 → 017 → 018 → 019 → 020 |
| pipeline | `crates/mneme-pipeline/` | 009 |
| app | `src-tauri/`, `src/lib/api.tauri.ts` | 021 → 022 → 023 → 024 → 025 |

## Rules for whoever implements an issue

- Stay inside the paths the issue owns. If another area needs a change, write it under "Follow-ups" in the issue instead of making it. Additive changes to `crates/mneme-core` are allowed when the issue says so.
- Build and test only your own crate (`cargo test -p <crate>`) or the frontend; other lanes may be mid-change.
- Check current documentation for crates and APIs before using them; do not rely on memory for versions or signatures.
- Never invent data that must be real: checksums, file sizes, benchmark numbers, API behavior. Look it up or measure it, or leave it marked as unknown.
- Tests that need a microphone, a permission prompt, a model file or the network are marked ignored or skip cleanly, and are listed under "Manual checks".
- Do not commit. Finish by filling in the "Result" section of the issue: what was built, what was verified and how, what was not verified, and any follow-ups.
