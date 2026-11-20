//! Downloader tests. They run against [`TestServer`], an HTTP server inside
//! the test process on 127.0.0.1, with small made-up "model" files and a test
//! catalog, so none of them uses the network.
//!
//! The server is plain HTTP, which the downloader refuses. These tests open
//! their stores with a policy that also accepts `http://127.0.0.1`; that
//! policy can only be built in `#[cfg(test)]` code of this crate.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::catalog::{ModelSpec, find_model};
use super::download::{
    DownloadProgress, DownloadState, MAX_REDIRECTS, NetPolicy, checked_url, hex, part_path,
    redirect_target, sha256_file,
};
use super::error::ModelError;
use super::store::ModelStore;
use super::test_server::{Reply, TestServer};

/// How long a test waits for something that should happen at once.
const SOON: Duration = Duration::from_secs(10);

/// The production rules, plus plain HTTP to the test server. The read timeout
/// is long on purpose: a test that ends quickly did not end by timing out.
fn test_policy() -> NetPolicy {
    NetPolicy {
        allow_plain_http_to_loopback: true,
        resolve_timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(5),
        response_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(60),
    }
}

/// Deterministic bytes that do not compress or repeat (xorshift).
fn fake_model(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed.max(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as u8
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A catalog entry for `body`, downloaded from `url`.
fn spec_for(id: &str, url: String, body: &[u8]) -> ModelSpec {
    ModelSpec {
        id: id.to_owned(),
        name: format!("Test {id}"),
        file_name: format!("ggml-{id}.bin"),
        url,
        size_bytes: body.len() as u64,
        sha256: sha256_hex(body),
        english_only: false,
        retired: false,
        note: "A test file.".to_owned(),
    }
}

fn open_store(dir: &TempDir, catalog: Vec<ModelSpec>) -> ModelStore {
    ModelStore::open_with_policy(dir.path(), catalog, test_policy()).expect("the store opens")
}

/// A progress callback and the receiver its events arrive on.
fn recorder() -> (
    impl FnMut(DownloadProgress) + Send + 'static,
    Receiver<DownloadProgress>,
) {
    let (sender, receiver) = mpsc::channel();
    (
        move |progress| {
            let _ = sender.send(progress);
        },
        receiver,
    )
}

/// Runs a download to its end and returns its result and every event.
fn run_download(
    store: &ModelStore,
    id: &str,
) -> (
    Result<std::path::PathBuf, ModelError>,
    Vec<DownloadProgress>,
) {
    let (callback, events) = recorder();
    let handle = store
        .start_download(id, callback)
        .expect("the download starts");
    let result = handle.wait();
    (result, events.try_iter().collect())
}

/// The names of the files in the models folder, sorted.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the models folder is readable")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn states(events: &[DownloadProgress]) -> Vec<DownloadState> {
    events.iter().map(|event| event.state).collect()
}

/// Checks the rules every event sequence follows, whatever the outcome, and
/// returns the last event.
fn check_events<'a>(events: &'a [DownloadProgress], spec: &ModelSpec) -> &'a DownloadProgress {
    let first = events.first().expect("at least one event");
    assert_eq!(first.state, DownloadState::Downloading);
    assert_eq!(first.received_bytes, 0);

    let mut previous = 0;
    for event in events {
        assert_eq!(event.model_id, spec.id);
        assert_eq!(event.total_bytes, spec.size_bytes);
        assert!(event.received_bytes >= previous, "progress went backwards");
        assert!(event.received_bytes <= event.total_bytes);
        assert_eq!(event.error.is_some(), event.state == DownloadState::Error);
        previous = event.received_bytes;
    }

    let last = events.last().unwrap();
    assert!(last.state.is_terminal(), "last event was {}", last.state);
    let terminal = events.iter().filter(|e| e.state.is_terminal()).count();
    assert_eq!(terminal, 1, "exactly one final event: {:?}", states(events));
    last
}

