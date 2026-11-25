# Mneme: local meeting notes

Oct 27, 2026 · Francesco Hayes

## Overview

A desktop app that listens to a meeting, transcribes it with a local model, and turns my own rough notes plus the transcript into clean meeting notes. Nothing leaves the machine. Built for personal use first, structured so it can be open-sourced later.

Guiding principles:

- **Local only.** No accounts, no network calls at runtime except optional model downloads.
- **Small surface.** Five features done well beats fifteen done halfway.
- **Notes over audio.** The product is the notes. Raw audio is a temporary input, not an asset.
- **Open-source ready.** Permissive license, plain data on disk, no hard dependency on one vendor's model.

Design and UI decisions have their own section at the end of this file: Design and UI

## MVP features

The five features as originally scoped, each with a definition of done.

| # | Feature | Done when |
| --- | --- | --- |
| 1 | Runs locally | App works with Wi-Fi off once models are downloaded. No telemetry. |
| 2 | Background transcription | One click starts capture; audio is transcribed live in the background and hidden while I take notes; stop ends it and the full transcript is ready right away. |
| 3 | Side-by-side manual notes | I can type notes in an editor while the transcript runs, and both are saved to the same meeting. |
| 4 | Recent notes list | A home screen lists meetings newest first with title, date and duration; clicking one opens it. |
| 5 | Default transcription model | Settings lets me pick a model; new meetings use it; the choice persists across restarts. |

## Gaps worth adding to the MVP

The biggest gap is the "enhance" step: merging rough notes with the transcript into clean notes. It is planned for v2 with bundled llama.cpp, so the MVP can get the foundation right first. The rest are small items that make the base solid.

| Gap | Why it matters | Recommendation |
| --- | --- | --- |
| Enhanced notes (local LLM) | Turns my bullets + transcript into structured notes, decisions and action items. | v2 (bundled llama.cpp) |
| System audio + mic capture | Mic alone only hears me. The other side of a Zoom or Meet call comes through system audio, which needs OS-specific capture. | Add to MVP |
| Me vs. them labels | Recording mic and system audio as separate channels gives a cheap two-speaker split without diarization. | Add to MVP |
| Audio retention rule | "Store nothing" conflicts with keeping a transcript. Clarify: keep notes and transcript, delete raw audio after transcription. | Add to MVP |
| Model download manager | First-run step picks and downloads a default model; Settings shows size, progress, checksum check and delete. Recording is blocked until one model is installed. | Add to MVP |
| Incremental saves | Append each transcript line to transcript.md as it arrives, so a crash mid-meeting loses seconds, not the meeting. | Add to MVP |
| Recording indicator | A clear on-screen state for recording vs. idle; consent laws vary by state. | Add to MVP |
| Export to Markdown | Meetings are already .md files on disk, so there is no export step; a Show in Finder action covers it. | Covered by file storage |
| Basic search | Full-text search across titles, notes and transcripts, from a SQLite FTS5 index built from the meeting folders. | Nice to have |
| Meeting title | Auto-title from the enhanced notes, editable. | Nice to have |

## Out of scope for v1

Deferred on purpose to keep the MVP small:

- Enhanced notes from a local LLM (planned for v2, bundled llama.cpp)
- Asking questions during a meeting (live transcription makes this possible later)
- Calendar integration and auto-start on meeting time
- Bots that join calls
- Sharing, teams, accounts or any cloud sync
- Chat with your notes / Q&A across meetings
- Note templates per meeting type
- Full speaker diarization beyond me vs. them
- Mobile or web clients
- Live translation
- Bring-your-own local transcription models (non-Whisper or custom files)

## Proposed architecture and stack

A Tauri desktop app: Rust handles audio and models, React handles the UI, and a folder of Markdown files sits in the middle, one subfolder per meeting. Proposed, not yet decided.

[Diagram in the doc only: proposed architecture · capture, transcribe, store, enhance]

The transcriber writes chunks as they arrive. In v2, the enhancer reads my notes, the note timeline and the transcript, then writes enhanced notes back. Model downloads are the only network call.

