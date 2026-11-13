//! The model catalog and the model store's folder handling, through the
//! public API. None of these tests uses the network, except the ignored one
//! at the end. The downloader's own tests are in `src/models/tests.rs`,
//! against an HTTP server inside the test process.

use std::collections::HashSet;
use std::fs::{self, File};
use std::path::Path;

use mneme_transcribe::{
    DownloadState, MODEL_HOST, MODEL_REPOSITORY, MODEL_REVISION, ModelError, ModelSpec, ModelStore,
    catalog, find_model, installed_model_path,
};
use tempfile::TempDir;

/// A small catalog for folder tests. Nothing is downloaded from these URLs.
fn test_catalog() -> Vec<ModelSpec> {
    let entry = |id: &str, size_bytes: u64| ModelSpec {
        id: id.to_owned(),
        name: format!("Test {id}"),
        file_name: format!("ggml-{id}.bin"),
        url: format!("https://example.invalid/ggml-{id}.bin"),
        size_bytes,
        sha256: "0".repeat(64),
        english_only: id.ends_with(".en"),
        note: format!("Note for {id}."),
    };
    vec![entry("one", 10), entry("two.en", 20), entry("three", 30)]
}

/// Writes a file of `len` bytes.
fn write_file(path: &Path, len: u64) {
    File::create(path).unwrap().set_len(len).unwrap();
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn every_catalog_entry_is_https_on_the_expected_host_with_a_full_checksum() {
    assert_eq!(MODEL_HOST, "huggingface.co");
    assert_eq!(MODEL_REPOSITORY, "ggerganov/whisper.cpp");
    // Pinned to a commit, never to a branch that can move.
    assert_eq!(MODEL_REVISION.len(), 40);
    assert!(is_lower_hex(MODEL_REVISION));

    let models = catalog();
    assert!(!models.is_empty());
    for model in models {
        let id = &model.id;
        assert_eq!(model.file_name, format!("ggml-{id}.bin"), "{id}");
        assert_eq!(
            model.url,
            format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/{MODEL_REVISION}/{}",
                model.file_name
            ),
            "{id}"
        );
        assert_eq!(model.sha256.len(), 64, "{id}");
        assert!(is_lower_hex(&model.sha256), "{id}");
        // Every ggml model is tens of megabytes at least.
        assert!(model.size_bytes > 10_000_000, "{id}");
        assert_eq!(model.english_only, id.ends_with(".en"), "{id}");
        assert_eq!(
            model.english_only,
            model.name.ends_with("(English)"),
            "{id}"
        );
        assert!(!model.name.is_empty() && !model.note.is_empty(), "{id}");
        assert_eq!(
            model.english_only,
            model.note.contains("English only"),
            "{id}"
        );
        assert_eq!(find_model(id), Some(model));
    }

    let unique = |values: Vec<&str>| values.iter().collect::<HashSet<_>>().len() == values.len();
    assert!(unique(models.iter().map(|m| m.id.as_str()).collect()));
    assert!(unique(models.iter().map(|m| m.name.as_str()).collect()));
    assert!(unique(
        models.iter().map(|m| m.file_name.as_str()).collect()
    ));
    assert!(unique(models.iter().map(|m| m.sha256.as_str()).collect()));
    assert!(models.is_sorted_by_key(|m| m.size_bytes));
}

#[test]
fn the_catalog_covers_the_standard_sizes() {
    let ids: Vec<&str> = catalog().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "tiny",
            "tiny.en",
            "base",
            "base.en",
            "small",
            "small.en",
            "medium",
            "medium.en",
            "large-v3-turbo",
            "large-v3",
        ]
    );
    assert_eq!(find_model("large"), None);

    // The entry issue 005 downloaded and hashed on this machine.
    let tiny_en = find_model("tiny.en").unwrap();
    assert_eq!(tiny_en.name, "Tiny (English)");
    assert_eq!(tiny_en.size_bytes, 77_704_715);
    assert_eq!(
        tiny_en.sha256,
        "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f"
    );
}