/// Waits until the `.part` file of `spec` holds exactly `len` bytes.
fn wait_for_part(dir: &Path, spec: &ModelSpec, len: u64) {
    let part = part_path(dir, spec);
    let deadline = Instant::now() + SOON;
    while fs::metadata(&part).map(|m| m.len()).ok() != Some(len) {
        assert!(
            Instant::now() < deadline,
            "the .part file never reached {len} bytes"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_download_is_verified_and_installed() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(200_000, 1);
    server.route("/model.bin", Reply::Ok(body.clone()));
    let spec = spec_for("good", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);
    assert!(!store.is_installed("good"));

    let (result, events) = run_download(&store, "good");

    let path = result.expect("the download succeeds");
    assert_eq!(path, dir.path().join("ggml-good.bin"));
    assert_eq!(fs::read(&path).unwrap(), body);
    assert_eq!(files_in(dir.path()), ["ggml-good.bin"]);
    assert!(store.is_installed("good"));
    assert!(store.list()[0].installed);
    assert_eq!(store.installed_path("good").unwrap(), path);
    store
        .verify("good")
        .expect("the installed file has the catalog's checksum");
    assert!(!store.is_downloading("good"));

    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Done);
    assert_eq!(last.received_bytes, spec.size_bytes);
    // Even a download too quick for a second progress tick reports every
    // byte as `downloading` before it moves on to `verifying`.
    assert_eq!(events[events.len() - 3].received_bytes, spec.size_bytes);
    let tail = &states(&events)[events.len() - 3..];
    assert_eq!(
        tail,
        [
            DownloadState::Downloading,
            DownloadState::Verifying,
            DownloadState::Done
        ]
    );
    assert_eq!(server.requests(), ["/model.bin"]);
}

#[test]
fn progress_reaches_the_total() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(300_000, 2);
    server.route(
        "/model.bin",
        Reply::Trickle {
            body: body.clone(),
            chunk: 10_000,
            delay: Duration::from_millis(15),
        },
    );
    let spec = spec_for("slow", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (result, events) = run_download(&store, "slow");

    result.expect("the download succeeds");
    check_events(&events, &spec);
    let downloading: Vec<u64> = events
        .iter()
        .filter(|event| event.state == DownloadState::Downloading)
        .map(|event| event.received_bytes)
        .collect();
    assert_eq!(downloading.first(), Some(&0));
    assert_eq!(downloading.last(), Some(&spec.size_bytes));
    assert!(
        downloading
            .iter()
            .any(|&bytes| bytes > 0 && bytes < spec.size_bytes),
        "no event between 0 and the total: {downloading:?}"
    );
    // Verifying and done both report every byte.
    for event in events
        .iter()
        .filter(|e| e.state != DownloadState::Downloading)
    {
        assert_eq!(event.received_bytes, spec.size_bytes);
    }
}

#[test]
fn a_checksum_mismatch_installs_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(50_000, 3);
    // Same size as the catalog says, different bytes.
    let mut tampered = body.clone();
    tampered[25_000] ^= 0x01;
    server.route("/model.bin", Reply::Ok(tampered.clone()));
    let spec = spec_for("tampered", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (result, events) = run_download(&store, "tampered");

    assert_eq!(
        result,
        Err(ModelError::ChecksumMismatch {
            expected: sha256_hex(&body),
            actual: sha256_hex(&tampered),
        })
    );
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(!store.is_installed("tampered"));
    assert!(!store.is_downloading("tampered"));

    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Error);
    assert!(last.error.as_deref().unwrap().contains("checksum"));
    assert!(states(&events).contains(&DownloadState::Verifying));
    assert!(!states(&events).contains(&DownloadState::Done));
}

#[test]
fn cancel_while_the_connection_is_stalled_leaves_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(256 * 1024, 4);
    let pause_after = 100 * 1024;
    server.route(
        "/model.bin",
        Reply::Gated {
            body: body.clone(),
            pause_after,
        },
    );
    let spec = spec_for("stalled", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (callback, events) = recorder();
    let handle = store.start_download("stalled", callback).unwrap();
    // The server has sent part of the file and now sends nothing: the
    // download is blocked in a read, with a `.part` file on disk.
    wait_for_part(dir.path(), &spec, pause_after as u64);
    assert_eq!(files_in(dir.path()), ["ggml-stalled.bin.part"]);
    assert!(!store.is_installed("stalled"));
    assert!(store.is_downloading("stalled"));
    // Give the download time to get from writing those bytes to waiting for
    // the next ones, so the cancel has to interrupt a read and cannot be
    // noticed between two reads.
    std::thread::sleep(Duration::from_millis(300));

    let cancelled_at = Instant::now();
    assert!(store.cancel_download("stalled"));
    let result = handle.wait();

    assert_eq!(result, Err(ModelError::Cancelled));
    // Far below the 60 s read timeout: the cancel interrupted the read.
    assert!(cancelled_at.elapsed() < SOON);
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(!store.is_installed("stalled"));
    assert!(!store.is_downloading("stalled"));
    assert!(!store.cancel_download("stalled"));

    let events: Vec<_> = events.try_iter().collect();
    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Cancelled);
    assert_eq!(last.received_bytes, pause_after as u64);
    assert!(!states(&events).contains(&DownloadState::Verifying));
}