| Layer | Proposed choice | Why |
| --- | --- | --- |
| App shell | Tauri (Rust) + React + TypeScript | Small binary, native audio access, matches my stack |
| Notes editor | CodeMirror 6, notes stored as Markdown | Same base as Obsidian; editor stays swappable |
| Mic capture | cpal (Rust) | Cross-platform audio input |
| System audio | ScreenCaptureKit on macOS; WASAPI loopback on Windows | OS-specific; the hardest part of v1 |
| Transcription | whisper.cpp via Rust bindings | Runs well on CPU and Apple Silicon; many model sizes |
| Enhancement | llama.cpp, bundled, in v2 | Same pattern as whisper.cpp; deferred until the MVP is solid |
| Storage | Markdown files, one folder per meeting; SQLite only as a rebuildable index later (FTS5 search, vectors for Q&A) | Plain files the user owns; open in any editor or Obsidian; back up and sync with the folder; less to build |

**Shipping whisper.cpp.** Link it into the Rust side through the whisper-rs crate with its Metal feature on, so it ships inside the app binary. Model files stay outside the binary and live in the app's data folder, fetched from Settings. macOS needs a microphone usage string and screen/system audio permission, plus a Developer ID signature and notarization for others to open the app.

**Storing meetings.** Each meeting is a folder of plain Markdown files, and the folders are the source of truth. SQLite comes later only as an index that can be deleted and rebuilt from them.

```
Mneme/
  2026-10-27 0930 Standup/
    notes.md        ← frontmatter: id, title, date, duration
    transcript.md   ← [00:01:23] Them: ...
    notes-timeline.jsonl  ← {"t":"00:04:12","text":"- ask Ana about the API"}
  2026-10-27 1430 Meeting/      ← default title kept
```

- **Folder names** are `YYYY-MM-DD HHMM Title`, using the start time. A meeting that keeps its default title is `Meeting`; the title shown in the app ("Meeting, Oct 27 2:30 PM") comes from the frontmatter. The folder is renamed when the title changes, but never while recording. A second meeting started in the same minute gets a numeric suffix (`2026-10-27 1430 Meeting 2`).
- **Stable ID** in the frontmatter, not the folder name, so a rename never breaks links or the index.
- **Strict transcript lines:** `[hh:mm:ss] Me|Them: text`, appended as they arrive. Search, citations and jump-to-line parse this format.
- **Home list** scans the meeting folders and reads their frontmatter; fine for hundreds of meetings.
- **Safe writes:** notes save through a temp file and rename, so a crash never leaves a half-written file.
- **Note timeline:** while recording, each notes autosave appends one entry per changed line to `notes-timeline.jsonl`, with the line's text and the meeting time on the same clock as the transcript. It is a record of what was typed when, not a pointer into `notes.md`, so later edits cannot break it. Nothing reads it in v1; the v2 enhancer uses it to match each note to the conversation around it.
- **Later index** for search and cross-meeting Q&A: SQLite with FTS5 and vectors, plus a content hash per file so only changed files re-index, including edits made outside Mneme.
- **Richer segment data** (word timings, confidence) goes in a `transcript.jsonl` sidecar only if it is ever needed.

## Build order

The riskiest parts go first, so a dead end shows up before any UI is built.

1. Audio capture. A bare Tauri app records mic and system audio to two WAV files. Done when a real Zoom call gives two clean files.
2. Transcription pipeline. Run whisper.cpp on those files in chunks cut at silence. Done when I have speed numbers for each model size on my Mac.
3. Live pipeline. Connect capture to transcription with two channels and Me and Them labels. Done when a 30 minute call gives a full transcript at stop.
4. Storage. Meeting folders with notes.md, transcript.md and notes-timeline.jsonl, appended lines and safe writes. Done when a forced quit mid-meeting loses only seconds.
5. UI. First run with the model download, Home, the live meeting view, the after view and Settings.
6. Finish. Recording indicator, the permissions flow and signing.

## Risks and things to settle

The first three can change the design, so they get settled in steps 1 and 2. The rest are small and have a known fix.

