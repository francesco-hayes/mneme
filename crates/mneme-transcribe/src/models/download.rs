//! Downloads one model file: the only code in the app that opens a network
//! connection (PRD 00, local-only rules).
//!
//! A download streams to `<file name>.part` in the models folder, hashing the
//! bytes as they arrive, then checks size and SHA-256 and renames the file
//! into place. Whatever goes wrong, the `.part` file is removed.
//!
//! Every URL contacted must be HTTPS: the catalog URL and each redirect after
//! it. Redirects are followed here, one hop at a time, so each `Location` is
//! checked before anything is sent to it; `ureq` is told to follow none.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use ureq::http::Uri;

use super::catalog::ModelSpec;
use super::error::ModelError;

/// Added to a model's file name while it is being downloaded.
pub(crate) const PART_SUFFIX: &str = ".part";

/// Redirects followed in a row before giving up. Hugging Face needs one.
pub(crate) const MAX_REDIRECTS: u32 = 5;

/// The longest gap between two `downloading` events while bytes arrive.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

const READ_BUFFER_BYTES: usize = 64 * 1024;

/// Where a download is.
///
/// [`DownloadState::as_str`] gives the names the frontend contract uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloadState {
    /// Bytes are arriving.
    Downloading,
    /// Every byte arrived; the file is being flushed to disk and its
    /// checksum compared.
    Verifying,
    /// The model is installed. Last event of a download.
    Done,
    /// The download was cancelled and nothing was installed. Last event.
    Cancelled,
    /// The download failed and nothing was installed. Last event.
    Error,
}

impl DownloadState {
    /// `downloading`, `verifying`, `done`, `cancelled` or `error`.
    pub const fn as_str(self) -> &'static str {
        match self {
            DownloadState::Downloading => "downloading",
            DownloadState::Verifying => "verifying",
            DownloadState::Done => "done",
            DownloadState::Cancelled => "cancelled",
            DownloadState::Error => "error",
        }
    }

    /// True for the states that end a download: done, cancelled and error.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            DownloadState::Done | DownloadState::Cancelled | DownloadState::Error
        )
    }
}

impl fmt::Display for DownloadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One progress event of a download. Field for field the frontend's
/// `DownloadProgress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// The catalog id of the model.
    pub model_id: String,
    /// Bytes written so far.
    pub received_bytes: u64,
    /// The model's size from the catalog. The same in every event.
    pub total_bytes: u64,
    pub state: DownloadState,
    /// What went wrong, as a sentence for the user. Set only when `state` is
    /// [`DownloadState::Error`].
    pub error: Option<String>,
}

/// The network rules of a download.
///
/// The app only ever uses [`NetPolicy::PRODUCTION`]. The plain-HTTP switch
/// exists for this crate's own tests, whose server is plain HTTP on
/// localhost; nothing outside `#[cfg(test)]` code can turn it on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NetPolicy {
    /// Accept `http://127.0.0.1` URLs.
    pub allow_plain_http_to_loopback: bool,
    pub resolve_timeout: Duration,
    /// TCP connect and TLS handshake.
    pub connect_timeout: Duration,
    /// From the request being sent to the response headers arriving.
    pub response_timeout: Duration,
    /// The longest a single read may wait for bytes before the download
    /// fails as stalled.
    pub read_timeout: Duration,
}

impl NetPolicy {
    pub(crate) const PRODUCTION: NetPolicy = NetPolicy {
        allow_plain_http_to_loopback: false,
        resolve_timeout: Duration::from_secs(15),
        connect_timeout: Duration::from_secs(20),
        response_timeout: Duration::from_secs(30),
        read_timeout: Duration::from_secs(30),
    };
}

/// Stops one download from another thread.
#[derive(Clone)]
pub(crate) struct CancelToken {
    inner: Arc<CancelInner>,
}

struct CancelInner {
    cancelled: AtomicBool,
    /// The agent this download makes its requests with, and no other.
    agent: ureq::Agent,
}

impl CancelToken {
    pub(crate) fn new(policy: &NetPolicy) -> Self {
        CancelToken {
            inner: Arc::new(CancelInner {
                cancelled: AtomicBool::new(false),
                agent: build_agent(policy),
            }),
        }
    }

    /// Asks the download to stop. Closing the agent's connections makes a
    /// read that is waiting on the network return at once, so a stalled
    /// download does not have to wait out its read timeout.
    pub(crate) fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.agent.clone().shutdown();
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    fn agent(&self) -> &ureq::Agent {
        &self.inner.agent
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

fn build_agent(policy: &NetPolicy) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        // Second line of defence: `checked_url` below already refuses every
        // URL that is not HTTPS before it reaches the agent.
        .https_only(!policy.allow_plain_http_to_loopback)
        // Redirects are followed in `open_response`, one checked hop at a time.
        .max_redirects(0)
        // Statuses are handled in `open_response`.
        .http_status_as_error(false)
        // Connect directly; ignore proxy environment variables.
        .proxy(None)
        .timeout_resolve(Some(policy.resolve_timeout))
        .timeout_connect(Some(policy.connect_timeout))
        .timeout_recv_response(Some(policy.response_timeout))
        .timeout_per_read(Some(policy.read_timeout))
        .build();
    ureq::Agent::new_with_config(config)
}

/// Sends progress events to the caller's callback.
pub(crate) struct Reporter<'a> {
    model_id: &'a str,
    total_bytes: u64,
    received_bytes: u64,
    /// The byte count of the last `downloading` event sent.
    reported_bytes: Option<u64>,
    last_event: Instant,
    callback: &'a mut (dyn FnMut(DownloadProgress) + Send),
}