#[test]
fn cancel_while_bytes_are_arriving_leaves_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(2_000_000, 5);
    server.route(
        "/model.bin",
        Reply::Trickle {
            body: body.clone(),
            chunk: 4_000,
            delay: Duration::from_millis(10),
        },
    );
    let spec = spec_for("flowing", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (callback, events) = recorder();
    let handle = store.start_download("flowing", callback).unwrap();
    let mut seen = Vec::new();
    loop {
        let event = events
            .recv_timeout(SOON)
            .expect("progress before the cancel");
        let received = event.received_bytes;
        seen.push(event);
        if received > 0 {
            break;
        }
    }
    handle.cancel();
    let result = handle.wait();

    assert_eq!(result, Err(ModelError::Cancelled));
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(!store.is_installed("flowing"));
    assert!(!store.is_downloading("flowing"));

    seen.extend(events.try_iter());
    let last = check_events(&seen, &spec);
    assert_eq!(last.state, DownloadState::Cancelled);
    assert!(last.received_bytes > 0 && last.received_bytes < spec.size_bytes);

    // The same model can be downloaded afterwards.
    server.route("/model.bin", Reply::Ok(body.clone()));
    let (result, _) = run_download(&store, "flowing");
    result.expect("a download after a cancel succeeds");
    assert!(store.is_installed("flowing"));
}

#[test]
fn a_server_error_is_reported() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(10_000, 6);
    server.route("/broken.bin", Reply::Status(500));
    let broken = spec_for("broken", server.url("/broken.bin"), &body);
    // No route: the server answers 404.
    let missing = spec_for("missing", server.url("/missing.bin?token=secret"), &body);
    let store = open_store(&dir, vec![broken.clone(), missing.clone()]);

    let (result, events) = run_download(&store, "broken");
    assert_eq!(
        result,
        Err(ModelError::HttpStatus {
            status: 500,
            url: server.url("/broken.bin"),
        })
    );
    let last = check_events(&events, &broken);
    assert_eq!(last.state, DownloadState::Error);
    assert!(last.error.as_deref().unwrap().contains("500"));
    assert_eq!(last.received_bytes, 0);

    let (result, events) = run_download(&store, "missing");
    // The query string is left out of the message.
    assert_eq!(
        result,
        Err(ModelError::HttpStatus {
            status: 404,
            url: server.url("/missing.bin"),
        })
    );
    assert_eq!(check_events(&events, &missing).state, DownloadState::Error);

    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(store.list().iter().all(|model| !model.installed));
    assert!(store.active_downloads().is_empty());
}

#[test]
fn a_refused_connection_is_reported() {
    let dir = TempDir::new().unwrap();
    // A port nothing listens on: bind one, note it, close it.
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let body = fake_model(1_000, 7);
    let spec = spec_for(
        "offline",
        format!("http://127.0.0.1:{port}/model.bin"),
        &body,
    );
    let store = open_store(&dir, vec![spec.clone()]);

    let (result, events) = run_download(&store, "offline");

    assert!(
        matches!(result, Err(ModelError::Network { .. })),
        "got {result:?}"
    );
    assert_eq!(check_events(&events, &spec).state, DownloadState::Error);
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
}

#[test]
fn a_wrong_declared_size_fails_before_anything_is_written() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(40_000, 8);
    let longer = fake_model(40_001, 8);
    server.route("/model.bin", Reply::Ok(longer));
    let spec = spec_for("resized", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (result, events) = run_download(&store, "resized");

    assert_eq!(
        result,
        Err(ModelError::SizeMismatch {
            expected: 40_000,
            actual: 40_001,
        })
    );
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Error);
    assert_eq!(last.received_bytes, 0);
}