#[test]
fn a_new_store_creates_its_folder_and_lists_nothing_installed() {
    let parent = TempDir::new().unwrap();
    let dir = parent.path().join("app-data").join("models");
    let store = ModelStore::open(&dir).unwrap();

    assert!(dir.is_dir());
    assert_eq!(store.dir(), dir);
    let listed = store.list();
    assert_eq!(listed.len(), catalog().len());
    for (info, spec) in listed.iter().zip(catalog()) {
        assert_eq!(info.id, spec.id);
        assert_eq!(info.name, spec.name);
        assert_eq!(info.size_bytes, spec.size_bytes);
        assert_eq!(info.note, spec.note);
        assert_eq!(info.english_only, spec.english_only);
        assert!(!info.installed);
    }
    assert!(store.active_downloads().is_empty());
}

#[test]
fn list_marks_a_model_installed_only_by_its_final_name_and_size() {
    let dir = TempDir::new().unwrap();
    let store = ModelStore::open_with_catalog(dir.path(), test_catalog()).unwrap();
    let installed = |store: &ModelStore| -> Vec<bool> {
        store.list().iter().map(|model| model.installed).collect()
    };
    assert_eq!(installed(&store), [false, false, false]);

    // The right name with the right size.
    write_file(&dir.path().join("ggml-one.bin"), 10);
    // A download in progress: the right size, but still under its .part name.
    write_file(&dir.path().join("ggml-two.en.bin.part"), 20);
    // The right name with the wrong size.
    write_file(&dir.path().join("ggml-three.bin"), 29);
    // Not a file.
    fs::create_dir(dir.path().join("ggml-unrelated.bin")).unwrap();

    assert_eq!(installed(&store), [true, false, false]);
    assert!(store.is_installed("one"));
    assert!(!store.is_installed("two.en"));
    assert!(!store.is_installed("three"));
    assert!(!store.is_installed("no-such-model"));

    assert_eq!(
        store.installed_path("one").unwrap(),
        dir.path().join("ggml-one.bin")
    );
    assert_eq!(
        store.installed_path("two.en"),
        Err(ModelError::NotInstalled {
            id: "two.en".to_owned(),
            name: "Test two.en".to_owned(),
        })
    );
    assert_eq!(
        store.installed_path("no-such-model"),
        Err(ModelError::UnknownModel {
            id: "no-such-model".to_owned(),
        })
    );

    let listed = store.list();
    assert_eq!(listed[1].id, "two.en");
    assert_eq!(listed[1].name, "Test two.en");
    assert_eq!(listed[1].size_bytes, 20);
    assert_eq!(listed[1].note, "Note for two.en.");
    assert!(listed[1].english_only);
    assert!(!listed[0].english_only);
}

#[test]
fn delete_removes_the_model_and_says_when_there_is_none() {
    let dir = TempDir::new().unwrap();
    let store = ModelStore::open_with_catalog(dir.path(), test_catalog()).unwrap();
    write_file(&dir.path().join("ggml-one.bin"), 10);
    write_file(&dir.path().join("ggml-two.en.bin"), 20);
    // Wrong size: not installed, but still a file with the model's name.
    write_file(&dir.path().join("ggml-three.bin"), 5);

    store.delete("one").unwrap();
    assert!(!store.is_installed("one"));
    assert!(store.is_installed("two.en"));
    assert_eq!(files_in(dir.path()), ["ggml-three.bin", "ggml-two.en.bin"]);

    // Deleting what is not there is an error that names the model, and
    // changes nothing.
    let error = store.delete("one").unwrap_err();
    assert_eq!(
        error,
        ModelError::NotInstalled {
            id: "one".to_owned(),
            name: "Test one".to_owned(),
        }
    );
    assert_eq!(error.to_string(), "Test one is not installed.");
    assert_eq!(
        store.delete("no-such-model"),
        Err(ModelError::UnknownModel {
            id: "no-such-model".to_owned(),
        })
    );
    assert_eq!(files_in(dir.path()), ["ggml-three.bin", "ggml-two.en.bin"]);

    store.delete("three").unwrap();
    assert_eq!(files_in(dir.path()), ["ggml-two.en.bin"]);
}

