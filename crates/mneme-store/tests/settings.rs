//! The settings file through the public API, in temporary folders.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use mneme_store::{
    DEFAULT_LANGUAGE, Error, Patch, SETTINGS_FILE, Settings, SettingsPatch, SettingsStore,
    SettingsWarning, safe_write,
};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The default meetings root the tests pass in. Nothing is ever created
/// there: the settings store only carries the path.
const DEFAULT_ROOT: &str = "/default/Documents/Mneme";

/// A temp folder standing in for the app data folder, and the path of the
/// settings file in it. The file is not created.
fn data_dir() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_FILE);
    (dir, path)
}

fn open(path: &Path) -> (SettingsStore, Vec<SettingsWarning>) {
    SettingsStore::open(path, DEFAULT_ROOT).unwrap()
}

/// Opens a store on a file that must be in good order.
fn open_clean(path: &Path) -> SettingsStore {
    let (store, warnings) = open(path);
    assert!(warnings.is_empty(), "{warnings:?}");
    store
}

fn defaults() -> Settings {
    Settings {
        default_model_id: None,
        language: "en".to_owned(),
        global_shortcut: None,
        meetings_root: PathBuf::from(DEFAULT_ROOT),
    }
}

fn set(text: &str) -> Patch<String> {
    Patch::Set(text.to_owned())
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn json_of(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

// Defaults

#[test]
fn a_missing_file_gives_the_defaults_and_is_not_created() {
    let (dir, path) = data_dir();

    let store = open_clean(&path);

    assert_eq!(store.get(), defaults());
    assert_eq!(store.get().language, DEFAULT_LANGUAGE);
    assert_eq!(store.path(), path);
    assert!(names_in(dir.path()).is_empty());
}

#[test]
fn a_missing_folder_gives_the_defaults_and_is_created_by_the_first_save() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir
        .path()
        .join("Application Support")
        .join("app.mneme.desktop");
    let path = folder.join(SETTINGS_FILE);

    let store = open_clean(&path);
    assert_eq!(store.get(), defaults());
    assert!(names_in(dir.path()).is_empty());

    store
        .update(&SettingsPatch {
            language: set("pt"),
            ..SettingsPatch::default()
        })
        .unwrap();

    assert_eq!(names_in(&folder), [SETTINGS_FILE]);
    assert_eq!(json_of(&path), json!({"language": "pt"}));
}

#[test]
fn a_patch_that_changes_nothing_writes_nothing() {
    let (dir, path) = data_dir();
    let store = open_clean(&path);

    let unchanged = store.update(&SettingsPatch::default()).unwrap();
    assert_eq!(unchanged, defaults());
    // Clearing what is not set, or setting an empty value, is no change.
    store
        .update(&SettingsPatch {
            default_model_id: Patch::Clear,
            language: set("  "),
            global_shortcut: set(""),
            meetings_root: Patch::Clear,
        })
        .unwrap();

    assert!(names_in(dir.path()).is_empty());

    // With a file in place: the same, down to its bytes.
    let by_hand = "{\"language\":   \"en\"}";
    fs::write(&path, by_hand).unwrap();
    let store = open_clean(&path);
    store.update(&SettingsPatch::default()).unwrap();
    store
        .update(&SettingsPatch {
            language: set("en"),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), by_hand);
}

// The file as written

#[test]
fn the_file_uses_the_names_of_the_frontend_contract() {
    let (dir, path) = data_dir();
    let store = open_clean(&path);

    let settings = store
        .update(&SettingsPatch {
            default_model_id: set("large-v3-turbo"),
            language: set("en"),
            global_shortcut: set("CmdOrCtrl+Shift+M"),
            meetings_root: Patch::Set(PathBuf::from("/Users/ana/Notes/Meetings")),
        })
        .unwrap();

    let expected = json!({
        "defaultModelId": "large-v3-turbo",
        "language": "en",
        "globalShortcut": "CmdOrCtrl+Shift+M",
        "meetingsRoot": "/Users/ana/Notes/Meetings",
    });
    assert_eq!(json_of(&path), expected);
    // What a Tauri command returns to the frontend has the same names.
    assert_eq!(serde_json::to_value(&settings).unwrap(), expected);
    assert_eq!(
        serde_json::to_value(defaults()).unwrap(),
        json!({
            "defaultModelId": null,
            "language": "en",
            "globalShortcut": null,
            "meetingsRoot": DEFAULT_ROOT,
        })
    );

    // Indented, one key per line, ending in a newline, and no temp file.
    let text = fs::read_to_string(&path).unwrap();
    println!("{text}");
    assert_eq!(
        text,
        serde_json::to_string_pretty(&json_of(&path)).unwrap() + "\n"
    );
    assert_eq!(text.lines().count(), 6, "{text}");
    assert!(text.contains("\n  \"language\": \"en\""), "{text}");
    assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);
}