#[test]
fn a_body_of_the_wrong_length_installs_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(150_000, 9);
    let mut too_long = body.clone();
    too_long.extend_from_slice(&fake_model(150_000, 10));
    // No Content-Length, so only counting the bytes can tell.
    server.route("/long.bin", Reply::NoLength(too_long));
    server.route("/short.bin", Reply::NoLength(body[..100_000].to_vec()));
    // Declares the right length, then the connection drops early.
    server.route(
        "/dropped.bin",
        Reply::DeclaredLength {
            body: body[..100_000].to_vec(),
            declared: 150_000,
        },
    );
    let long = spec_for("long", server.url("/long.bin"), &body);
    let short = spec_for("short", server.url("/short.bin"), &body);
    let dropped = spec_for("dropped", server.url("/dropped.bin"), &body);
    let store = open_store(&dir, vec![long.clone(), short.clone(), dropped.clone()]);

    let (result, events) = run_download(&store, "long");
    assert!(
        matches!(
            result,
            Err(ModelError::SizeMismatch { expected: 150_000, actual }) if actual > 150_000
        ),
        "got {result:?}"
    );
    assert_eq!(check_events(&events, &long).state, DownloadState::Error);

    let (result, events) = run_download(&store, "short");
    assert_eq!(
        result,
        Err(ModelError::SizeMismatch {
            expected: 150_000,
            actual: 100_000,
        })
    );
    assert_eq!(check_events(&events, &short).state, DownloadState::Error);

    let (result, events) = run_download(&store, "dropped");
    assert!(
        matches!(result, Err(ModelError::Network { .. })),
        "got {result:?}"
    );
    assert_eq!(check_events(&events, &dropped).state, DownloadState::Error);

    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(store.list().iter().all(|model| !model.installed));
}

#[test]
fn a_stalled_download_times_out_and_leaves_nothing() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(100_000, 11);
    // Never released: the server goes quiet after 30,000 bytes.
    server.route(
        "/model.bin",
        Reply::Gated {
            body: body.clone(),
            pause_after: 30_000,
        },
    );
    let spec = spec_for("quiet", server.url("/model.bin"), &body);
    let policy = NetPolicy {
        read_timeout: Duration::from_millis(300),
        ..test_policy()
    };
    let store = ModelStore::open_with_policy(dir.path(), vec![spec.clone()], policy).unwrap();

    let started = Instant::now();
    let (result, events) = run_download(&store, "quiet");

    assert!(
        matches!(result, Err(ModelError::Network { .. })),
        "got {result:?}"
    );
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < SOON);
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Error);
    assert_eq!(last.received_bytes, 30_000);
}

#[test]
fn redirects_are_followed() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(20_000, 12);
    server.route("/file.bin", Reply::Ok(body.clone()));
    server.route("/absolute", Reply::Redirect(server.url("/file.bin")));
    server.route("/relative", Reply::Redirect("/hop".to_owned()));
    server.route("/hop", Reply::Redirect("/file.bin".to_owned()));
    let absolute = spec_for("absolute", server.url("/absolute"), &body);
    let relative = spec_for("relative", server.url("/relative"), &body);
    let store = open_store(&dir, vec![absolute, relative]);

    let (result, _) = run_download(&store, "absolute");
    result.expect("an absolute redirect is followed");
    let (result, _) = run_download(&store, "relative");
    result.expect("two relative redirects are followed");

    assert_eq!(
        server.requests(),
        ["/absolute", "/file.bin", "/relative", "/hop", "/file.bin"]
    );
    assert_eq!(
        files_in(dir.path()),
        ["ggml-absolute.bin", "ggml-relative.bin"]
    );
}

