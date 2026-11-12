# 007: Model catalog and download manager

Status: todo · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 006 · Owns: `crates/mneme-transcribe/`

## Goal

Know which Whisper models exist, download the ones the user picks, verify them, and manage what is installed.

## Scope

1. **Catalog**: a fixed list covering the standard sizes (tiny, base, small, medium, large-v3, large-v3-turbo) and their English-only variants where they exist. Per model: id, display name, file name, download URL, size in bytes, SHA-256, English-only flag, and a short note on speed and quality.
   - Sizes and checksums come from the model source (`ggerganov/whisper.cpp` on Hugging Face). Look each one up; do not write any from memory. If one cannot be confirmed, leave that model out and say so in the Result.
2. **`ModelStore`** over a models folder: list catalog entries with an installed flag, path for an installed model, delete.
3. **Download**: stream to a `.part` file in the models folder, report progress (received and total bytes) through a callback or channel, verify SHA-256, then rename into place. Supports cancel. On failure, cancel or checksum mismatch, the `.part` file is removed and the model is not installed. A file is "installed" only if it has the final name.
4. Leftover `.part` files from a crash are cleaned up when the store is opened.
5. The HTTP client is used only here. Keep the dependency small and TLS on.

## Acceptance criteria

- [ ] Tests against a local HTTP server in the test process: successful download and verify; checksum mismatch leaves nothing installed; cancel mid-download leaves nothing; server error is reported; progress reaches the total.
- [ ] Tests for list, delete and `.part` cleanup using a temp folder.
- [ ] A test that every catalog URL is HTTPS on the expected host and every checksum is 64 hex characters.
- [ ] An ignored test that downloads the smallest real model and verifies its checksum.
- [ ] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

_Filled in by the implementer._