#[test]
fn only_what_was_set_is_in_the_file() {
    let (_dir, path) = data_dir();
    let store = open_clean(&path);

    store
        .update(&SettingsPatch {
            default_model_id: set("base.en"),
            ..SettingsPatch::default()
        })
        .unwrap();

    // The default meetings root in particular is never frozen into the
    // file: a store opened with another default follows it.
    assert_eq!(json_of(&path), json!({"defaultModelId": "base.en"}));
    let (moved_home, warnings) = SettingsStore::open(&path, "/Volumes/New/Mneme").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        moved_home.get().meetings_root,
        PathBuf::from("/Volumes/New/Mneme")
    );
}

// Patches

#[test]
fn a_patch_changes_only_the_fields_it_names() {
    let (_dir, path) = data_dir();
    let store = open_clean(&path);
    let mut expected = defaults();

    let after = store
        .update(&SettingsPatch {
            language: set("de"),
            ..SettingsPatch::default()
        })
        .unwrap();
    expected.language = "de".to_owned();
    assert_eq!(after, expected);
    assert_eq!(json_of(&path), json!({"language": "de"}));

    let after = store
        .update(&SettingsPatch {
            default_model_id: set("small"),
            ..SettingsPatch::default()
        })
        .unwrap();
    expected.default_model_id = Some("small".to_owned());
    assert_eq!(after, expected);

    let after = store
        .update(&SettingsPatch {
            global_shortcut: set("Alt+Space"),
            ..SettingsPatch::default()
        })
        .unwrap();
    expected.global_shortcut = Some("Alt+Space".to_owned());
    assert_eq!(after, expected);

    let after = store
        .update(&SettingsPatch {
            meetings_root: Patch::Set(PathBuf::from("/Volumes/Work/Meetings")),
            ..SettingsPatch::default()
        })
        .unwrap();
    expected.meetings_root = PathBuf::from("/Volumes/Work/Meetings");
    assert_eq!(after, expected);

    // Two at once; the other two stay.
    let after = store
        .update(&SettingsPatch {
            default_model_id: set("medium"),
            language: set("auto"),
            ..SettingsPatch::default()
        })
        .unwrap();
    expected.default_model_id = Some("medium".to_owned());
    expected.language = "auto".to_owned();
    assert_eq!(after, expected);
    assert_eq!(store.get(), expected);
    assert_eq!(
        json_of(&path),
        json!({
            "defaultModelId": "medium",
            "language": "auto",
            "globalShortcut": "Alt+Space",
            "meetingsRoot": "/Volumes/Work/Meetings",
        })
    );
}