#[test]
fn a_redirect_to_plain_http_is_refused_before_anything_is_sent_to_it() {
    let server = TestServer::start();
    // Where the redirect points. `localhost` is not the literal address the
    // test policy accepts, so to the downloader this is any plain HTTP host;
    // it resolves to this machine, so a request would arrive if one were made.
    let target = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(20_000, 13);
    target.route("/file.bin", Reply::Ok(body.clone()));
    let insecure = target.url("/file.bin").replace("127.0.0.1", "localhost");
    server.route("/downgrade", Reply::Redirect(format!("{insecure}?sig=abc")));
    let spec = spec_for("downgrade", server.url("/downgrade"), &body);
    let store = open_store(&dir, vec![spec.clone()]);

    let (result, events) = run_download(&store, "downgrade");

    assert_eq!(result, Err(ModelError::InsecureUrl { url: insecure }));
    assert_eq!(server.requests(), ["/downgrade"]);
    assert_eq!(target.requests(), [] as [&str; 0]);
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    let last = check_events(&events, &spec);
    assert_eq!(last.state, DownloadState::Error);
    assert!(last.error.as_deref().unwrap().contains("not HTTPS"));
}

#[test]
fn a_store_opened_the_public_way_refuses_plain_http() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(20_000, 14);
    server.route("/file.bin", Reply::Ok(body.clone()));
    let spec = spec_for("plain", server.url("/file.bin"), &body);
    // The production policy: no exception for the test server.
    let store = ModelStore::open_with_catalog(dir.path(), vec![spec.clone()]).unwrap();

    let (result, events) = run_download(&store, "plain");

    assert_eq!(
        result,
        Err(ModelError::InsecureUrl {
            url: server.url("/file.bin")
        })
    );
    assert_eq!(server.requests(), [] as [&str; 0]);
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert_eq!(check_events(&events, &spec).state, DownloadState::Error);
}

#[test]
fn bad_redirects_are_reported() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(1_000, 15);
    server.route("/loop", Reply::Redirect("/loop".to_owned()));
    server.route("/nowhere", Reply::RedirectWithoutLocation);
    let looping = spec_for("loop", server.url("/loop"), &body);
    let nowhere = spec_for("nowhere", server.url("/nowhere"), &body);
    let store = open_store(&dir, vec![looping, nowhere]);

    let (result, _) = run_download(&store, "loop");
    assert_eq!(
        result,
        Err(ModelError::TooManyRedirects {
            limit: MAX_REDIRECTS
        })
    );
    // The first request and five redirects.
    assert_eq!(server.requests().len(), MAX_REDIRECTS as usize + 1);

    let (result, _) = run_download(&store, "nowhere");
    assert!(
        matches!(result, Err(ModelError::BadRedirect { .. })),
        "got {result:?}"
    );
    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
}

#[test]
fn a_second_download_of_the_same_model_is_refused() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(60_000, 16);
    server.route(
        "/model.bin",
        Reply::Gated {
            body: body.clone(),
            pause_after: 20_000,
        },
    );
    let other_body = fake_model(5_000, 17);
    server.route("/other.bin", Reply::Ok(other_body.clone()));
    let spec = spec_for("twice", server.url("/model.bin"), &body);
    let other = spec_for("other", server.url("/other.bin"), &other_body);
    let store = open_store(&dir, vec![spec.clone(), other]);

    let (callback, events) = recorder();
    let first = store.start_download("twice", callback).unwrap();
    wait_for_part(dir.path(), &spec, 20_000);

    let (second_callback, second_events) = recorder();
    let second = store.start_download("twice", second_callback);
    assert_eq!(
        second.unwrap_err(),
        ModelError::AlreadyDownloading {
            id: "twice".to_owned(),
            name: "Test twice".to_owned(),
        }
    );
    assert_eq!(second_events.try_iter().count(), 0);
    assert_eq!(store.active_downloads(), ["twice"]);
    // The first download's file was not touched by the refusal.
    assert_eq!(files_in(dir.path()), ["ggml-twice.bin.part"]);

    // Another model downloads alongside.
    let (result, _) = run_download(&store, "other");
    result.expect("a different model downloads at the same time");

    server.release();
    first.wait().expect("the first download finishes");
    let events: Vec<_> = events.try_iter().collect();
    assert_eq!(check_events(&events, &spec).state, DownloadState::Done);
    assert_eq!(fs::read(dir.path().join("ggml-twice.bin")).unwrap(), body);
    assert!(store.active_downloads().is_empty());

    // Installed now, so a third attempt is refused for another reason.
    let (callback, _) = recorder();
    assert_eq!(
        store.start_download("twice", callback).unwrap_err(),
        ModelError::AlreadyInstalled {
            id: "twice".to_owned(),
            name: "Test twice".to_owned(),
        }
    );
    assert_eq!(server.requests(), ["/model.bin", "/other.bin"]);
}