impl<'a> Reporter<'a> {
    pub(crate) fn new(
        spec: &'a ModelSpec,
        callback: &'a mut (dyn FnMut(DownloadProgress) + Send),
    ) -> Self {
        Reporter {
            model_id: &spec.id,
            total_bytes: spec.size_bytes,
            received_bytes: 0,
            reported_bytes: None,
            last_event: Instant::now(),
            callback,
        }
    }

    /// Sends an event in `state` with the current byte count.
    pub(crate) fn send(&mut self, state: DownloadState, error: Option<String>) {
        if state == DownloadState::Downloading {
            self.reported_bytes = Some(self.received_bytes);
            self.last_event = Instant::now();
        }
        (self.callback)(DownloadProgress {
            model_id: self.model_id.to_owned(),
            received_bytes: self.received_bytes,
            total_bytes: self.total_bytes,
            state,
            error,
        });
    }

    /// Records bytes written and sends a `downloading` event if the last one
    /// is old enough.
    fn received(&mut self, received_bytes: u64) {
        self.received_bytes = received_bytes;
        if self.last_event.elapsed() >= PROGRESS_INTERVAL {
            self.send(DownloadState::Downloading, None);
        }
    }

    /// Sends a `downloading` event for the current byte count unless one was
    /// already sent, so the last `downloading` event always carries the total.
    fn flush(&mut self) {
        if self.reported_bytes != Some(self.received_bytes) {
            self.send(DownloadState::Downloading, None);
        }
    }
}

/// The `.part` file of a download. Dropping it deletes the file unless
/// [`PartFile::install`] renamed it, which covers every way out of a
/// download: an error, a cancel, or a panic in the progress callback.
struct PartFile {
    path: PathBuf,
    file: Option<File>,
    installed: bool,
}

impl PartFile {
    fn create(path: PathBuf) -> Result<Self, ModelError> {
        let file = File::create(&path).map_err(|e| ModelError::io(&path, &e))?;
        Ok(PartFile {
            path,
            file: Some(file),
            installed: false,
        })
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), ModelError> {
        let file = self.file.as_mut().expect("the part file is open");
        file.write_all(bytes)
            .map_err(|e| ModelError::io(&self.path, &e))
    }

    /// Flushes the bytes to the disk itself, so a model that looks installed
    /// after a power cut is complete.
    fn sync(&mut self) -> Result<(), ModelError> {
        let file = self.file.as_mut().expect("the part file is open");
        file.flush()
            .and_then(|()| file.sync_all())
            .map_err(|e| ModelError::io(&self.path, &e))
    }