| Risk | Why it matters | Next step |
| --- | --- | --- |
| System audio API | ScreenCaptureKit needs Screen Recording permission and macOS 15 asks for it again over time. Core Audio taps ask for audio only but need macOS 14.2 or later. | Pick one in step 1, along with the minimum macOS version. |
| Speaker bleed | Without headphones the mic hears the other side, so lines repeat and Them gets labeled Me. | Settled 2026-11-24: v1 recommends headphones; no echo cancellation. |
| Whisper is not a streaming model | It needs chunks cut at silence, and it can invent text on silent audio. | Test chunking in step 2 and drop silent chunks. |
| Slow models fall behind | A large model on an older Mac can run slower than real time, so the transcript is not ready at stop. | Measure in step 2 and set a target per model. |
| Two channels finish out of order | Appended lines in transcript.md can land out of time order. | Sort the file by timestamp at stop. |
| Language | Meetings in Spanish or mixed languages need a language setting, and English-only models will not work for them. | Settled 2026-11-12: v1 is English only; no language setting. |
| No stored audio | A bad transcript cannot be redone with a better model, and models cannot be compared. | Add a dev-only flag that keeps audio. |
| Note timing for v2 | Enhanced notes need to know when each note line was typed, and that cannot be added after the fact. | Settled 2026-11-12: changed note lines are appended with the meeting time to notes-timeline.jsonl while recording. |
| Folder name clashes | Two meetings with the same title on one day want the same folder. | Settled 2026-11-12: every folder name includes the start time. |
| Interrupted meetings | A crash leaves a meeting with no end and no duration. | Keep a recording status in the frontmatter and recover on launch. |
| Permissions during development | macOS ties permissions to the code signature, so unsigned builds ask again after each rebuild. | Sign dev builds with one stable certificate. |
| whisper-rs upkeep | Transcription depends on this crate. | Check how actively it is maintained before step 2. |

## Decisions log

Newest first. Add a row whenever something is settled.