#[test]
fn an_unknown_model_cannot_be_downloaded() {
    let dir = TempDir::new().unwrap();
    let store = open_store(&dir, Vec::new());
    let (callback, events) = recorder();
    assert_eq!(
        store.start_download("nope", callback).unwrap_err(),
        ModelError::UnknownModel {
            id: "nope".to_owned()
        }
    );
    assert_eq!(events.try_iter().count(), 0);
    assert!(!store.cancel_download("nope"));
}

#[test]
fn a_file_of_the_wrong_size_is_not_installed_and_a_download_replaces_it() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(30_000, 18);
    server.route("/model.bin", Reply::Ok(body.clone()));
    let spec = spec_for("partial", server.url("/model.bin"), &body);
    fs::write(dir.path().join("ggml-partial.bin"), &body[..29_999]).unwrap();
    let store = open_store(&dir, vec![spec]);

    assert!(!store.is_installed("partial"));
    assert!(matches!(
        store.installed_path("partial"),
        Err(ModelError::NotInstalled { .. })
    ));

    let (result, _) = run_download(&store, "partial");
    result.expect("the download replaces the bad file");
    assert_eq!(fs::read(dir.path().join("ggml-partial.bin")).unwrap(), body);
    assert!(store.is_installed("partial"));
}

#[test]
fn verify_catches_a_file_changed_after_it_was_installed() {
    let dir = TempDir::new().unwrap();
    let body = fake_model(30_000, 19);
    let spec = spec_for(
        "rotted",
        "https://example.invalid/model.bin".to_owned(),
        &body,
    );
    let mut changed = body.clone();
    changed[0] ^= 0xff;
    fs::write(dir.path().join("ggml-rotted.bin"), &changed).unwrap();
    let store = open_store(&dir, vec![spec]);

    // Right name and size, so it lists as installed; only the hash can tell.
    assert!(store.is_installed("rotted"));
    assert_eq!(
        store.verify("rotted"),
        Err(ModelError::ChecksumMismatch {
            expected: sha256_hex(&body),
            actual: sha256_hex(&changed),
        })
    );
}

#[test]
fn a_panic_in_the_progress_callback_still_cleans_up() {
    let server = TestServer::start();
    let dir = TempDir::new().unwrap();
    let body = fake_model(30_000, 20);
    server.route("/model.bin", Reply::Ok(body.clone()));
    let spec = spec_for("panics", server.url("/model.bin"), &body);
    let store = open_store(&dir, vec![spec]);

    let handle = store
        .start_download("panics", |progress| {
            if progress.state == DownloadState::Verifying {
                // Unwinds like a panic without printing a message.
                std::panic::resume_unwind(Box::new("callback failed"));
            }
        })
        .unwrap();
    let deadline = Instant::now() + SOON;
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "the download never ended");
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(files_in(dir.path()), [] as [&str; 0]);
    assert!(!store.is_installed("panics"));
    assert!(!store.is_downloading("panics"));
}

#[test]
fn only_https_urls_pass_the_production_policy() {
    let production = NetPolicy::PRODUCTION;
    assert!(!production.allow_plain_http_to_loopback);

    for url in [
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        "https://us.aws.cdn.hf.co/some/object?Signature=abc",
        "HTTPS://huggingface.co/file",
    ] {
        assert_eq!(checked_url(url, &production), Ok(url.to_owned()), "{url}");
    }
    for url in [
        "http://huggingface.co/file",
        "http://127.0.0.1:8080/file",
        "http://localhost/file",
        "ftp://huggingface.co/file",
        "file:///etc/hosts",
        "huggingface.co/file",
        "/file",
        "",
    ] {
        assert!(
            matches!(
                checked_url(url, &production),
                Err(ModelError::InsecureUrl { .. })
            ),
            "{url:?} was accepted"
        );
    }

    // The test policy adds exactly one thing: plain HTTP to 127.0.0.1.
    let test = test_policy();
    assert!(checked_url("http://127.0.0.1:8080/file", &test).is_ok());
    assert!(checked_url("https://huggingface.co/file", &test).is_ok());
    for url in [
        "http://huggingface.co/file",
        "http://localhost:8080/file",
        "http://127.0.0.2/file",
        "http://[::1]/file",
    ] {
        assert!(checked_url(url, &test).is_err(), "{url:?} was accepted");
    }
}