    /// Renames the file to its final name. From here on the model is
    /// installed.
    fn install(mut self, final_path: &Path) -> Result<(), ModelError> {
        self.file = None;
        fs::rename(&self.path, final_path).map_err(|e| ModelError::io(final_path, &e))?;
        self.installed = true;
        Ok(())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        if !self.installed {
            self.file = None;
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The path a model is downloaded to before it is renamed into place.
pub(crate) fn part_path(dir: &Path, spec: &ModelSpec) -> PathBuf {
    dir.join(format!("{}{PART_SUFFIX}", spec.file_name))
}

/// Downloads `spec` into `dir` and returns the installed file's path.
///
/// Sends `downloading` and `verifying` events; the caller sends the final
/// one. On any error the `.part` file is gone when this returns, and after a
/// cancel the error is always [`ModelError::Cancelled`].
pub(crate) fn download(
    spec: &ModelSpec,
    dir: &Path,
    policy: &NetPolicy,
    cancel: &CancelToken,
    reporter: &mut Reporter<'_>,
) -> Result<PathBuf, ModelError> {
    let result = download_inner(spec, dir, policy, cancel, reporter);
    match result {
        // Closing the connection to cancel surfaces as a network error.
        Err(_) if cancel.is_cancelled() => Err(ModelError::Cancelled),
        other => other,
    }
}

fn download_inner(
    spec: &ModelSpec,
    dir: &Path,
    policy: &NetPolicy,
    cancel: &CancelToken,
    reporter: &mut Reporter<'_>,
) -> Result<PathBuf, ModelError> {
    let response = open_response(cancel, policy, &spec.url)?;
    if let Some(declared) = response.body().content_length()
        && declared != spec.size_bytes
    {
        return Err(ModelError::SizeMismatch {
            expected: spec.size_bytes,
            actual: declared,
        });
    }

    fs::create_dir_all(dir).map_err(|e| ModelError::io(dir, &e))?;
    let mut part = PartFile::create(part_path(dir, spec))?;
    let mut body = response.into_body().into_reader();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    let mut received: u64 = 0;
    loop {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let count = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(ModelError::Network {
                    message: error.to_string(),
                });
            }
        };
        received += count as u64;
        if received > spec.size_bytes {
            // Stop before a wrong or hostile server fills the disk.
            return Err(ModelError::SizeMismatch {
                expected: spec.size_bytes,
                actual: received,
            });
        }
        part.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        reporter.received(received);
    }
    drop(body);
    if received != spec.size_bytes {
        return Err(ModelError::SizeMismatch {
            expected: spec.size_bytes,
            actual: received,
        });
    }
    reporter.flush();

    if cancel.is_cancelled() {
        return Err(ModelError::Cancelled);
    }
    reporter.send(DownloadState::Verifying, None);
    part.sync()?;
    let actual = hex(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        return Err(ModelError::ChecksumMismatch {
            expected: spec.sha256.clone(),
            actual,
        });
    }

    // Last point at which a cancel still wins.
    if cancel.is_cancelled() {
        return Err(ModelError::Cancelled);
    }
    let final_path = dir.join(&spec.file_name);
    part.install(&final_path)?;
    Ok(final_path)
}

/// Requests `url`, following redirects, and returns the `200` response.
fn open_response(
    cancel: &CancelToken,
    policy: &NetPolicy,
    url: &str,
) -> Result<ureq::http::Response<ureq::Body>, ModelError> {
    let mut url = checked_url(url, policy)?;
    for _ in 0..=MAX_REDIRECTS {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let response = cancel
            .agent()
            .get(url.as_str())
            // The checksum is of the file as stored; ask for it unencoded.
            .header("accept-encoding", "identity")
            .call()
            .map_err(|error| request_error(error, &url))?;
        let status = response.status().as_u16();
        match status {
            200 => return Ok(response),
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| ModelError::BadRedirect {
                        url: without_query(&url),
                        reason: "it has no Location header".to_owned(),
                    })?;
                url = redirect_target(&url, location, policy)?;
            }
            _ => {
                return Err(ModelError::HttpStatus {
                    status,
                    url: without_query(&url),
                });
            }
        }
    }
    Err(ModelError::TooManyRedirects {
        limit: MAX_REDIRECTS,
    })
}

fn request_error(error: ureq::Error, url: &str) -> ModelError {
    match error {
        ureq::Error::RequireHttpsOnly(_) => ModelError::InsecureUrl {
            url: without_query(url),
        },
        other => ModelError::Network {
            message: other.to_string(),
        },
    }
}

/// Returns `url` if the policy allows contacting it, and refuses it
/// otherwise: anything that is not HTTPS, with the one test-only exception
/// described on [`NetPolicy`].
pub(crate) fn checked_url(url: &str, policy: &NetPolicy) -> Result<String, ModelError> {
    let insecure = || ModelError::InsecureUrl {
        url: without_query(url),
    };
    let uri: Uri = url.parse().map_err(|_| insecure())?;
    let scheme = uri.scheme_str().ok_or_else(insecure)?;
    let host = uri
        .host()
        .filter(|host| !host.is_empty())
        .ok_or_else(insecure)?;
    let allowed = scheme.eq_ignore_ascii_case("https")
        || (policy.allow_plain_http_to_loopback
            && scheme.eq_ignore_ascii_case("http")
            && is_loopback(host));
    if allowed {
        Ok(url.to_owned())
    } else {
        Err(insecure())
    }
}

/// Only the literal address the test server binds to. A name such as
/// `localhost` does not count.
fn is_loopback(host: &str) -> bool {
    host == "127.0.0.1"
}

/// The URL a redirect from `current` with this `Location` header leads to,
/// checked against the policy.
pub(crate) fn redirect_target(
    current: &str,
    location: &str,
    policy: &NetPolicy,
) -> Result<String, ModelError> {
    let bad = |reason: &str| ModelError::BadRedirect {
        url: without_query(current),
        reason: reason.to_owned(),
    };
    let location = location.trim();
    let current_uri: Uri = current
        .parse()
        .map_err(|_| bad("the current URL is invalid"))?;
    let scheme = current_uri
        .scheme_str()
        .ok_or_else(|| bad("the current URL has no scheme"))?;
    let authority = current_uri
        .authority()
        .ok_or_else(|| bad("the current URL has no host"))?;

    let target = if location.starts_with("//") {
        format!("{scheme}:{location}")
    } else if location.starts_with('/') {
        format!("{scheme}://{authority}{location}")
    } else if location.contains("://") {
        location.to_owned()
    } else {
        return Err(bad("its Location is not an absolute URL or path"));
    };
    checked_url(&target, policy)
}

/// `url` without its query string and fragment, for messages. CDN redirect
/// URLs carry signed, short-lived parameters that say nothing to a reader.
fn without_query(url: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    url[..end].to_owned()
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// SHA-256 of a file, as lowercase hex.
pub(crate) fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => hasher.update(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(hex(&hasher.finalize()))
}
