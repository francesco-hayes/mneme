//! The fixed list of Whisper models Mneme knows.
//!
//! Mneme v1 transcribes English only, so the app offers one model per size:
//! the English-only (`*.en`) variant where Whisper has one, and the
//! multilingual model for the two Large sizes, which have none. The
//! multilingual Tiny, Base, Small and Medium models were offered once and are
//! now [retired](ModelSpec::retired): still in the catalog, so a copy that is
//! already installed keeps working, but no longer listed for download.
//!
//! Every file is a ggml model in the `ggerganov/whisper.cpp` repository on
//! Hugging Face. Sizes and SHA-256 checksums were read from Hugging Face on
//! 2026-11-12 and never typed from memory:
//!
//! - `https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/<revision>`
//!   lists each file with the size and SHA-256 (`lfs.oid`) of its LFS object.
//! - A `HEAD` request on each download URL below repeats both values in the
//!   `x-linked-size` and `x-linked-etag` headers. All ten entries agreed.
//!
//! The download URLs name a commit, not `main`, so a later change to the
//! repository cannot make a checksum here stale. To move to a newer revision,
//! read the values again from the two sources above and update
//! [`MODEL_REVISION`] and the table together.

use std::sync::LazyLock;

/// The Hugging Face host the catalog downloads from.
pub const MODEL_HOST: &str = "huggingface.co";

/// The Hugging Face repository the catalog's files live in.
pub const MODEL_REPOSITORY: &str = "ggerganov/whisper.cpp";

/// The commit of [`MODEL_REPOSITORY`] every catalog URL is pinned to. It was
/// the head of `main` on 2026-11-12 (last modified 2024-10-29).
pub const MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

/// One model the app can download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Stable identifier, for example `base` or `tiny.en`. This is the id the
    /// settings file and the UI use.
    pub id: String,
    /// Name shown to the user, for example `Tiny (English)`.
    pub name: String,
    /// File name inside the models folder, for example `ggml-tiny.en.bin`.
    pub file_name: String,
    /// Where the file is downloaded from. Always HTTPS.
    pub url: String,
    /// Exact size of the file in bytes.
    pub size_bytes: u64,
    /// SHA-256 of the file: 64 lowercase hex characters.
    pub sha256: String,
    /// True for `*.en` models, which transcribe English only.
    pub english_only: bool,
    /// True for a model the app no longer offers. It stays in the catalog so
    /// an installed copy can still be found, used, verified and deleted;
    /// [`ModelStore::list`](super::ModelStore::list) leaves it out unless it
    /// is installed. The development tools can still download it.
    pub retired: bool,
    /// One line on speed and quality relative to the other models. These are
    /// orderings by model size, not measurements.
    pub note: String,
}

/// The catalog: the standard Whisper sizes and their English-only variants,
/// smallest file first. It includes the [retired](ModelSpec::retired) models.
pub fn catalog() -> &'static [ModelSpec] {
    &CATALOG
}

/// The catalog entry with this id.
pub fn find_model(id: &str) -> Option<&'static ModelSpec> {
    catalog().iter().find(|spec| spec.id == id)
}

static CATALOG: LazyLock<Vec<ModelSpec>> =
    LazyLock::new(|| ENTRIES.iter().map(Entry::to_spec).collect());

struct Entry {
    id: &'static str,
    name: &'static str,
    size_bytes: u64,
    sha256: &'static str,
    note: &'static str,
    retired: bool,
}

impl Entry {
    fn to_spec(&self) -> ModelSpec {
        let file_name = format!("ggml-{}.bin", self.id);
        ModelSpec {
            id: self.id.to_owned(),
            name: self.name.to_owned(),
            url: format!(
                "https://{MODEL_HOST}/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{file_name}"
            ),
            file_name,
            size_bytes: self.size_bytes,
            sha256: self.sha256.to_owned(),
            english_only: self.id.ends_with(".en"),
            retired: self.retired,
            note: self.note.to_owned(),
        }
    }
}

const fn entry(
    id: &'static str,
    name: &'static str,
    size_bytes: u64,
    sha256: &'static str,
    note: &'static str,
) -> Entry {
    Entry {
        id,
        name,
        size_bytes,
        sha256,
        note,
        retired: false,
    }
}

/// An [`entry`] the app no longer offers (see [`ModelSpec::retired`]).
const fn retired(
    id: &'static str,
    name: &'static str,
    size_bytes: u64,
    sha256: &'static str,
    note: &'static str,
) -> Entry {
    Entry {
        id,
        name,
        size_bytes,
        sha256,
        note,
        retired: true,
    }
}

/// Sizes and checksums as Hugging Face lists them at [`MODEL_REVISION`]. The
/// file of each entry is `ggml-<id>.bin`.
#[rustfmt::skip]
const ENTRIES: &[Entry] = &[
    retired("tiny", "Tiny", 77_691_713, "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        "Fastest and smallest. Least accurate."),
    entry("tiny.en", "Tiny (English)", 77_704_715, "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        "Fastest and smallest. Least accurate. English only."),
    retired("base", "Base", 147_951_465, "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        "Fast. More accurate than Tiny."),
    entry("base.en", "Base (English)", 147_964_211, "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        "Fast. More accurate than Tiny. English only."),
    retired("small", "Small", 487_601_967, "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        "Balanced speed and accuracy."),
    entry("small.en", "Small (English)", 487_614_201, "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        "Balanced speed and accuracy. English only."),
    retired("medium", "Medium", 1_533_763_059, "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        "More accurate than Small. Slower."),
    entry("medium.en", "Medium (English)", 1_533_774_781, "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356",
        "More accurate than Small. Slower. English only."),
    entry("large-v3-turbo", "Large v3 Turbo", 1_624_555_275, "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        "Nearly as accurate as Large v3, and faster."),
    entry("large-v3", "Large v3", 3_095_033_483, "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        "Most accurate. Slowest and largest."),
];
