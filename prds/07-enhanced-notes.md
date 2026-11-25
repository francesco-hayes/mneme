# PRD 07: Enhanced notes

Covers the scope doc's gap "Enhanced notes (local LLM)", its "Enhancement" architecture row and the nice-to-have "Meeting title". This is the v2 feature the note timeline was saved for. Issues: 037–044 (proposed below, not yet written).

Status: draft, 2026-11-24.

## Problem

After a meeting I have rough bullets and a long transcript labeled Me and Them. The transcript has the detail but is too long to reread; the bullets are short and miss things. Turning the two into clean notes with decisions and action items is still manual work. Mneme should do that step in about the time it takes to switch back to the app.

## Principle: the user decides where their notes go

Flexibility that enables privacy, not a fixed idea of what is private (decided 2026-11-24). Mneme runs a model on the Mac out of the box, and also lets the user point it at any server they choose: LM Studio on the same Mac, a box on their home network, or a hosted service they trust. Mneme does not pick or suggest a hosted provider. What it does guarantee:

- Nothing is sent anywhere until the user sets up a server themselves.
- It always says plainly where a meeting's notes and transcript go, before they are sent and after.
- With the default setup, everything stays on the Mac and the app works with Wi-Fi off.

## Goals

- One action turns a meeting's notes and transcript into enhanced notes: my notes kept as the outline, filled in from the transcript, plus decisions and action items.
- Each note line is matched to the conversation around the time it was typed, using `notes-timeline.jsonl`.
- Three places the model can run: a model from Mneme's list, a model file already on disk, or a server the user names.
- My own `notes.md` is never changed. Enhanced notes are a separate plain Markdown file in the meeting folder.
- Works on meetings up to two hours, the length the live pipeline already supports.
- A meeting that keeps its default title gets one from the enhanced notes.

## Non-goals

- Accounts, or a hosted provider chosen, bundled or recommended by Mneme.
- Provider-specific APIs. A server speaks the OpenAI-style chat API, which LM Studio, Ollama, llama.cpp's own server, vLLM and most hosted services offer.
- Note templates per meeting type.
- Q&A, chat or search across meetings (the later SQLite index).
- Enhancing during a meeting, or live suggestions.
- Speaker names beyond Me and Them.
- Marking which words came from me and which from the model inside the enhanced text. Possible later; see Open decisions.
- Non-English meetings (v1 is English only).

## Requirements

### Where the model runs

The user picks one source in Settings. All three share the prompt, the long-meeting split and the output rules below; only the engine call differs, behind one trait (`NotesModel` or similar) with a fake for tests.

| Source | Runs on | Network | Mneme manages memory |
| --- | --- | --- | --- |
| Mneme's list (default) | This Mac, bundled llama.cpp | Download once | Yes |
| Model file | This Mac, bundled llama.cpp | None | Yes |
| Server | Wherever the URL points | Every enhancement | No |

### Bundled engine

- llama.cpp with Metal, loading a GGUF model from a file path. Used by the first two sources.
- Output streams token by token so the UI can show progress and cancel can stop it within a second.
- Greedy or fixed-seed sampling, so the same inputs give the same notes and tests can rely on it.
- The model is loaded for an enhancement and freed after it. Unlike Whisper, it is not kept warm: it is several times larger and used once per meeting.
- **Spike first (issue 037).** whisper-rs and the llama.cpp bindings each build their own copy of ggml, the maths library under both. Try, in order, and record what worked:
  1. Both copies side by side in one binary. Simplest, if it links and runs cleanly.
  2. One shared ggml built for both. Cleanest, but whisper-rs is about ten whisper.cpp versions behind, so it may need updating or forking.
  3. llama.cpp's server as a bundled sidecar process, talked to over stdin/stdout or a socket bound to localhost only. Always works and keeps an LLM crash out of the app, at the cost of a second binary to sign.
  Also check how actively the chosen bindings crate (likely `llama-cpp-2`) is maintained, as was done for whisper-rs.

### Mneme's list

- Three models, one per tier. Sizes are 4-bit files; memory is for a one-hour meeting with Whisper's warm model on top:

  | Tier | Parameters | File | Memory | Recommended when the Mac has |
  | --- | --- | --- | --- | --- |
  | Small | 3–4B | 2–2.5 GB | 3–4 GB | under 16 GB |
  | Standard | 7–8B | 4.5–5 GB | 6–7 GB | 16–31 GB |
  | Large | 12–14B | 8–9 GB | 10–12 GB | 32 GB or more |

  These are estimates; the spike replaces them with measured numbers and picks the actual models by running candidates on real meetings from this Mac.
