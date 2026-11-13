# 014: Settings file

Status: done · PRD: [04](../prds/04-storage.md) · Lane: store · Depends on: 013 · Owns: `crates/mneme-store/`

## Goal

Persist the few app settings across restarts.

## Scope

1. **`Settings`**: `default_model_id` (none until first download), `language` (default `en`; v1 is English only, decided 2026-11-12), `global_shortcut` (none by default), `meetings_root` (default `~/Documents/Mneme`).
2. **`SettingsStore`** over a file path (`settings.json` in the app data folder; the path is passed in, this crate does not know about Tauri). Load, update with a partial patch, save through the safe write helper.
3. Missing file gives defaults. A corrupt file gives defaults and is kept aside as `settings.json.bad` rather than overwritten silently. Unknown keys are kept on save.
4. Serialized field names match the frontend contract in PRD 05 (`defaultModelId`, `language`, `globalShortcut`).

## Acceptance criteria

- [x] Tests: defaults when missing; patch updates only named fields; values survive reopen; corrupt file falls back and is preserved; unknown keys survive.
- [x] `cargo test -p mneme-store` and clippy pass.

## Manual checks

None.

## Result

Done 2026-11-12. New module `crates/mneme-store/src/settings.rs`, tests in `crates/mneme-store/tests/settings.rs`, one new variant in `error.rs`, exports in `lib.rs`. No new dependency.

Scope change during the issue (from the user, via the orchestrator): Mneme v1 is English only, so the default `language` is `en`, not `auto`. It is still a plain string with no validation here; `auto` can still be stored.

### Public API

```rust
pub const SETTINGS_FILE: &str = "settings.json";
pub const DEFAULT_LANGUAGE: &str = "en";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]   // camelCase
pub struct Settings {
    pub default_model_id: Option<String>,
    pub language: String,                // never empty
    pub global_shortcut: Option<String>,
    pub meetings_root: PathBuf,          // always absolute and UTF-8
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]   // + Deserialize: value = Set, null = Clear
pub enum Patch<T> { #[default] Keep, Clear, Set(T) }

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]   // camelCase, deny_unknown_fields, every field #[serde(default)]
pub struct SettingsPatch {
    pub default_model_id: Patch<String>,
    pub language: Patch<String>,
    pub global_shortcut: Patch<String>,
    pub meetings_root: Patch<PathBuf>,
}

impl SettingsStore {      // Send + Sync, one Mutex inside
    pub fn open(path: impl Into<PathBuf>, default_meetings_root: impl Into<PathBuf>)
        -> Result<(SettingsStore, Vec<SettingsWarning>)>;
    pub fn path(&self) -> &Path;
    pub fn get(&self) -> Settings;
    pub fn update(&self, patch: &SettingsPatch) -> Result<Settings>;
}

#[non_exhaustive]
pub enum SettingsWarning {            // Display is one log line
    Corrupt { path: PathBuf, reason: String, kept_as: PathBuf },
    CorruptNotMoved { path: PathBuf, reason: String, error: io::Error },
    InvalidValue { key: &'static str, reason: String },
    RemovedTempFile { path: PathBuf },
}

// added to the existing #[non_exhaustive] Error
Error::InvalidSetting { key: &'static str, reason: String }
```

### The file as written

After setting all four (printed by the test `the_file_uses_the_names_of_the_frontend_contract`):

```json
{
  "defaultModelId": "large-v3-turbo",
  "globalShortcut": "CmdOrCtrl+Shift+M",
  "language": "en",
  "meetingsRoot": "/Users/ana/Notes/Meetings"
}
```

Two-space indent, trailing newline, written through `safe_write`. Keys come out in alphabetical order because `serde_json` is built without `preserve_order`; nothing depends on the order.

### Decisions