#[test]
fn redirect_targets_are_resolved_and_checked() {
    let production = NetPolicy::PRODUCTION;
    let from = "https://huggingface.co/ggerganov/whisper.cpp/resolve/abc/ggml-tiny.bin";

    assert_eq!(
        redirect_target(
            from,
            "https://us.aws.cdn.hf.co/object?Signature=abc",
            &production
        ),
        Ok("https://us.aws.cdn.hf.co/object?Signature=abc".to_owned())
    );
    assert_eq!(
        redirect_target(from, "/api/resolve-cache/file?x=1", &production),
        Ok("https://huggingface.co/api/resolve-cache/file?x=1".to_owned())
    );
    assert_eq!(
        redirect_target("https://example.com:8443/a", "/b", &production),
        Ok("https://example.com:8443/b".to_owned())
    );
    assert_eq!(
        redirect_target(from, "//cdn.example.com/object", &production),
        Ok("https://cdn.example.com/object".to_owned())
    );

    // HTTPS never continues over plain HTTP, wherever it points.
    for location in [
        "http://us.aws.cdn.hf.co/object?Signature=abc",
        "http://huggingface.co/ggerganov/whisper.cpp/resolve/abc/ggml-tiny.bin",
        "http://127.0.0.1:8080/object",
        "ftp://us.aws.cdn.hf.co/object",
    ] {
        assert!(
            matches!(
                redirect_target(from, location, &production),
                Err(ModelError::InsecureUrl { .. })
            ),
            "{location:?} was followed"
        );
    }
    // The message leaves out the signed query string.
    assert_eq!(
        redirect_target(
            from,
            "http://cdn.example.com/object?Signature=abc",
            &production
        ),
        Err(ModelError::InsecureUrl {
            url: "http://cdn.example.com/object".to_owned()
        })
    );
    assert!(matches!(
        redirect_target(from, "other-file.bin", &production),
        Err(ModelError::BadRedirect { .. })
    ));
}

#[test]
fn download_states_use_the_frontend_contract_names() {
    let names: Vec<&str> = [
        DownloadState::Downloading,
        DownloadState::Verifying,
        DownloadState::Done,
        DownloadState::Cancelled,
        DownloadState::Error,
    ]
    .iter()
    .map(|state| state.as_str())
    .collect();
    assert_eq!(
        names,
        ["downloading", "verifying", "done", "cancelled", "error"]
    );
    assert_eq!(DownloadState::Verifying.to_string(), "verifying");
    assert!(!DownloadState::Downloading.is_terminal());
    assert!(!DownloadState::Verifying.is_terminal());
}

#[test]
fn sha256_matches_the_published_test_vectors() {
    // FIPS 180-2 examples: the empty message and "abc".
    let dir = TempDir::new().unwrap();
    let empty = dir.path().join("empty");
    let abc = dir.path().join("abc");
    fs::File::create(&empty).unwrap();
    fs::File::create(&abc).unwrap().write_all(b"abc").unwrap();

    assert_eq!(
        sha256_file(&empty).unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_file(&abc).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
}

/// The one catalog entry that can be checked against a real file without the
/// network: the test model issue 005 put in `models/`. Skips when it is
/// absent.
#[test]
fn the_local_tiny_en_model_matches_its_catalog_entry() {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/models/ggml-tiny.en.bin"
    ));
    if !path.is_file() {
        let _ = writeln!(
            std::io::stderr(),
            "SKIPPED the_local_tiny_en_model_matches_its_catalog_entry: no {}",
            path.display()
        );
        return;
    }
    let spec = find_model("tiny.en").expect("tiny.en is in the catalog");
    assert_eq!(fs::metadata(path).unwrap().len(), spec.size_bytes);
    assert_eq!(sha256_file(path).unwrap(), spec.sha256);
}