- Settings reads the Mac's memory (`hw.memsize`) and marks one model Recommended. Any of the three can still be picked; one too large for the Mac shows a warning, not a block.
- Same downloader as Whisper: temp file, progress, SHA-256 check, rename into place, delete, cancel. Checksums and sizes are copied from the source, never guessed.
- Licenses must allow an open-source app to download them (Apache-2.0 or MIT preferred). Each model's license is in the catalog and shown in Settings.
- Notes models are optional. First run does not ask for one.

### Model file

- "Use a model file…" opens a file picker for a `.gguf` file, for example one LM Studio already downloaded. The file is used where it is, not copied.
- Checked on pick: GGUF header, and the model name and context length read from its metadata. A file larger than about 60% of the Mac's memory shows a warning.
- A file that has moved or been deleted shows "Model file not found" with the path, and the Enhance button explains it.

### Server

- Settings fields: base URL, model name, context size in tokens, and an optional API key. A Test button lists the server's models (`GET /v1/models`) and fills the model name.
- Uses `POST /v1/chat/completions` with streaming.
- The API key is stored in the macOS Keychain, never in `settings.json`, logs or error messages, and is sent only to the host it was saved with.
- HTTPS is required except for loopback (`localhost`, `127.0.0.1`, `::1`) and private network addresses (`10.*`, `172.16–31.*`, `192.168.*`, `*.local`).
- Redirects to another host are not followed.
- Only the prompt is sent: notes, note times, transcript, title and date. No folder paths, settings or other meetings.
- Timeouts: 10 s to connect, 120 s without a new token. Errors show the host and the server's status and message.
- The context size is required, because some servers (Ollama by default) cut a long prompt silently instead of failing. Settings says so next to the field.

### Saying where notes go

- Settings shows where the current source sends notes, in one line: "Stays on this Mac", "Stays on this Mac (localhost:1234)", or "Sent to api.example.com".
- The first time a meeting would go to a host that is not loopback, a dialog names the host and asks once. The answer is remembered for that host; changing the URL to a new host asks again. This covers auto-enhance too.
- Each `enhanced.md` records where it ran, and the Enhanced view shows it under the text ("Enhanced on this Mac with Standard" or "Enhanced by api.example.com with model-name").

### What goes into the model

- The prompt gets:
  1. The notes body from `notes.md`, without frontmatter.
  2. The time each note line was first typed, from `notes-timeline.jsonl`. A line's time is the earliest entry whose text equals the final line or is a prefix of it, so a line that grew across autosaves counts from its first draft (as decided 2026-11-12). Lines typed after the meeting have no time and are passed without one.
  3. The transcript as `[hh:mm:ss] Me|Them: text` lines, with gap markers kept so the model knows audio is missing.
  4. The meeting title and date.
- Prompt building is plain Rust in its own crate (`mneme-enhance`), with no model in the loop, so it is fully unit tested.
- The prompt text lives in one file in the repo with a version number. The version is recorded in each output.

### Long meetings

- A one-hour meeting is roughly 12,000 transcript tokens, two hours roughly 25,000.
- If a meeting fits in the source's context size with room for the output, it is one pass.
- If not, the transcript is split into time sections at pauses, each section is enhanced with the notes typed during it, and a final pass merges the sections and writes decisions and action items. The split is unit tested without a model.
- For the bundled engine the context size is a parameter set from the spike, never the model's maximum by default (memory grows with it). For a server it is the size the user entered.

### What comes out

- Markdown, in this order:
  - My notes as the outline: my headings and bullets keep their order and wording where possible, with detail from the transcript added under them.
  - Anything important the notes missed, under a heading of its own.
  - **Decisions**: one line each.
  - **Action items**: `- [ ] Who: what`, where Who is Me, Them, or a name said in the transcript. Leave it out when no one was named.
- The model may only use facts from the notes and transcript. When the transcript is unclear it says so instead of guessing. Gap markers in the transcript are not filled in.
- If notes are empty, the outline comes from the transcript alone.
- A first line `# Title` suggests a title.

### Storage

```
2026-10-27 0930 Standup/
  notes.md               mine, never touched by enhancement
  transcript.md
  notes-timeline.jsonl
  enhanced.md            new: frontmatter + enhanced notes
```

