# 007: Model catalog and download manager

Status: done · PRD: [02](../prds/02-transcription-and-models.md) · Lane: transcribe · Depends on: 006 · Owns: `crates/mneme-transcribe/`

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

- [x] Tests against a local HTTP server in the test process: successful download and verify; checksum mismatch leaves nothing installed; cancel mid-download leaves nothing; server error is reported; progress reaches the total.
- [x] Tests for list, delete and `.part` cleanup using a temp folder.
- [x] A test that every catalog URL is HTTPS on the expected host and every checksum is 64 hex characters.
- [x] An ignored test that downloads the smallest real model and verifies its checksum.
- [x] `cargo test -p mneme-transcribe` and clippy pass.

## Manual checks

None.

## Result

Done 2026-11-12 on macOS 27.0.1, Apple Silicon, rustc 1.100.0.

### Catalog

Ten models: the six standard sizes, plus the English-only variant of the four sizes that have one. Large v3 and Large v3 Turbo have no English-only file in the repository. The file of each entry is `ggml-<id>.bin`.

| id | name | file | bytes | SHA-256 | English only |
| --- | --- | --- | ---: | --- | --- |
| `tiny` | Tiny | `ggml-tiny.bin` | 77,691,713 | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` | no |
| `tiny.en` | Tiny (English) | `ggml-tiny.en.bin` | 77,704,715 | `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f` | yes |
| `base` | Base | `ggml-base.bin` | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` | no |
| `base.en` | Base (English) | `ggml-base.en.bin` | 147,964,211 | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` | yes |
| `small` | Small | `ggml-small.bin` | 487,601,967 | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` | no |
| `small.en` | Small (English) | `ggml-small.en.bin` | 487,614,201 | `c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d` | yes |
| `medium` | Medium | `ggml-medium.bin` | 1,533,763,059 | `6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208` | no |
| `medium.en` | Medium (English) | `ggml-medium.en.bin` | 1,533,774,781 | `cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356` | yes |
| `large-v3-turbo` | Large v3 Turbo | `ggml-large-v3-turbo.bin` | 1,624,555,275 | `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69` | no |
| `large-v3` | Large v3 | `ggml-large-v3.bin` | 3,095,033,483 | `64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2` | no |