#[test]
fn keep_clear_and_set_are_three_different_things() {
    let (_dir, path) = data_dir();
    let store = open_clean(&path);
    let all_set = SettingsPatch {
        default_model_id: set("small"),
        language: set("fr"),
        global_shortcut: set("Alt+Space"),
        meetings_root: Patch::Set(PathBuf::from("/Volumes/Work/Meetings")),
    };
    let after_all_set = store.update(&all_set).unwrap();
    assert_eq!(after_all_set.global_shortcut.as_deref(), Some("Alt+Space"));

    // Keep: the default patch leaves all four as they are.
    assert_eq!(
        store.update(&SettingsPatch::default()).unwrap(),
        after_all_set
    );

    // Clear, one field at a time; each time the other three stay.
    let cleared = store
        .update(&SettingsPatch {
            global_shortcut: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        cleared,
        Settings {
            global_shortcut: None,
            ..after_all_set.clone()
        }
    );
    assert_eq!(open_clean(&path).get(), cleared);

    store.update(&all_set).unwrap();
    let cleared = store
        .update(&SettingsPatch {
            default_model_id: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        cleared,
        Settings {
            default_model_id: None,
            ..after_all_set.clone()
        }
    );
    assert_eq!(open_clean(&path).get(), cleared);

    store.update(&all_set).unwrap();
    let cleared = store
        .update(&SettingsPatch {
            language: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        cleared,
        Settings {
            language: "en".to_owned(),
            ..after_all_set.clone()
        }
    );
    assert_eq!(open_clean(&path).get(), cleared);

    store.update(&all_set).unwrap();
    let cleared = store
        .update(&SettingsPatch {
            meetings_root: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        cleared,
        Settings {
            meetings_root: PathBuf::from(DEFAULT_ROOT),
            ..after_all_set.clone()
        }
    );
    assert_eq!(open_clean(&path).get(), cleared);

    // Set again after a clear.
    assert_eq!(store.update(&all_set).unwrap(), after_all_set);
}

#[test]
fn a_patch_from_the_frontend_tells_null_from_absent() {
    let parse = |text: &str| serde_json::from_str::<SettingsPatch>(text).unwrap();

    assert_eq!(parse("{}"), SettingsPatch::default());
    assert_eq!(
        parse(r#"{"globalShortcut": null}"#),
        SettingsPatch {
            global_shortcut: Patch::Clear,
            ..SettingsPatch::default()
        }
    );
    assert_eq!(
        parse(r#"{"globalShortcut": "Alt+Space"}"#),
        SettingsPatch {
            global_shortcut: set("Alt+Space"),
            ..SettingsPatch::default()
        }
    );
    assert_eq!(
        parse(
            r#"{"defaultModelId": null, "language": "en", "globalShortcut": "F5",
                "meetingsRoot": "/Volumes/Work"}"#
        ),
        SettingsPatch {
            default_model_id: Patch::Clear,
            language: set("en"),
            global_shortcut: set("F5"),
            meetings_root: Patch::Set(PathBuf::from("/Volumes/Work")),
        }
    );
    assert_eq!(
        parse(r#"{"defaultModelId": "small", "language": null, "meetingsRoot": null}"#),
        SettingsPatch {
            default_model_id: set("small"),
            language: Patch::Clear,
            global_shortcut: Patch::Keep,
            meetings_root: Patch::Clear,
        }
    );

    // A misspelled or wrongly typed key is an error, not a silent no-op.
    for bad in [
        r#"{"globalshortcut": null}"#,
        r#"{"default_model_id": "small"}"#,
        r#"{"language": 5}"#,
        r#"{"globalShortcut": ["Alt", "Space"]}"#,
        r#"{"language": "en", "theme": "dark"}"#,
        "null",
        "\"en\"",
    ] {
        assert!(serde_json::from_str::<SettingsPatch>(bad).is_err(), "{bad}");
    }

    // End to end: the frontend clears the shortcut and nothing else.
    let (_dir, path) = data_dir();
    let store = open_clean(&path);
    store
        .update(&parse(
            r#"{"defaultModelId": "small", "globalShortcut": "Alt+Space"}"#,
        ))
        .unwrap();
    let after = store.update(&parse(r#"{"globalShortcut": null}"#)).unwrap();
    assert_eq!(after.global_shortcut, None);
    assert_eq!(after.default_model_id.as_deref(), Some("small"));
    let after = store.update(&parse(r#"{"language": "es"}"#)).unwrap();
    assert_eq!(after.global_shortcut, None);
    assert_eq!(after.default_model_id.as_deref(), Some("small"));
    assert_eq!(after.language, "es");
    assert_eq!(
        json_of(&path),
        json!({"defaultModelId": "small", "language": "es"})
    );
}

#[test]
fn empty_text_is_never_stored() {
    let (_dir, path) = data_dir();
    let store = open_clean(&path);
    store
        .update(&SettingsPatch {
            default_model_id: set("small"),
            language: set("fr"),
            global_shortcut: set("Alt+Space"),
            ..SettingsPatch::default()
        })
        .unwrap();

    // An empty language is the default language, not an empty string.
    let after = store
        .update(&SettingsPatch {
            language: set(""),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after.language, "en");
    assert_eq!(
        json_of(&path),
        json!({"defaultModelId": "small", "globalShortcut": "Alt+Space"})
    );

    // Empty or blank text in a field that can be none means none.
    let after = store
        .update(&SettingsPatch {
            default_model_id: set(" \t"),
            global_shortcut: set(""),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after, defaults());
    assert_eq!(json_of(&path), json!({}));

    // Text is stored trimmed.
    let after = store
        .update(&SettingsPatch {
            default_model_id: set(" small "),
            language: set(" de\n"),
            global_shortcut: set("\tAlt+Space "),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after.default_model_id.as_deref(), Some("small"));
    assert_eq!(after.language, "de");
    assert_eq!(after.global_shortcut.as_deref(), Some("Alt+Space"));
    assert_eq!(
        json_of(&path),
        json!({"defaultModelId": "small", "language": "de", "globalShortcut": "Alt+Space"})
    );

    // The language is any text here: checking it is the transcriber's job.
    let after = store
        .update(&SettingsPatch {
            language: set("Klingon"),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after.language, "Klingon");

    // Empty and null values written by hand read as not set, quietly.
    write_json(
        &path,
        &json!({"defaultModelId": "", "language": "  ", "globalShortcut": null,
                "meetingsRoot": ""}),
    );
    assert_eq!(open_clean(&path).get(), defaults());
    write_json(&path, &json!({"language": null, "meetingsRoot": null}));
    assert_eq!(open_clean(&path).get(), defaults());
}

// Reopening

#[test]
fn values_survive_a_reopen() {
    let (_dir, path) = data_dir();
    let saved = open_clean(&path)
        .update(&SettingsPatch {
            default_model_id: set("large-v3-turbo"),
            language: set("it"),
            global_shortcut: set("CmdOrCtrl+Shift+M"),
            meetings_root: Patch::Set(PathBuf::from("/Volumes/Work/Réunions")),
        })
        .unwrap();

    let reopened = open_clean(&path);

    assert_eq!(reopened.get(), saved);
    assert_eq!(
        reopened.get(),
        Settings {
            default_model_id: Some("large-v3-turbo".to_owned()),
            language: "it".to_owned(),
            global_shortcut: Some("CmdOrCtrl+Shift+M".to_owned()),
            meetings_root: PathBuf::from("/Volumes/Work/Réunions"),
        }
    );

    // And a change made after the reopen builds on them.
    reopened
        .update(&SettingsPatch {
            language: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        open_clean(&path).get(),
        Settings {
            language: "en".to_owned(),
            ..saved
        }
    );
}

// Unknown keys

#[test]
fn unknown_keys_survive_a_save() {
    let (_dir, path) = data_dir();
    let from_a_newer_version = json!({
        "language": "de",
        "theme": "dark",
        "audio": {
            "inputDevice": "MacBook Pro Microphone",
            "gain": 1.5,
            "channels": [0, 1],
            "echoCancel": {"enabled": true, "tail": null},
        },
        "recentModels": ["small", {"id": "medium", "pinned": false}],
        "launchCount": 42,
        "big": 18446744073709551615u64,
        "negative": -7,
        "ratio": 0.1,
        "nothing": null,
        "": "an empty key",
        "DefaultModelId": "another key: names are case-sensitive",
    });
    write_json(&path, &from_a_newer_version);

    let store = open_clean(&path);
    assert_eq!(store.get().language, "de");
    assert_eq!(store.get().default_model_id, None);
    store
        .update(&SettingsPatch {
            default_model_id: set("small"),
            global_shortcut: set("F5"),
            ..SettingsPatch::default()
        })
        .unwrap();

    let mut expected = from_a_newer_version.clone();
    expected["defaultModelId"] = json!("small");
    expected["globalShortcut"] = json!("F5");
    assert_eq!(json_of(&path), expected);

    // Again after a reopen, and through a clear.
    let store = open_clean(&path);
    store
        .update(&SettingsPatch {
            language: Patch::Clear,
            global_shortcut: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    let object = expected.as_object_mut().unwrap();
    object.remove("language");
    object.remove("globalShortcut");
    assert_eq!(json_of(&path), expected);
    assert_eq!(expected["audio"]["echoCancel"]["tail"], Value::Null);
}

// A file that cannot be used

#[test]
fn a_corrupt_file_gives_the_defaults_and_is_kept_as_bad() {
    let (dir, path) = data_dir();
    let broken = "{\"defaultModelId\": \"small\", \"language\": \"en\"";
    fs::write(&path, broken).unwrap();

    let (store, warnings) = open(&path);

    assert_eq!(store.get(), defaults());
    let bad = dir.path().join("settings.json.bad");
    assert_eq!(names_in(dir.path()), ["settings.json.bad"]);
    assert_eq!(fs::read_to_string(&bad).unwrap(), broken);
    match warnings.as_slice() {
        [
            SettingsWarning::Corrupt {
                path: warned,
                reason,
                kept_as,
            },
        ] => {
            assert_eq!(warned, &path);
            assert_eq!(kept_as, &bad);
            assert!(reason.starts_with("is not valid JSON ("), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    let line = warnings[0].to_string();
    assert!(line.starts_with(&path.display().to_string()), "{line}");
    assert!(line.contains("is not valid JSON"), "{line}");
    assert!(line.contains("\"settings.json.bad\""), "{line}");
    assert!(!line.contains('\n'), "{line}");

    // The next save writes a fresh file and leaves the bad one alone.
    store
        .update(&SettingsPatch {
            language: set("de"),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(json_of(&path), json!({"language": "de"}));
    assert_eq!(fs::read_to_string(&bad).unwrap(), broken);
    assert_eq!(names_in(dir.path()), ["settings.json", "settings.json.bad"]);
    // A reopen now finds a good file and has nothing to report.
    assert_eq!(open_clean(&path).get().language, "de");
}

#[test]
fn every_kind_of_unusable_file_is_set_aside_unchanged() {
    let cases: [(&[u8], &str); 9] = [
        (b"", "is not valid JSON"),
        (b"  \n", "is not valid JSON"),
        (b"language = en\n", "is not valid JSON"),
        (b"{\"language\": \"en\"} trailing", "is not valid JSON"),
        (b"{\"language\": \"\xff\xfe\"}", "is not valid JSON"),
        (
            b"[\"en\"]",
            "holds an array where a JSON object is expected",
        ),
        (b"\"en\"", "holds text where a JSON object is expected"),
        (b"null", "holds null where a JSON object is expected"),
        (b"42", "holds a number where a JSON object is expected"),
    ];
    for (bytes, expected_reason) in cases {
        let (dir, path) = data_dir();
        fs::write(&path, bytes).unwrap();

        let (store, warnings) = open(&path);

        let shown = String::from_utf8_lossy(bytes);
        assert_eq!(store.get(), defaults(), "{shown}");
        assert_eq!(names_in(dir.path()), ["settings.json.bad"], "{shown}");
        assert_eq!(
            fs::read(dir.path().join("settings.json.bad")).unwrap(),
            bytes,
            "{shown}"
        );
        match warnings.as_slice() {
            [SettingsWarning::Corrupt { reason, .. }] => {
                assert!(reason.starts_with(expected_reason), "{shown}: {reason}");
            }
            other => panic!("{shown}: {other:?}"),
        }
    }
}

#[test]
fn a_folder_where_the_file_should_be_is_set_aside_too() {
    let (dir, path) = data_dir();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("inside.txt"), "kept").unwrap();

    let (store, warnings) = open(&path);

    assert_eq!(store.get(), defaults());
    assert!(
        matches!(
            warnings.as_slice(),
            [SettingsWarning::Corrupt { reason, .. }] if reason.starts_with("cannot be read (")
        ),
        "{warnings:?}"
    );
    let bad = dir.path().join("settings.json.bad");
    assert_eq!(fs::read_to_string(bad.join("inside.txt")).unwrap(), "kept");
    store
        .update(&SettingsPatch {
            language: set("en"),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(json_of(&path), json!({"language": "en"}));
}

#[test]
fn an_earlier_bad_file_is_never_replaced() {
    let (dir, path) = data_dir();

    for (round, expected_name) in [
        "settings.json.bad",
        "settings.json.bad.2",
        "settings.json.bad.3",
    ]
    .into_iter()
    .enumerate()
    {
        fs::write(&path, format!("broken {round}")).unwrap();
        let (store, warnings) = open(&path);
        assert_eq!(store.get(), defaults());
        match warnings.as_slice() {
            [SettingsWarning::Corrupt { kept_as, .. }] => {
                assert_eq!(kept_as, &dir.path().join(expected_name));
            }
            other => panic!("{other:?}"),
        }
        assert!(
            warnings[0]
                .to_string()
                .contains(&format!("\"{expected_name}\"")),
            "{}",
            warnings[0]
        );
    }

    assert_eq!(
        names_in(dir.path()),
        [
            "settings.json.bad",
            "settings.json.bad.2",
            "settings.json.bad.3"
        ]
    );
    for (round, name) in names_in(dir.path()).iter().enumerate() {
        assert_eq!(
            fs::read_to_string(dir.path().join(name)).unwrap(),
            format!("broken {round}")
        );
    }
}

#[test]
fn one_bad_value_costs_only_that_setting_and_stays_in_the_file() {
    let (dir, path) = data_dir();
    let by_hand = json!({
        "defaultModelId": "small",
        "language": 5,
        "globalShortcut": "Alt+Space",
        "meetingsRoot": "/Volumes/Work/Meetings",
        "theme": "dark",
    });
    write_json(&path, &by_hand);
    let bytes_before = fs::read(&path).unwrap();

    let (store, warnings) = open(&path);

    assert_eq!(
        store.get(),
        Settings {
            default_model_id: Some("small".to_owned()),
            language: "en".to_owned(),
            global_shortcut: Some("Alt+Space".to_owned()),
            meetings_root: PathBuf::from("/Volumes/Work/Meetings"),
        }
    );
    match warnings.as_slice() {
        [SettingsWarning::InvalidValue { key, reason }] => {
            assert_eq!(*key, "language");
            assert_eq!(reason, "is a number, not text");
        }
        other => panic!("{other:?}"),
    }
    let line = warnings[0].to_string();
    assert!(
        line.starts_with("setting `language`: is a number, not text;"),
        "{line}"
    );
    // Opening changed nothing on disk and set nothing aside.
    assert_eq!(fs::read(&path).unwrap(), bytes_before);
    assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);

    // A save of another setting leaves the bad value where it is.
    let after = store
        .update(&SettingsPatch {
            global_shortcut: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after.language, "en");
    let mut expected = by_hand.clone();
    expected.as_object_mut().unwrap().remove("globalShortcut");
    assert_eq!(json_of(&path), expected);
    // So the next launch reports it again.
    let (reopened, warnings) = open(&path);
    assert!(
        matches!(
            warnings.as_slice(),
            [SettingsWarning::InvalidValue {
                key: "language",
                ..
            }]
        ),
        "{warnings:?}"
    );

    // Setting it replaces the bad value.
    let after = reopened
        .update(&SettingsPatch {
            language: set("nl"),
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(after.language, "nl");
    expected["language"] = json!("nl");
    assert_eq!(json_of(&path), expected);
    assert_eq!(open_clean(&path).get(), after);
}

#[test]
fn every_known_key_of_the_wrong_type_is_reported() {
    let (_dir, path) = data_dir();
    write_json(
        &path,
        &json!({
            "defaultModelId": ["small"],
            "language": {"code": "en"},
            "globalShortcut": true,
            "meetingsRoot": 7,
            "theme": "dark",
        }),
    );

    let (store, warnings) = open(&path);

    assert_eq!(store.get(), defaults());
    let mut reported: Vec<(String, String)> = warnings
        .iter()
        .map(|warning| match warning {
            SettingsWarning::InvalidValue { key, reason } => (key.to_string(), reason.clone()),
            other => panic!("{other:?}"),
        })
        .collect();
    reported.sort();
    let expected = [
        ("defaultModelId", "is an array, not text"),
        ("globalShortcut", "is a boolean, not text"),
        ("language", "is an object, not text"),
        ("meetingsRoot", "is a number, not text"),
    ]
    .map(|(key, reason)| (key.to_owned(), reason.to_owned()));
    assert_eq!(reported, expected);

    // Clearing a setting removes the bad value as well.
    store
        .update(&SettingsPatch {
            language: Patch::Clear,
            ..SettingsPatch::default()
        })
        .unwrap();
    assert_eq!(
        json_of(&path),
        json!({
            "defaultModelId": ["small"],
            "globalShortcut": true,
            "meetingsRoot": 7,
            "theme": "dark",
        })
    );
}

// The meetings root

#[test]
fn a_relative_meetings_root_is_never_used() {
    // In the file: the default is used and the value is reported.
    for relative in [
        "~/Documents/Mneme",
        "Documents/Mneme",
        "./Mneme",
        "~",
        " /leading/space",
    ] {
        let (_dir, path) = data_dir();
        write_json(&path, &json!({"meetingsRoot": relative, "language": "de"}));

        let (store, warnings) = open(&path);

        assert_eq!(
            store.get().meetings_root,
            PathBuf::from(DEFAULT_ROOT),
            "{relative}"
        );
        assert_eq!(store.get().language, "de", "{relative}");
        match warnings.as_slice() {
            [SettingsWarning::InvalidValue { key, reason }] => {
                assert_eq!(*key, "meetingsRoot");
                assert_eq!(
                    reason,
                    &format!("is not an absolute path: \"{relative}\" (`~` is not expanded)")
                );
            }
            other => panic!("{relative}: {other:?}"),
        }
        assert_eq!(json_of(&path)["meetingsRoot"], json!(relative));
    }

    // In a patch: refused, and nothing changes, not even the other fields
    // of the same patch.
    let (dir, path) = data_dir();
    let store = open_clean(&path);
    for relative in ["~/Documents/Mneme", "Mneme", "../Mneme"] {
        let err = store
            .update(&SettingsPatch {
                language: set("de"),
                meetings_root: Patch::Set(PathBuf::from(relative)),
                ..SettingsPatch::default()
            })
            .unwrap_err();
        match &err {
            Error::InvalidSetting { key, reason } => {
                assert_eq!(*key, "meetingsRoot");
                assert!(reason.contains(relative), "{reason}");
            }
            other => panic!("{relative}: {other:?}"),
        }
        assert_eq!(
            err.to_string(),
            format!(
                "setting `meetingsRoot`: is not an absolute path: \"{relative}\" (`~` is not expanded)"
            )
        );
        assert_eq!(store.get(), defaults(), "{relative}");
        assert!(names_in(dir.path()).is_empty(), "{relative}");
    }

    // As the default: the store does not open.
    for relative in ["~/Documents/Mneme", "Documents/Mneme", ""] {
        let err = SettingsStore::open(&path, relative).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::InvalidSetting {
                    key: "meetingsRoot",
                    ..
                }
            ),
            "{relative}: {err:?}"
        );
        assert!(
            err.to_string()
                .starts_with("setting `meetingsRoot`: the default passed in is not an absolute"),
            "{err}"
        );
    }
    assert!(names_in(dir.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn a_meetings_root_that_is_not_utf8_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let not_utf8 = PathBuf::from(OsStr::from_bytes(b"/Volumes/Work/\xff"));
    let (dir, path) = data_dir();

    let err = SettingsStore::open(&path, &not_utf8).unwrap_err();
    assert!(matches!(err, Error::InvalidSetting { .. }), "{err:?}");

    let store = open_clean(&path);
    let err = store
        .update(&SettingsPatch {
            meetings_root: Patch::Set(not_utf8),
            ..SettingsPatch::default()
        })
        .unwrap_err();
    assert!(
        matches!(&err, Error::InvalidSetting { key: "meetingsRoot", reason } if reason.starts_with("is not valid UTF-8")),
        "{err:?}"
    );
    assert!(names_in(dir.path()).is_empty());
}

#[test]
fn a_settings_path_without_a_file_name_is_refused() {
    let err = SettingsStore::open("/", DEFAULT_ROOT).unwrap_err();
    assert!(
        matches!(&err, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput),
        "{err:?}"
    );
}

// Saving

#[test]
fn a_temp_file_left_by_a_crash_is_removed_when_the_store_opens() {
    let (dir, path) = data_dir();
    write_json(&path, &json!({"language": "de"}));
    // What a save that died before its rename leaves, and files that only
    // look similar.
    fs::write(dir.path().join(".settings.json.4242-7.tmp"), "{\"langu").unwrap();
    for other in [
        ".settings.json.tmp",
        ".notes.md.4242-7.tmp",
        ".settings.json.4242-7.tmp.keep",
        "settings.json.4242-7.tmp",
    ] {
        fs::write(dir.path().join(other), "not ours").unwrap();
    }

    let (store, warnings) = open(&path);

    assert_eq!(store.get().language, "de");
    match warnings.as_slice() {
        [SettingsWarning::RemovedTempFile { path: removed }] => {
            assert_eq!(removed, &dir.path().join(".settings.json.4242-7.tmp"));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        warnings[0]
            .to_string()
            .ends_with("removed the temp file of an interrupted save"),
        "{}",
        warnings[0]
    );
    assert_eq!(
        names_in(dir.path()),
        [
            ".notes.md.4242-7.tmp",
            ".settings.json.4242-7.tmp.keep",
            ".settings.json.tmp",
            "settings.json",
            "settings.json.4242-7.tmp",
        ]
    );
}

#[cfg(unix)]
mod read_only_folder {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// Makes `dir` read-only for the life of the guard. `None` when that
    /// does not stop this process from writing there (running as root), in
    /// which case the test has nothing to check.
    struct ReadOnly<'a>(&'a Path);

    impl<'a> ReadOnly<'a> {
        fn new(dir: &'a Path) -> Option<Self> {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
            let guard = Self(dir);
            let probe = dir.join("probe");
            if fs::write(&probe, "x").is_ok() {
                fs::remove_file(&probe).unwrap();
                return None;
            }
            Some(guard)
        }
    }

    impl Drop for ReadOnly<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let (dir, path) = data_dir();
        write_json(&path, &json!({"language": "de", "theme": "dark"}));
        let bytes_before = fs::read(&path).unwrap();
        let store = open_clean(&path);
        let before = store.get();

        {
            let Some(_read_only) = ReadOnly::new(dir.path()) else {
                return;
            };
            let err = store
                .update(&SettingsPatch {
                    language: set("fr"),
                    global_shortcut: set("F5"),
                    ..SettingsPatch::default()
                })
                .unwrap_err();
            assert!(matches!(err, Error::Io { .. }), "{err:?}");
            assert_eq!(store.get(), before);
            assert_eq!(fs::read(&path).unwrap(), bytes_before);
        }

        // Once the folder can be written, the same store saves again, and
        // nothing of the failed patch leaks into the file.
        let after = store
            .update(&SettingsPatch {
                default_model_id: set("small"),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(after.language, "de");
        assert_eq!(after.global_shortcut, None);
        assert_eq!(
            json_of(&path),
            json!({"language": "de", "theme": "dark", "defaultModelId": "small"})
        );
        assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);
    }

    #[test]
    fn a_corrupt_file_that_cannot_be_moved_is_not_saved_over() {
        let (dir, path) = data_dir();
        let broken = "{\"language\": \"en\",";
        fs::write(&path, broken).unwrap();
        let patch = SettingsPatch {
            language: set("fr"),
            ..SettingsPatch::default()
        };

        let store = {
            let Some(_read_only) = ReadOnly::new(dir.path()) else {
                return;
            };
            let (store, warnings) = open(&path);
            assert_eq!(store.get(), defaults());
            match warnings.as_slice() {
                [
                    SettingsWarning::CorruptNotMoved {
                        path: warned,
                        reason,
                        error,
                    },
                ] => {
                    assert_eq!(warned, &path);
                    assert!(reason.starts_with("is not valid JSON"), "{reason}");
                    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                }
                other => panic!("{other:?}"),
            }
            let line = warnings[0].to_string();
            assert!(line.contains("could not be moved aside"), "{line}");

            let err = store.update(&patch).unwrap_err();
            assert!(
                matches!(&err, Error::Io { path: failed, .. } if failed == &path),
                "{err:?}"
            );
            assert_eq!(store.get(), defaults());
            // A patch that changes nothing has nothing to save and is fine.
            assert_eq!(store.update(&SettingsPatch::default()).unwrap(), defaults());
            store
        };
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);

        // The folder is writable again and the broken file is still there.
        // A plain save would now replace it; the store moves it aside
        // first.
        let after = store.update(&patch).unwrap();
        assert_eq!(after.language, "fr");
        assert_eq!(json_of(&path), json!({"language": "fr"}));
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json.bad")).unwrap(),
            broken
        );
        assert_eq!(names_in(dir.path()), ["settings.json", "settings.json.bad"]);
        // From here on it is an ordinary store.
        store
            .update(&SettingsPatch {
                language: set("de"),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(names_in(dir.path()), ["settings.json", "settings.json.bad"]);
    }
}

#[test]
fn a_save_goes_through_the_safe_write_helper() {
    // Nothing but the settings file is left after many saves, and a reader
    // in another thread never sees a file that is not complete JSON.
    let (dir, path) = data_dir();
    let store = Arc::new(open_clean(&path));
    store
        .update(&SettingsPatch {
            language: set("en"),
            ..SettingsPatch::default()
        })
        .unwrap();

    let writer = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            for round in 0..60 {
                store
                    .update(&SettingsPatch {
                        default_model_id: set(&format!("model-{round}")),
                        ..SettingsPatch::default()
                    })
                    .unwrap();
            }
        })
    };
    let mut reads = 0;
    while !writer.is_finished() {
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["language"], json!("en"));
        reads += 1;
    }
    writer.join().unwrap();

    assert!(reads > 0);
    assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);
    assert_eq!(
        json_of(&path),
        json!({"language": "en", "defaultModelId": "model-59"})
    );
    // The helper itself still replaces the file the store reads next time.
    safe_write(&path, b"{\"language\": \"sv\"}").unwrap();
    assert_eq!(open_clean(&path).get().language, "sv");
}

// Threads

#[test]
fn updates_from_several_threads_are_all_kept() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SettingsStore>();

    let (dir, path) = data_dir();
    write_json(&path, &json!({"theme": "dark"}));
    let store = Arc::new(open_clean(&path));
    const ROUNDS: usize = 25;

    // Each thread owns one setting and writes it many times. If an update
    // were built on a stale copy, one thread's last value would be undone
    // by another's save.
    let threads: Vec<_> = (0..4)
        .map(|worker| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for round in 0..ROUNDS {
                    let value = format!("{worker}-{round}");
                    let patch = match worker {
                        0 => SettingsPatch {
                            default_model_id: set(&value),
                            ..SettingsPatch::default()
                        },
                        1 => SettingsPatch {
                            language: set(&value),
                            ..SettingsPatch::default()
                        },
                        2 => SettingsPatch {
                            global_shortcut: set(&value),
                            ..SettingsPatch::default()
                        },
                        _ => SettingsPatch {
                            meetings_root: Patch::Set(PathBuf::from(format!("/roots/{value}"))),
                            ..SettingsPatch::default()
                        },
                    };
                    let after = store.update(&patch).unwrap();
                    // The returned settings always hold this thread's write.
                    let mine = match worker {
                        0 => after.default_model_id.unwrap(),
                        1 => after.language,
                        2 => after.global_shortcut.unwrap(),
                        _ => after.meetings_root.to_string_lossy().into_owned(),
                    };
                    assert!(mine.ends_with(&value), "{mine} / {value}");
                    let _ = store.get();
                }
            })
        })
        .collect();
    for handle in threads {
        handle.join().unwrap();
    }

    let last = ROUNDS - 1;
    let expected = Settings {
        default_model_id: Some(format!("0-{last}")),
        language: format!("1-{last}"),
        global_shortcut: Some(format!("2-{last}")),
        meetings_root: PathBuf::from(format!("/roots/3-{last}")),
    };
    assert_eq!(store.get(), expected);
    assert_eq!(open_clean(&path).get(), expected);
    assert_eq!(json_of(&path)["theme"], json!("dark"));
    assert_eq!(names_in(dir.path()), [SETTINGS_FILE]);
}