- `enhanced.md` frontmatter: `generated_at`, `source` (`list`, `file` or `server`), `model` (catalog id, file name or server model name), `ran_on` (`this-mac` or the server's host), `prompt_version`, `source_hash` (hash of the notes body and transcript it was made from), `edited` (`false` until changed in the app).
- Written once at the end through the existing safe write. A quit mid-enhancement leaves no partial file; a leftover temp file is removed the next time the meeting is enhanced.
- It moves, renames and goes to the Trash with its meeting folder, so collections need no changes.
- Unknown frontmatter keys are kept on rewrite, as with `notes.md`.
- The meeting is "out of date" when `source_hash` no longer matches the notes and transcript. Nothing is regenerated automatically.

### Title

- When `title_edited` is false and the notes have no `# ` heading (issue 034 would already have titled it), the suggested title becomes the meeting title and the folder is renamed. It follows the existing title rules.
- A title typed by hand is never replaced.

### When it runs

- Enhancement runs when the user presses Enhance notes, on any stopped or interrupted meeting. Nothing runs by itself by default (decided 2026-11-24): an enhancement nobody asked for is noise, and with a server source it would send notes out unasked.
- A setting, "Enhance notes automatically after each meeting", off by default, starts it after Stop for users who always want it. It applies only when a source is set up, and only to meetings stopped after it was turned on.
- Never while recording, whatever the source. Starting a meeting cancels an enhancement in progress, frees a bundled model and closes a server request; it is offered again on that meeting afterwards. Mneme cannot free a local server's memory, so Settings notes that LM Studio or Ollama may keep the model loaded during meetings.
- One enhancement at a time across the app.
- Refused with a clear reason when the meeting is recording, has no transcript lines and no notes, or no source is set up.
- Running again over enhanced notes that were edited (`edited: true`) asks before replacing them.
- Cancel stops generation, frees the model or closes the request, and keeps any earlier `enhanced.md` unchanged.

### UI

- **Enhance notes button:** in the after view's top bar, next to the Transcript button, on every stopped or interrupted meeting. It reads Enhance notes before the first run and Re-enhance after. With no source set up it is still shown, and pressing it explains that a notes model is needed and opens that Settings section. Disabled with the reason while recording or while another meeting is enhancing.
- **Meeting, after:** a switch between My notes and Enhanced above the editor, shown once enhanced notes exist or are being written. Enhanced opens when it exists. Its text is editable in the same CodeMirror editor and autosaves to `enhanced.md`, setting `edited`. Where it ran is shown under the text.
- While it runs: the text streams into the Enhanced view, with a progress line ("Enhancing… 1:12", the section count when split) and Cancel. My notes stay editable.
- "Out of date" note beside the Re-enhance button when `source_hash` no longer matches.
- **Home:** a small mark on meetings that have enhanced notes.
- **Settings, Notes model section:** the three sources as a choice; under Mneme's list the three tiers with size, license, Recommended mark, download and delete; under Model file the picked path; under Server the fields and Test. The "where notes go" line and the "Enhance notes automatically after each meeting" switch (off by default) sit at the bottom.
- Errors use the existing inline error style and name the cause (out of memory, model file missing, server unreachable, cancelled by a new meeting).

### API contract additions

```ts
type NotesSource =
  | { kind: 'list'; modelId: string }
  | { kind: 'file'; path: string }
  | { kind: 'server'; baseUrl: string; model: string; contextTokens: number; hasApiKey: boolean };

type EnhanceStatus = 'none' | 'running' | 'done' | 'outOfDate' | 'error';
interface EnhanceState { meetingId: string; status: EnhanceStatus; elapsedSecs: number; section?: { index: number; count: number }; error?: string }
interface NotesModelInfo extends ModelInfo { tier: 'small' | 'standard' | 'large'; license: string; recommended: boolean; tooLarge: boolean }
interface ModelFileInfo { path: string; name: string; sizeBytes: number; contextTokens: number; tooLarge: boolean }

interface Meeting { /* existing fields */ enhanced: EnhanceStatus }
interface MeetingSummary { /* existing fields */ hasEnhanced: boolean }
interface Settings { /* existing fields */ notesSource: NotesSource | null; autoEnhance: boolean; approvedHosts: string[] }

interface Api {
  // existing methods
  enhanceMeeting(id: string, replaceEdited?: boolean): Promise<void>; // rejects with 'needsHostApproval' for an unapproved host
  approveHost(host: string): Promise<void>;
  cancelEnhance(): Promise<void>;
  getEnhancedNotes(id: string): Promise<string | null>;
  saveEnhancedNotes(id: string, text: string): Promise<void>;
  onEnhanceState(cb: (s: EnhanceState) => void): () => void;
  onEnhanceText(cb: (e: { meetingId: string; text: string }) => void): () => void; // the full text so far
  listNotesModels(): Promise<NotesModelInfo[]>; // downloads reuse the existing download calls and progress event
  chooseModelFile(): Promise<ModelFileInfo | null>;
  testNotesServer(baseUrl: string, apiKey?: string): Promise<{ models: string[] }>;
  setNotesServerKey(apiKey: string | null): Promise<void>; // Keychain
}
```

The mock implements all of it, streaming canned text, so the UI is built and tested in the browser first.

### Changes to the local-only rules

PRD 00, the README and `scripts/audit-local-only.sh` say the model downloader is the only code allowed to use the network. Issue 039 changes that to two places: the downloader, and the notes server client, in a small crate of its own that also uses `ureq`. The audit allows exactly those two. The server client makes no connection unless a server is saved in Settings, so the default app still works with Wi-Fi off and the release test (PRD 06) still holds. The README's "nothing leaves the machine" becomes "nothing leaves the machine unless you point Mneme at a server".

## Risks

| Risk | Why it matters | Next step |
| --- | --- | --- |
| Two copies of ggml | whisper.cpp and llama.cpp in one binary may not link, or may share state badly | Spike 037 tries three options; the server source works without any of them |
| Memory | Warm Whisper (up to 1.6 GB) + the model + its context can push an 8 GB or 16 GB Mac into swap | Never run during recording; free the model after; recommend by the Mac's memory |
| Invented facts | Small models fill gaps confidently, and wrong action items are worse than none | Prompt forbids it; manual review of real meetings; keep `notes.md` untouched so nothing is lost |
| Silent cut-off on servers | A server with a small context drops most of the transcript without an error | Context size is a required field; the split respects it; Settings warns about Ollama's default |
| Notes sent somewhere unintended | A typo or a pasted URL could send meetings to the wrong host | Ask once per new host, HTTPS off the local network, no redirects to other hosts, where it ran shown on every result |
| Me/Them labels wrong | Without headphones, Them is labeled Me and lines repeat; the notes will repeat the error | Do the open real-call check in known-gaps.md before judging enhancement quality |
| Speed | A slow first pass makes auto-enhance feel broken | Spike measures one and two hours; stream text so progress is visible |
| Model churn | Better small models appear every few months | Catalog is data; the prompt is not tied to one model family |

## Open decisions for Francesco

| Decision | Default in this draft |
| --- | --- |
| How the bundled engine is linked | Decided by the spike |
| Which three models are in the list | Decided by the spike from real meetings |
| Plain HTTP to private network addresses | Allowed; HTTPS required everywhere else |
| Ask once per host, or every time for hosts off the local network | Once per host |
| Show which lines are mine and which the model added | Not in this version |
| Keep the previous `enhanced.md` when re-running | No; ask first if it was edited |

## Proposed issues

| # | Issue | Depends on |
| --- | --- | --- |
| 037 | Spike: link llama.cpp beside whisper.cpp (three options), pick the three models, measure speed and memory at one and two hours. Prompt drafting uses LM Studio's server on this Mac so prompt quality is judged apart from the engine | — |
| 038 | Notes model list and downloads (reusing the Whisper downloader), memory-based recommendation, model file picker | 037 |
| 039 | Server client crate: chat API with streaming, Test, Keychain key, HTTPS and redirect rules, host approval; update the local-only rules and audit | — |
| 040 | `mneme-enhance` crate: timeline matching, prompt building, section split and merge, output checks; engine trait and fake | — |
| 041 | Storage: `enhanced.md`, source hash, out-of-date check, title from the suggestion | — |
| 042 | App: run, cancel, one at a time, cancel on New meeting, auto-enhance after Stop, Tauri commands and events | 039–041; 038 for the bundled sources |
| 043 | UI: My notes / Enhanced switch, streaming view, where-it-ran line, Home mark, Settings section, host dialog; mock API first | API contract above |
| 044 | Prompt tuning on real meetings and a small fixed set of test transcripts in the repo | 042 |

039–041 do not depend on the spike, so enhancement can work end to end through a server (LM Studio on this Mac) while the bundled engine is still being settled.

## Done when

- Unit tests cover timeline matching (growing lines, lines with no time), prompt building, section splitting, `enhanced.md` writes, the out-of-date check, the run rules (no run while recording, cancel on start, one at a time, ask before replacing edits), and the server rules (HTTPS outside the local network, no cross-host redirect, key never logged, unapproved host refused) against the fake engine and a local test server.
- An integration test enhances a short fixed transcript with the recommended model and finds the expected action item. It skips cleanly when the model file is absent.
- `scripts/audit-local-only.sh` passes with the two allowed network crates and fails if a third appears.
- Every screen change works against the mock API in the browser dev server.
- Manual, in the built app: three real meetings (about 15 minutes, one hour, close to two hours) are enhanced with the bundled engine and Wi-Fi off, timed, and read for invented facts; one is enhanced through LM Studio's server; Activity Monitor shows the bundled model's memory freed afterwards; starting a meeting during an enhancement cancels it and records normally.