- **The file holds only what was set.** A missing key, `null` or an empty string means "not set" and the setting has its default (no model, `en`, no shortcut, the default root). `Clear` removes the key. A fresh install that picked a model has just `{"defaultModelId": "..."}`.
- **Three-way patch.** `Patch::Keep` / `Clear` / `Set(v)` per field. `SettingsPatch` deserializes straight from the frontend's `Partial<Settings>`: absent key = `Keep`, `null` = `Clear`, value = `Set`. `Clear` means "back to the default" for all four fields. An unknown or wrongly typed key in a deserialized patch is an error.
- **A save touches only the keys the patch names.** Unknown keys, nested ones included, are written back. They are kept as JSON values, not as text: spacing, key order and number spelling (`1.50` becomes `1.5`) are not preserved.
- **A patch that changes nothing writes nothing**, so it does not create the file. Opening never creates anything. The first real save creates the app data folder if it is missing.
- **Corrupt file** (not JSON, not a JSON object, empty, not UTF-8, unreadable, or a folder in its place): every setting has its default and the file is renamed to `settings.json.bad`; `open` returns `SettingsWarning::Corrupt` with the reason and the new name.
- **`.bad` already exists:** it is never replaced. The next free name is used: `settings.json.bad.2`, `.bad.3`, up to `.bad.999`.
- **The file cannot be moved aside** (read-only folder, or all 999 names taken): `open` still succeeds with defaults and returns `CorruptNotMoved`. The store then refuses to save over the file: `update` retries the move first and returns `Error::Io` until it works.
- **One field of the wrong type** (`"language": 5`): keep the good fields. That one setting has its default, `open` returns `InvalidValue { key, reason }`, nothing is moved or rewritten, and the bad value stays in the file until that setting is set or cleared. A newer version may have written it, so it is not thrown away. The warning repeats on every launch until then.
- **Meetings root default:** the caller passes it to `open`. It must be absolute and UTF-8 or `open` fails with `Error::InvalidSetting`. It is never written to the file, so a changed default is followed until the user picks a root. This crate never looks up the home folder and never expands `~`: a relative `meetingsRoot` in the file (including `~/Documents/Mneme`) is an `InvalidValue` warning and the default is used; in a patch it is `Error::InvalidSetting` and nothing changes.
- **Language:** any text, trimmed. Empty or blank text is not stored: in a patch it counts as `Clear`, so the language becomes `en`; in the file it reads as `en` without a warning. The same rule applies to the model id and the shortcut (blank = none). The meetings root is not trimmed; an empty one counts as `Clear`.
- **Failed update:** nothing changes, in the file or in what `get()` returns.
- **Threads:** `update` holds the store's lock across apply and save, so concurrent updates run one at a time and none is lost.
- **Leftover temp file:** `open` deletes `.settings.json.<pid>-<n>.tmp` files next to the settings file (matched with `is_temp_file_of`) and reports each as `RemovedTempFile`.
- No file watching: the file is read once at `open`. An edit made by hand while the app runs is replaced by the next save.

### Verification

- `cargo test -p mneme-store`: 283 passed, 0 failed, 0 ignored (103 unit, 179 integration, 1 doctest). The 256 earlier tests still pass; 27 are new: 25 in `tests/settings.rs`, 1 unit test for the `.bad` naming, 1 doctest on `SettingsPatch`.
- `cargo clippy -p mneme-store --all-targets -- -D warnings`: clean.
- `cargo fmt -p mneme-store -- --check`: clean.
- `RUSTDOCFLAGS="-D warnings" cargo doc -p mneme-store --no-deps`: clean.
- Acceptance tests by name: defaults when missing (`a_missing_file_gives_the_defaults_and_is_not_created`); patch updates only named fields (`a_patch_changes_only_the_fields_it_names`, `keep_clear_and_set_are_three_different_things`, `a_patch_from_the_frontend_tells_null_from_absent`); values survive reopen (`values_survive_a_reopen`); corrupt file falls back and is preserved (`a_corrupt_file_gives_the_defaults_and_is_kept_as_bad`, `every_kind_of_unusable_file_is_set_aside_unchanged`, `an_earlier_bad_file_is_never_replaced`); unknown keys survive (`unknown_keys_survive_a_save`).
- Targeted breakage check: 13 deliberate breakages of `settings.rs`, one at a time, each failed at least one test (absent key read as clear; null read as keep; clear treated as keep; unknown keys dropped; nested unknown values dropped; corrupt file not moved; existing `.bad` overwritten; saving over an unmoved corrupt file; no-op patch writing; text not trimmed; relative root accepted; one bad value making the whole file corrupt; memory updated after a failed save). The source was restored afterwards. This ran before the default language changed from `auto` to `en`; that change touched one constant, doc text and test values, and the full test run above is from after it.

### Not verified

- Nothing was run against a real app data folder or through Tauri; every test uses a temp folder.
- The two read-only-folder tests return early when run as root. They ran for real here.
- Behavior with `serde_json`'s `preserve_order` feature (key order in the file would change, nothing else should) was not run.
- Hard-linked or symlinked `settings.json`: a save replaces the link with a plain file, as `safe_write` does everywhere. Not tested.

### Follow-ups

For 021:

- Open one store at launch and keep it: `SettingsStore::open(app_data_dir.join(SETTINGS_FILE), documents_dir.join("Mneme"))`. Take both folders from Tauri's path API so they are absolute; `open` fails only on a relative or non-UTF-8 default root. Log every returned warning (`Display` is one line).
- `updateSettings` can take `SettingsPatch` as the command argument and return `Settings` as is. `Settings` serializes `meetingsRoot` as an extra field the frontend type does not declare; wrap it if that is unwanted. The patch also accepts `meetingsRoot`.
- This store checks nothing about meaning. Before calling `update`: refuse a `defaultModelId` that is not installed (the mock does), parse the language with the transcribe crate, register the shortcut. Clear `defaultModelId` when that model is deleted (the mock does).
- A `default_model_id` read from the file may name a model that is no longer installed; check before starting a meeting.
- A changed `meetings_root` means a new `Store` plus `recover()` (from 013). `update` does not create or check the folder.
- An `update` error leaves settings unchanged; show `Error`'s message.

For other lanes:

- The default language is now `en` (PRD 04 already says so). The mock in `src/lib/api/mock.ts` still starts with `language: "auto"`, and this issue's Scope line still says `auto`.
- `issues/README.md` still lists 014 as `in progress`.