| Date | Decision | Rationale |
| --- | --- | --- |
| 2026-11-24 | Notes are enhanced when the user presses Enhance notes; automatic enhancement after Stop is a setting, off by default | Running unasked is annoying, and with a server source it would send notes out without a deliberate action. |
| 2026-11-24 | Enhanced notes can run on a model on the Mac (from Mneme's list or a model file) or on any server the user names, local or remote. Nothing is sent until the user sets one up, and the app always says where notes go | Flexibility that enables privacy: the user decides what they trust, rather than Mneme deciding for them. Narrows the "Local only" principle to the default setup; see PRD 07. |
| 2026-11-24 | Keep the two-hour backlog limit: at the limit a new chunk is dropped and reported, not queued | Protects memory when the transcriber has stalled or is hours behind; only then does it act, and every hole is named in the transcript summary. |
| 2026-11-24 | v1 recommends headphones and does no echo cancellation | Keeps v1 simple; revisit if bleed is a problem in real calls. |
| 2026-11-12 | v1 is English only: no language setting in the UI; transcription runs as English | The app was only ever meant for English; a language picker is not MVP. The engine's language support stays in the code, unused. |
| 2026-11-12 | Note timeline entries stay `{"t","text"}`: each saved state of a line is recorded, with no link to the text it replaced | Keeps the format as designed; the earliest entry for a text says when it was first typed, and v2 matches drafts of a growing line by prefix. |
| 2026-11-12 | A second meeting started in the same minute gets a numeric folder suffix (`2026-10-27 1430 Meeting 2`) | Start, stop, start again within one minute wants the same folder name; a recording must never fail to start. Corrects the assumption in the folder-name row below. |
| 2026-11-12 | Note timing is saved from v1: while recording, each autosave appends changed note lines with the meeting time to `notes-timeline.jsonl` | The time a line was typed cannot be recovered later; v2 enhancement uses it to match notes to the transcript; append-only, so edits never break it. |
| 2026-11-12 | Meeting folders are named `YYYY-MM-DD HHMM Title`; a meeting that keeps the default title uses `Meeting` | Folders sort in true time order; the date is never doubled; two meetings cannot clash because they cannot start in the same minute. |
| 2026-10-27 | Meetings stored as Markdown files, one folder per meeting; SQLite only as a rebuildable index later | Plain data on disk the user owns; no schema, migrations or export step; search and RAG still work from a derived index. |
| 2026-10-27 | Product name is Mneme | Chosen name for the app and the open-source repo. |
| 2026-10-27 | Notes editor is CodeMirror 6; notes stored as plain Markdown | Same base as Obsidian; light and stable; clean text for SQLite, export and the v2 LLM; Live Preview-style rendering can come later. |
| 2026-10-27 | MIT license for the open-source release | Short and simple; matches whisper.cpp and llama.cpp; no patents in play. |
| 2026-10-27 | Live transcription in the background; transcript hidden during the meeting, shown in full after | Transcript ready at stop, crash-safe chunks, audio discarded per chunk; sets up in-meeting Q&A later. |
| 2026-10-27 | Enhanced notes move to v2, using bundled llama.cpp | Get the foundation right first; same in-app, download-what-you-select pattern as Whisper. |
| 2026-10-27 | Whisper models only: users select and download them in Settings; bring-your-own local models deferred | One engine keeps v1 simple; custom models are a later feature. |
| 2026-10-27 | No stored audio: raw audio is deleted once transcribed | The notes are the product; nothing sensitive lingers on disk. |
| 2026-10-27 | Me vs. them labels from separate mic and system audio channels | Speaker split with no diarization model. |
| 2026-10-27 | Models download on demand from Settings; none bundled | Keeps the installer small; users only pull the sizes they pick. |
| 2026-10-27 | macOS first; other platforms later | System audio capture is per-OS; one platform keeps v1 small. |
| 2026-10-27 | Tauri (Rust + React) for the app shell | Small binary, native audio access, direct whisper.cpp bindings. |
| 2026-10-27 | Scope is meeting notes only, not a Granola clone | Keep the MVP small with a strong base. |
| 2026-10-27 | All processing and storage stay on the machine | Privacy; no accounts or servers to run. |
| 2026-10-27 | SQLite for local storage (superseded: Markdown files, see newest row) | Single file, zero setup, good enough for one user. |
| 2026-10-27 | Transcription model is user-selectable | Trade speed vs. accuracy per machine. |
| 2026-10-27 | Personal tool first, open-source later | Ship for one user before designing for many. |

## Open questions

- [x] Platform for v1: macOS first (decided 2026-10-27)
- [x] App shell: Tauri (decided 2026-10-27)
- [ ] Transcription engine: whisper.cpp with downloadable Whisper models only; bring-your-own local models deferred (decided 2026-10-27)
- [ ] Enhancement LLM: bundled llama.cpp, in v2 after the MVP (decided 2026-10-27)
- [ ] Live vs. post-meeting transcription: live in the background, transcript shown after the meeting (decided 2026-10-27)
- [ ] Raw audio: deleted once transcribed, never stored (decided 2026-10-27)
- [ ] Notes editor: CodeMirror 6, notes stored as Markdown (decided 2026-10-27)
- [ ] Model delivery: download on demand from Settings, none bundled (decided 2026-10-27)
- [ ] Apple Developer account for signing and notarization before sharing builds
- [ ] License for the open-source release: MIT (decided 2026-10-27)
- [ ] Product name: Mneme (decided 2026-10-27)
- [ ] Storage: Markdown files, one folder per meeting; SQLite only as a rebuildable index later (decided 2026-10-27)

---

# Design and UI

How Mneme looks and behaves on screen. Settled calls go in the log at the bottom. Everything above it is proposed until it is logged.

## Design principles (proposed)

- **Notes first.** The editor is the largest thing on screen in every meeting view, and everything else stays out of its way.
- **Always clear when it is listening.** Recording state is visible from anywhere in the app.
- **Feels like a Mac app.** System font, native window chrome, light and dark mode that follow the system.
- **Quiet during meetings.** No live transcript, no popups, no badges while recording.

## Screens

| Screen | Purpose | Key elements |
| --- | --- | --- |
| First run | Get one model installed before anything else | Model picker with size and speed notes, download progress, checksum check, mic and system audio permission prompts |
| Home | Find and open past meetings | Meetings newest first with title, date and duration; New meeting button; clicking a meeting opens it full window; search later |
| Meeting, live | Take notes while it records | Back button, title field (date-time default), styled raw Markdown editor, recording bar with timer and stop |
| Meeting, after | Review notes and the full transcript | Notes full width; Transcript button with ready state and line count; transcript in a collapsible side pane with me vs. them styling; export to Markdown |
| Settings | Manage models and defaults | Installed models with size and delete, download more, default model, global shortcut (unset by default) |
| Menu bar icon | Show recording state and start or stop from anywhere | Idle and recording states, timer while recording, Start / Stop / Open Mneme |

## Core flow

1. **Start.** New meeting (button, menu bar icon, or global shortcut if one is set) starts recording right away with a date-time title and puts the cursor in the editor.
2. **During.** I type notes while transcription runs hidden; the recording bar and menu bar icon stay visible.
3. **Stop.** Stop button, menu bar icon or global shortcut ends capture; the notes keep the full window and the Transcript button shows the transcript is ready.
4. **Later.** Home lists the meeting; I open it to read, edit or export.

## Open design questions

- [x] Window layout: Home list that opens each meeting full window, with a back button (decided 2026-11-12)
- [x] Transcript after a meeting: collapsible side pane beside the notes, closed by default (decided 2026-11-12)
- [x] Recording indicator: in-window bar plus a menu bar icon that can start and stop (decided 2026-11-12)
- [x] Global keyboard shortcut: one global start/stop toggle, rebindable in Settings (decided 2026-11-12)
- [x] Editor rendering: styled raw Markdown for v1; Live Preview-style later (decided 2026-11-12)
- [x] Visual identity: system accent and placeholder icon for v1; real identity before the open-source release (decided 2026-11-12)
- [x] Meeting title: date-time default, editable any time; no prompt (decided 2026-11-12)
- [x] Default key combination for the global shortcut: none; off until set in Settings (decided 2026-11-12)
- [x] Transcript pane at stop: stays closed; the Transcript button shows a ready state with the line count (decided 2026-11-12)
- [x] Folder name: `YYYY-MM-DD HHMM Title` for every meeting; default-titled meetings use `Meeting` (decided 2026-11-12)

## Design decisions log

Newest first. Add a row whenever a design call is settled.

| Date | Decision | Rationale |
| --- | --- | --- |
| 2026-11-12 | Global shortcut has no default key combination; it is off until set in Settings | Zero clash risk with other apps; nothing starts a recording by accident. |
| 2026-11-12 | Transcript pane stays closed at stop; the Transcript button shows a ready state with the line count | Notes keep the full window; the button still confirms the transcript exists. |
| 2026-11-12 | Home list opens each meeting full window, with a back button; no sidebar in v1 | Editor gets the whole window; matches MVP feature 4; least to build, and a sidebar can be added later. |
| 2026-11-12 | After a meeting, the transcript lives in a collapsible side pane, closed by default | Notes stay first, but notes can still be checked against the transcript side by side. |
| 2026-11-12 | Recording indicator is an in-window bar plus a menu bar icon with timer and Start / Stop / Open Mneme | The Mneme window is often behind the call; recording state has to be visible from anywhere. |
| 2026-11-12 | One global shortcut toggles recording, rebindable in Settings | Fastest way to start when a call begins; rebinding avoids clashes for other users. |
| 2026-11-12 | Editor shows styled raw Markdown: markers visible but dimmed, headings and bold styled in place | Stock CodeMirror 6 highlighting; the file on disk matches the editor; Live Preview can come later. |
| 2026-11-12 | v1 uses the system accent color and a placeholder icon; real identity before the open-source release | Feels like a Mac app; no design time spent ahead of the audio work. |
| 2026-11-12 | Meetings start with a date-time title, editable during or after; never a title prompt | Nothing blocks the start of a recording; rename is safe because the ID lives in frontmatter. |
| 2026-10-27 | Transcript hidden during the meeting, shown in full after | Keeps attention on my own notes; the transcript is ready the moment I stop. |
| 2026-10-27 | Me vs. them shown as two speaker styles in the transcript | Comes free from separate mic and system audio channels. |
| 2026-10-27 | Recording disabled until a Whisper model is installed | Nothing to transcribe with; first run walks through the download. |
| 2026-10-27 | Notes edited as plain Markdown in CodeMirror 6 | Clean text on disk; Live Preview-style rendering can come later. |