#[test]
fn opening_a_store_removes_leftover_part_files_and_nothing_else() {
    let dir = TempDir::new().unwrap();
    write_file(&dir.path().join("ggml-one.bin"), 10);
    write_file(&dir.path().join("ggml-two.en.bin.part"), 7);
    // Left by a catalog entry that no longer exists.
    write_file(&dir.path().join("ggml-retired.bin.part"), 3);
    write_file(&dir.path().join("notes.txt"), 1);
    write_file(&dir.path().join("partial"), 1);
    fs::create_dir(dir.path().join("folder.part")).unwrap();
    write_file(&dir.path().join("folder.part").join("inner.part"), 1);

    let store = ModelStore::open_with_catalog(dir.path(), test_catalog()).unwrap();

    assert_eq!(
        files_in(dir.path()),
        ["folder.part", "ggml-one.bin", "notes.txt", "partial"]
    );
    assert_eq!(files_in(&dir.path().join("folder.part")), ["inner.part"]);
    assert!(store.is_installed("one"));
    assert!(!store.is_installed("two.en"));
}

#[test]
fn a_catalog_that_could_not_be_stored_safely_is_rejected() {
    let dir = TempDir::new().unwrap();
    let open = |change: &dyn Fn(&mut Vec<ModelSpec>)| {
        let mut catalog = test_catalog();
        change(&mut catalog);
        ModelStore::open_with_catalog(dir.path(), catalog).map(|_| ())
    };
    let rejected = |change: &dyn Fn(&mut Vec<ModelSpec>)| {
        matches!(open(change), Err(ModelError::InvalidCatalog { .. }))
    };

    assert_eq!(open(&|_| {}), Ok(()));
    assert!(rejected(&|c| c[1].id = "one".to_owned()));
    assert!(rejected(&|c| c[1].file_name = "ggml-one.bin".to_owned()));
    assert!(rejected(&|c| c[0].id = String::new()));
    assert!(rejected(&|c| c[0].file_name = String::new()));
    assert!(rejected(&|c| c[0].file_name = "../outside.bin".to_owned()));
    assert!(rejected(&|c| c[0].file_name = "sub/model.bin".to_owned()));
    assert!(rejected(&|c| c[0].file_name = "..".to_owned()));
    assert!(rejected(&|c| c[0].file_name = "model.bin.part".to_owned()));
    assert!(rejected(&|c| c[0].sha256 = "abc123".to_owned()));
    assert!(rejected(&|c| c[0].sha256 = "g".repeat(64)));
}

#[test]
fn the_store_can_be_shared_between_threads() {
    fn assert_send_sync<T: Send + Sync + Clone + 'static>() {}
    assert_send_sync::<ModelStore>();
}

#[test]
fn installed_model_path_finds_a_model_without_touching_the_folder() {
    let dir = TempDir::new().unwrap();
    let spec = find_model("tiny").unwrap();
    // A sparse file of the catalog's size stands in for the model.
    write_file(&dir.path().join(&spec.file_name), spec.size_bytes);
    // An unfinished download and a stray file, which opening a store on the
    // folder would delete or could trip over.
    write_file(&dir.path().join("ggml-base.bin.part"), 5);
    write_file(&dir.path().join("notes.txt"), 5);
    let before = files_in(dir.path());

    assert_eq!(
        installed_model_path(dir.path(), "tiny"),
        Ok(dir.path().join("ggml-tiny.bin"))
    );
    // The same answer a store gives, minus the store's clean-up.
    assert_eq!(files_in(dir.path()), before);
    assert!(dir.path().join("ggml-base.bin.part").exists());
}