Where the values come from, read on 2026-11-12 (Hugging Face's clock said 2026-11-13 00:58 GMT):

- Source 1: `https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/<revision>`. Each file is listed with `lfs.oid` (the SHA-256 of the LFS object) and `lfs.size`. The table rows were generated from the saved response, not typed.
- Source 2: a `HEAD` request on each of the ten download URLs. Each answered `302` with `x-linked-size`, `x-linked-etag` (the SHA-256) and `x-repo-commit`. All ten agree with source 1.
- Cross-check on a real file: `shasum -a 256 crates/mneme-transcribe/models/ggml-tiny.en.bin` gives `921e4cf8…0b1f` and the file is 77,704,715 bytes, equal to the catalog entry and to what issue 005 recorded. A unit test repeats this whenever the file is there. The ignored test downloaded the same file through the new downloader and it verified.
- No model was left out for lack of confirmation. Not in the catalog by choice: `large-v1`, `large-v2`, the quantized files (`-q5_0`, `-q5_1`, `-q8_0`) and the Core ML encoder archives.

**Pinned revision.** Hugging Face accepts a commit in place of `main` in resolve URLs. Every catalog URL is `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/<file>`. That commit was the head of `main` on 2026-11-12 (repository last modified 2024-10-29), and `x-repo-commit` confirmed it on every request. A later change to the repository cannot make a checksum stale. To move forward, read both sources again and change `MODEL_REVISION` and the table together.

**Notes.** Each entry has one line of relative wording ("Fastest and smallest. Least accurate.", "More accurate than Small. Slower.", "Nearly as accurate as Large v3, and faster."). They are orderings by model size and what the model authors say about the turbo model, not measurements. Issue 008 measures.

**Ids compared with the mock** (`src/lib/api/mockSeed.ts`). The mock's five ids (`tiny.en`, `base`, `small`, `medium`, `large-v3-turbo`) are all in the catalog with the same ids, names and sizes. The catalog adds `tiny`, `base.en`, `small.en`, `medium.en` and `large-v3`. The note texts differ from the mock's.

### What was built

In `crates/mneme-transcribe/src/models/` (`catalog.rs`, `store.rs`, `download.rs`, `error.rs`), re-exported from the crate root:

```rust
pub const MODEL_HOST: &str = "huggingface.co";
pub const MODEL_REPOSITORY: &str = "ggerganov/whisper.cpp";
pub const MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

pub struct ModelSpec { pub id: String, pub name: String, pub file_name: String, pub url: String,
    pub size_bytes: u64, pub sha256: String, pub english_only: bool, pub note: String }
pub fn catalog() -> &'static [ModelSpec];          // smallest file first
pub fn find_model(id: &str) -> Option<&'static ModelSpec>;

// Field for field the frontend's ModelInfo.
pub struct ModelInfo { pub id: String, pub name: String, pub size_bytes: u64, pub note: String,
    pub english_only: bool, pub installed: bool }

pub struct ModelStore;                              // Clone + Send + Sync; clones share downloads
impl ModelStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ModelError>;   // creates dir, removes *.part
    pub fn open_with_catalog(dir: impl AsRef<Path>, catalog: Vec<ModelSpec>) -> Result<Self, ModelError>;
    pub fn dir(&self) -> &Path;
    pub fn catalog(&self) -> &[ModelSpec];
    pub fn spec(&self, id: &str) -> Result<&ModelSpec, ModelError>;
    pub fn list(&self) -> Vec<ModelInfo>;
    pub fn is_installed(&self, id: &str) -> bool;
    pub fn installed_path(&self, id: &str) -> Result<PathBuf, ModelError>;
    pub fn delete(&self, id: &str) -> Result<(), ModelError>;
    pub fn verify(&self, id: &str) -> Result<(), ModelError>;         // full SHA-256 of the installed file
    pub fn start_download<F>(&self, id: &str, on_progress: F) -> Result<DownloadHandle, ModelError>
        where F: FnMut(DownloadProgress) + Send + 'static;
    pub fn cancel_download(&self, id: &str) -> bool;                  // false: nothing was running
    pub fn is_downloading(&self, id: &str) -> bool;
    pub fn active_downloads(&self) -> Vec<String>;
}

pub struct DownloadHandle;                          // dropping it leaves the download running
impl DownloadHandle {
    pub fn model_id(&self) -> &str;
    pub fn cancel(&self);
    pub fn is_finished(&self) -> bool;
    pub fn wait(self) -> Result<PathBuf, ModelError>;                 // blocks until the end
}

// Field for field the frontend's DownloadProgress.
pub struct DownloadProgress { pub model_id: String, pub received_bytes: u64, pub total_bytes: u64,
    pub state: DownloadState, pub error: Option<String> }
pub enum DownloadState { Downloading, Verifying, Done, Cancelled, Error }
impl DownloadState { pub const fn as_str(self) -> &'static str;      // "downloading" | "verifying" | "done" | "cancelled" | "error"
                     pub const fn is_terminal(self) -> bool; }

pub enum ModelError { UnknownModel { id }, AlreadyInstalled { id, name }, AlreadyDownloading { id, name },
    NotInstalled { id, name }, InvalidCatalog { reason }, InsecureUrl { url }, BadRedirect { url, reason },
    TooManyRedirects { limit }, HttpStatus { status, url }, Network { message },
    SizeMismatch { expected, actual }, ChecksumMismatch { expected, actual }, Cancelled, Io { path, message } }
```

Behaviour:

- **Installed** means a regular file with the model's final name and exactly the catalog's size. One `metadata` call per model, so listing is cheap. The SHA-256 is checked once, when the download finishes; `verify` repeats it on request (it reads the whole file). A `.part` file, a file of the wrong size and a folder with the model's name are not installed.
- **Download.** `start_download` returns at once and works on its own thread. It streams to `<file name>.part`, hashing as the bytes arrive, then flushes the file to disk, compares size and SHA-256, and renames. Events, all on the download thread: `downloading` with 0 bytes; `downloading` about ten times a second while bytes arrive; `downloading` with the full size; `verifying`; then exactly one of `done`, `cancelled`, `error` (with `error` holding a sentence for the user). `total_bytes` is the catalog size in every event. By the time the last event is delivered the `.part` file is gone and the same id can be started again. The `verifying` phase is the flush to disk and the comparison, so it is short; there is no second pass over the file.
- **Failures.** A response that is not `200`, a `Content-Length` different from the catalog size (refused before any file is created), more or fewer bytes than the catalog size, a dropped or stalled connection (no bytes for 30 s), a checksum mismatch, a disk error, or a panic in the callback: each removes the `.part` file and installs nothing. The `.part` file is deleted by a drop guard, so there is one clean-up path for all of them.
- **Cancel** returns at once and sets a flag; it also shuts down that download's HTTP agent (`ureq::Agent::shutdown`), which makes a read that is waiting on the network return immediately. So cancel works while bytes flow and while the connection is stalled. During name resolution or TCP connect it takes effect when those finish or time out (15 s and 20 s). A cancel that arrives after the rename changes nothing: that download is `done`.
- **Two downloads of the same model**: the second `start_download` returns `AlreadyDownloading`, sends no events and does not touch the first. Different models download side by side. Downloading an installed model returns `AlreadyInstalled`; delete it first to fetch it again.
- **Delete** removes the file with the model's name. If there is none it returns `NotInstalled` ("Base is not installed."); an unknown id returns `UnknownModel`. A wrong-size file under the model's name is deleted too. Messages for these cases match the mock's.
- **No resume.** A download interrupted by a crash or a quit leaves a `.part` file. `open` deletes every `*.part` file directly in the folder, and the download starts from zero next time.
- **One store per folder.** Opening a second `ModelStore` on a folder, in this process or another, deletes the `.part` of a download the first one is running; that download then fails with an error and installs nothing.

### HTTP client and the HTTPS-only rule

`ureq` 3.5.2 with `default-features = false, features = ["rustls"]`. It is blocking, so it brings no async runtime and forces none on callers; the store runs each download on a plain thread. It has the two things the downloader needs beyond GET: a per-read timeout (`timeout_per_read`, so a stalled download fails after 30 s and a slow one is not cut off by a total time limit) and `Agent::shutdown()` to interrupt a blocked read on cancel. TLS is rustls with the `ring` provider and the bundled Mozilla root certificates (`webpki-roots`). gzip is off. Proxy environment variables are ignored (`proxy(None)`): connections are direct. The `User-Agent` is ureq's default (`ureq/3.5.2`), which says nothing about the app or the user.

What Hugging Face does, observed on 2026-11-12: a resolve URL answers `302` with an absolute `Location` on `https://us.aws.cdn.hf.co/…` carrying signed, short-lived query parameters; that URL answers `200` with the file and a `Content-Length`. One redirect, HTTPS to HTTPS.

How the rule holds across redirects: ureq is told to follow no redirects (`max_redirects(0)`). The downloader follows them itself, at most five, and passes every URL, the first and each `Location`, through one check (`checked_url`) before anything is sent to it. Anything that is not `https://` is refused with `InsecureUrl`. Relative and scheme-relative `Location` values are resolved against the current URL, so they stay HTTPS. As a second layer the agent is built with `https_only(true)`. The redirect host is not restricted: Hugging Face chooses the CDN host, and the SHA-256 check is what protects the content. Error messages leave out query strings, so signed CDN parameters never reach the UI or a log.

The tests' server is plain HTTP on 127.0.0.1. A policy that also accepts `http://127.0.0.1` exists for them; it can only be constructed in `#[cfg(test)]` code inside this crate. `ModelStore::open` and `open_with_catalog` always use the HTTPS-only policy, and a test shows that a store opened that way refuses the test server without connecting.

### Dependencies added

In `crates/mneme-transcribe/Cargo.toml`:

- `ureq = { version = "3.5.2", default-features = false, features = ["rustls"] }`. **The only crate that can open a socket.** I searched the sources of everything it pulls in for `TcpStream`, `UdpSocket`, `ToSocketAddrs` and raw socket calls: only ureq connects (`TcpStream::connect` in its `transport/tcp.rs`, name lookup in `resolver.rs`). `rustls` encrypts over a stream it is given and `ureq-proto` does no I/O.
- Pulled in by ureq: `ureq-proto`, `rustls`, `rustls-webpki`, `rustls-pki-types`, `webpki-roots`, `ring` (compiles C and assembly with `cc`), `untrusted`, `subtle`, `zeroize`, `getrandom` 0.2, `utf8-zero`. `http`, `httparse`, `base64`, `percent-encoding`, `log`, `once_cell` were already in the lock file. The lock file also gained `windows-sys` 0.52 and its target crates, which `ring` names for Windows only; they are not built on macOS.
- `sha2 = "0.11.1"` with `digest`, `block-buffer`, `crypto-common`, `hybrid-array`, `const-oid`, `cpufeatures`. No I/O. The lock file also has `sha2` 0.10.9 from another dependency, so there are two copies.
- Dev only: `tempfile = "3.27.0"` (already in the lock file).
- No test HTTP server crate: the server in `src/models/test_server.rs` is about 200 lines on `std::net`, compiled only for tests.

### Verification

| Command | Outcome |
| --- | --- |
| `cargo test -p mneme-transcribe` | Passed: 110 unit tests (86 from before, 24 new), 5 chunker, 8 in `tests/models.rs` plus 1 ignored, 10 Whisper engine, 4 doc tests. The 104 tests from before all still pass. |
| `cargo clippy -p mneme-transcribe --all-targets -- -D warnings` | Passed, no warnings. |
| `cargo fmt -p mneme-transcribe -- --check` | Passed. |
| `cargo doc -p mneme-transcribe --no-deps` | No warnings. |
| `cargo test -p mneme-transcribe --test models -- --ignored --nocapture` (run once) | Passed. It downloaded `ggml-tiny.en.bin` from the pinned Hugging Face URL into a temp folder: 77,704,715 bytes in 5.2 s, 47 progress events ending `downloading`, `verifying`, `done`; the file was then read back from disk and its SHA-256 matched the catalog. This is the only run that used the network, and nothing else was downloaded. |
| `cargo test -p mneme-transcribe --lib -- models::`, six times in a row | 24 passed each time, about 2 s. |
| `crates/mneme-transcribe/scripts/fetch-test-model.sh` | "Already there and verified". Its `curl` flags were checked with a `HEAD` request: 200 after one redirect. |

Acceptance criteria:

1. **Local HTTP server tests: pass** (`src/models/tests.rs`). Success with verify; checksum mismatch; cancel while stalled and cancel while bytes arrive; statuses 500 and 404; progress reaching the total. Also: refused connection, wrong declared size, body too long, too short and cut off, stall timeout, redirects followed (absolute and relative), redirect to plain HTTP refused with nothing sent to it, redirect loop, redirect without `Location`, second download of the same model refused, unknown id, wrong-size file replaced, `verify` catching a changed file, a panicking callback, and the URL rules as pure functions.
2. **List, delete and `.part` cleanup in a temp folder: pass** (`tests/models.rs`), plus catalog validation.
3. **Catalog shape: passes.** Every URL is exactly `https://huggingface.co/ggerganov/whisper.cpp/resolve/<40 hex>/<file>`, every checksum is 64 lowercase hex characters, ids, names, files and checksums are unique.
4. **Ignored real download: present, and run once successfully** (table above). It downloads `tiny.en`, 13 KB larger than `tiny`, because that is the file already on this machine to compare with.
5. **`cargo test` and clippy: pass.**

That the tests fail when the behaviour is broken was checked by breaking the code thirteen ways, one at a time, and running them: checksum comparison skipped; `.part` left behind on failure; cancel without the agent shutdown; redirect target not checked; plain HTTP accepted; no final progress event; HTTP status ignored; no `.part` cleanup at open; installed check without the size; a finished download left registered; second download allowed; deleting an absent model succeeding; rename skipped. Each made at least one test fail. Two tests were tightened on the way: the stalled-cancel test passed without the shutdown until it was made to wait for the read to block, and a quick download needed an explicit check that its last `downloading` event carries the total.

### Not verified

- Nine of the ten model files were never downloaded. Their sizes and checksums agree between two Hugging Face sources and their URLs answer, but only `tiny.en` was fetched and hashed.
- An HTTPS-to-HTTP downgrade from a real TLS server. The refusal is tested on the function that checks every hop, and end to end against the local server with a plain-HTTP target the policy does not accept.
- Cancel during name resolution or TCP connect, and cancel while waiting for response headers. The first is bounded by the timeouts above by design; neither has its own test.
- A full disk, and a real network dropping mid-download. Both are covered only by the local server's dropped and stalled connections.
- Networks that need a proxy or that inspect TLS with their own certificate. Both are expected to fail: no proxy support, and the trust store is the bundled Mozilla list, not the macOS keychain.
- Redirect hosts in other regions. Only `us.aws.cdn.hf.co` was seen from this machine.
- How long the flush to disk in `verifying` takes for the 3 GB model.

### Deviations from the issue text

- Deleting an absent model is an error (`NotInstalled`), not a silent success, to match the mock and give the UI a message.
- "Installed" also requires the exact size, not only the final name.
- `verify`, `open_with_catalog`, `is_downloading` and `active_downloads` are additions.
- `fetch-test-model.sh` now uses the pinned revision and `--proto-redir '=https'`, so it follows the same rule as the downloader.

### Follow-ups

- **Orchestrator: `Cargo.lock`.** The lock entries for `ureq`, `sha2` 0.11.1 and their dependencies are already in commit `0a3639f` (issue 003), which was made after my first build. Nothing more to add for the lock; the commit for this issue brings the matching `Cargo.toml`. `issues/README.md` still lists 007 as in progress.
- **008.** Resolve `--model-id` with `ModelStore::open(dir)?.installed_path(id)`. Do not run it on the app's models folder while the app is downloading (see "One store per folder"). After measuring, rewrite the notes in `catalog.rs` if the numbers disagree with the relative wording. Only `tiny.en` is on this machine; other sizes have to be downloaded first, 148 MB to 3.1 GB each.
- **019 and 020.** The mock's five ids are valid. Add `tiny`, `base.en`, `small.en`, `medium.en` and `large-v3` to the mock catalog, or decide the picker shows fewer than the backend lists, and take the note texts from `catalog.rs`. `verifying` is brief. `cancelled` arrives a moment after `cancelDownload` returns, not before as in the mock, and up to about 35 s later if the connection was still being set up (name resolution 15 s, connect 20 s), so show a cancelling state. The first `downloading` event can arrive just after `downloadModel` resolves. Error texts are full sentences, ready to show.
- **021.** Keep one `ModelStore` in app state, opened once at launch on the models folder in the app data dir. Map `ModelInfo` and `DownloadProgress` field for field (`state.as_str()`, `error`). `downloadModel` is `store.start_download(id, |p| emit(p))`; return once it has started and drop the handle. `cancelDownload` is `store.cancel_download(id)` and ignores `false`. The store does not know which model a recording uses or which is the default: refuse deleting the model in use and clear `defaultModelId` in the command, as the mock does. On quit, cancel `active_downloads()` or just exit; a leftover `.part` is removed at the next launch. Tests in `src-tauri` cannot point the real downloader at a local server, by design; build `DownloadProgress` values directly or use `open_with_catalog` with files placed in a temp folder.
- **025.** `ureq` is the only socket-capable crate under `mneme-transcribe`. ureq 3.5.2 was published on 2026-11-12, the day it was adopted; look at it again before release. Decide whether redirects should be limited to Hugging Face hosts (`huggingface.co`, `*.hf.co`); today any HTTPS host is followed and the checksum decides. Decide whether proxy support or the system trust store is wanted.
- **Disk space** is not checked before a download; a full disk surfaces as an `Io` error with nothing installed.