#[test]
fn installed_model_path_applies_the_installed_rule() {
    let dir = TempDir::new().unwrap();
    let not_installed = |id: &str, name: &str| {
        Err(ModelError::NotInstalled {
            id: id.to_owned(),
            name: name.to_owned(),
        })
    };

    // Nothing there.
    assert_eq!(
        installed_model_path(dir.path(), "base"),
        not_installed("base", "Base")
    );
    // Only a download in progress.
    write_file(&dir.path().join("ggml-base.bin.part"), 147_951_465);
    assert_eq!(
        installed_model_path(dir.path(), "base"),
        not_installed("base", "Base")
    );
    // The right name with the wrong size.
    write_file(&dir.path().join("ggml-base.bin"), 147_951_464);
    assert_eq!(
        installed_model_path(dir.path(), "base"),
        not_installed("base", "Base")
    );
    // A folder with the model's name.
    fs::create_dir(dir.path().join("ggml-tiny.en.bin")).unwrap();
    assert_eq!(
        installed_model_path(dir.path(), "tiny.en"),
        not_installed("tiny.en", "Tiny (English)")
    );
    // An id the catalog does not have.
    assert_eq!(
        installed_model_path(dir.path(), "enormous"),
        Err(ModelError::UnknownModel {
            id: "enormous".to_owned()
        })
    );

    // A folder that does not exist is not created.
    let missing = dir.path().join("never-made");
    assert_eq!(
        installed_model_path(&missing, "base"),
        not_installed("base", "Base")
    );
    assert!(!missing.exists());
}

/// Downloads the smallest real model from Hugging Face (about 78 MB) into a
/// temp folder and verifies it. Needs the network, so it only runs on
/// request:
///
/// ```text
/// cargo test -p mneme-transcribe --test models -- --ignored --nocapture
/// ```
#[test]
#[ignore = "downloads about 78 MB from Hugging Face"]
fn the_real_tiny_en_model_downloads_and_verifies() {
    let dir = TempDir::new().unwrap();
    let store = ModelStore::open(dir.path()).unwrap();
    let spec = find_model("tiny.en").unwrap();
    println!("downloading {}", spec.url);

    let (sender, events) = std::sync::mpsc::channel();
    let started = std::time::Instant::now();
    let download = store
        .start_download("tiny.en", move |progress| {
            let _ = sender.send(progress);
        })
        .unwrap();
    let path = download.wait().expect("the real download succeeds");
    let elapsed = started.elapsed();
    let events: Vec<_> = events.try_iter().collect();

    assert_eq!(path, dir.path().join("ggml-tiny.en.bin"));
    assert_eq!(fs::metadata(&path).unwrap().len(), spec.size_bytes);
    assert_eq!(files_in(dir.path()), ["ggml-tiny.en.bin"]);
    assert!(store.is_installed("tiny.en"));
    // Reads the file back from disk and hashes it again.
    store
        .verify("tiny.en")
        .expect("the file on disk has the catalog's SHA-256");

    let states: Vec<DownloadState> = events.iter().map(|event| event.state).collect();
    assert_eq!(states.first(), Some(&DownloadState::Downloading));
    assert_eq!(
        states[states.len() - 3..],
        [
            DownloadState::Downloading,
            DownloadState::Verifying,
            DownloadState::Done
        ]
    );
    assert!(
        events
            .iter()
            .all(|event| event.total_bytes == spec.size_bytes)
    );
    assert_eq!(events.last().unwrap().received_bytes, spec.size_bytes);
    println!(
        "downloaded and verified {} bytes in {:.1} s, {} progress events",
        spec.size_bytes,
        elapsed.as_secs_f64(),
        events.len()
    );

    store.delete("tiny.en").unwrap();
    assert!(!store.is_installed("tiny.en"));
}
