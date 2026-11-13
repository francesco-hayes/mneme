//! What the store knows about one meeting, and how it maps to frontmatter.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::FrontmatterError;
use crate::frontmatter::{NotesFile, Scalar, quote};
use crate::naming::{DEFAULT_FOLDER_TITLE, folder_name, sanitize_title};
use crate::time::StartTime;

/// File in a meeting folder holding the frontmatter and the user's notes.
pub const NOTES_FILE: &str = "notes.md";
/// File in a meeting folder holding the transcript, one line per segment.
pub const TRANSCRIPT_FILE: &str = "transcript.md";
/// File in a meeting folder recording when each note line was typed.
pub const TIMELINE_FILE: &str = "notes-timeline.jsonl";

// Frontmatter keys, in the order a new meeting gets them.
pub(crate) const KEY_ID: &str = "id";
pub(crate) const KEY_TITLE: &str = "title";
pub(crate) const KEY_TITLE_EDITED: &str = "title_edited";
pub(crate) const KEY_DATE: &str = "date";
pub(crate) const KEY_DURATION: &str = "duration";
pub(crate) const KEY_STATUS: &str = "status";
pub(crate) const KEY_MODEL: &str = "model";
pub(crate) const KEY_LANGUAGE: &str = "language";

/// The stable identity of a meeting: the `id` key in its frontmatter.
///
/// It is assigned once at creation and never changes. The folder name is not
/// the id; folders are renamed, by Mneme and by hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeetingId(String);

impl MeetingId {
    /// Wraps an id read from somewhere else, such as the frontend.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// A fresh random id: a version 4 UUID in lowercase hyphenated form.
    pub(crate) fn random() -> Self {
        Self(uuid::Uuid::new_v4().hyphenated().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MeetingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for MeetingId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for MeetingId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

/// Where a meeting is in its life. Written to frontmatter in lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeetingStatus {
    /// Being recorded right now. Its folder is never renamed in this state.
    Recording,
    /// Stopped normally.
    Complete,
    /// Found still marked `recording` at launch: the app quit mid-meeting.
    Interrupted,
}

impl MeetingStatus {
    /// The word used in frontmatter and in the frontend contract.
    pub const fn as_str(self) -> &'static str {
        match self {
            MeetingStatus::Recording => "recording",
            MeetingStatus::Complete => "complete",
            MeetingStatus::Interrupted => "interrupted",
        }
    }

    /// Reads exactly what [`as_str`](Self::as_str) writes.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recording" => Some(MeetingStatus::Recording),
            "complete" => Some(MeetingStatus::Complete),
            "interrupted" => Some(MeetingStatus::Interrupted),
            _ => None,
        }
    }
}

impl fmt::Display for MeetingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One meeting as read from its folder: the frontmatter of `notes.md` plus
/// where the folder is.
///
/// This is a snapshot. The files are the source of truth, and both Mneme and
/// the user can change them; read the meeting again rather than holding on to
/// an old value, in particular for `folder`, which changes on rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingSummary {
    pub id: MeetingId,
    /// Display title, for example `Meeting, Oct 27 2:30 PM` or `Standup`.
    pub title: String,
    /// False until the user changes the title. While false the folder keeps
    /// the name it was created with: `Meeting` as its title part, followed by
    /// a number if that name was taken.
    pub title_edited: bool,
    /// Start time with its UTC offset: the `date` key.
    pub started_at: StartTime,
    /// Length in seconds; `None` while recording.
    pub duration_secs: Option<u64>,
    pub status: MeetingStatus,
    /// Id of the model that transcribed this meeting, if recorded.
    pub model: Option<String>,
    /// Language setting used for transcription, if recorded.
    pub language: Option<String>,
    /// The meeting's folder, a direct child of the store root.
    pub folder: PathBuf,
}

impl MeetingSummary {
    /// Path of this meeting's `notes.md`.
    pub fn notes_path(&self) -> PathBuf {
        self.folder.join(NOTES_FILE)
    }

    /// Path of this meeting's `transcript.md`.
    pub fn transcript_path(&self) -> PathBuf {
        self.folder.join(TRANSCRIPT_FILE)
    }

    /// Path of this meeting's `notes-timeline.jsonl`.
    pub fn timeline_path(&self) -> PathBuf {
        self.folder.join(TIMELINE_FILE)
    }

    /// The folder name this meeting's title asks for: `YYYY-MM-DD HHMM Title`
    /// from the start time's own wall clock, with `Meeting` as the title
    /// part until the title is edited. It never carries a number.
    ///
    /// It differs from the actual name of [`folder`](Self::folder) while a
    /// rename is waiting for the recording to stop, after the folder was
    /// renamed by hand, and for a meeting that was created as
    /// `… Meeting 2` because the plain name was taken. Only the first of
    /// those means the folder should move, so do not compare the two names
    /// to decide that; ask [`pending_folder_name`](Self::pending_folder_name).
    pub fn expected_folder_name(&self) -> String {
        if self.title_edited {
            folder_name(&self.started_at, &sanitize_title(&self.title))
        } else {
            folder_name(&self.started_at, DEFAULT_FOLDER_TITLE)
        }
    }

    /// The name this meeting's folder should be moved to, or `None` if the
    /// folder is fine where it is.
    ///
    /// This is the one place that decides whether a folder name is
    /// acceptable for its meeting. Rename, finish and recovery all go by it.
    ///
    /// - Title never edited: always `None`. The folder keeps the name it has,
    ///   whether that is the plain `… Meeting`, the `… Meeting 2` it was
    ///   created with in a minute that already had a meeting, or a name
    ///   someone gave it by hand. The folder name is not read to tell these
    ///   apart; none of them is ever "corrected".
    /// - Title edited: `None` when the folder already has exactly
    ///   [`expected_folder_name`](Self::expected_folder_name), otherwise that
    ///   name. An edited title always asks for the name without a number.
    ///
    /// `Some` does not mean the move can happen now: it waits while the
    /// meeting is recording, and fails if the name is taken.
    pub fn pending_folder_name(&self) -> Option<String> {
        if !self.title_edited {
            return None;
        }
        let expected = self.expected_folder_name();
        let current = self.folder.file_name().and_then(|name| name.to_str());
        (current != Some(expected.as_str())).then_some(expected)
    }

    /// Reads the meeting keys out of parsed frontmatter.
    ///
    /// `id`, `date` and `status` are required. A missing `title` falls back
    /// to the default title for the start time, a missing `title_edited` to
    /// false, and the rest to `None`.
    pub(crate) fn from_notes(doc: &NotesFile, folder: &Path) -> Result<Self, FrontmatterError> {
        let id = required(doc, KEY_ID)?.text;

        let date = required(doc, KEY_DATE)?;
        let started_at = StartTime::parse_rfc3339(&date.text).map_err(|_| {
            invalid(
                KEY_DATE,
                "must be a date and time with a UTC offset, like 2026-10-27T14:30:00+01:00",
            )
        })?;

        let status = required(doc, KEY_STATUS)?;
        let status = MeetingStatus::parse(&status.text)
            .ok_or_else(|| invalid(KEY_STATUS, "must be recording, complete or interrupted"))?;

        let title = optional(doc, KEY_TITLE)?
            .map(|scalar| scalar.text)
            .unwrap_or_else(|| started_at.default_title());

        let title_edited = match optional(doc, KEY_TITLE_EDITED)? {
            None => false,
            Some(scalar) => parse_bool(&scalar.text)
                .ok_or_else(|| invalid(KEY_TITLE_EDITED, "must be true or false"))?,
        };

        let duration_secs = match optional(doc, KEY_DURATION)? {
            None => None,
            Some(scalar) => Some(scalar.text.parse::<u64>().map_err(|_| {
                invalid(KEY_DURATION, "must be a whole number of seconds, or empty")
            })?),
        };

        Ok(Self {
            id: MeetingId(id),
            title,
            title_edited,
            started_at,
            duration_secs,
            status,
            model: optional(doc, KEY_MODEL)?.map(|scalar| scalar.text),
            language: optional(doc, KEY_LANGUAGE)?.map(|scalar| scalar.text),
            folder: folder.to_owned(),
        })
    }

    /// The `notes.md` of a new meeting: every key in its standard order and
    /// an empty body.
    pub(crate) fn to_new_notes(&self) -> NotesFile {
        let mut doc = NotesFile::new();
        // A UUID can never be mistaken for a number or a date, so it needs
        // no quotes; anything else gets the usual treatment.
        if uuid::Uuid::try_parse(self.id.as_str()).is_ok() {
            doc.set(KEY_ID, self.id.as_str());
        } else {
            doc.set_string(KEY_ID, self.id.as_str());
        }
        doc.set_string(KEY_TITLE, &self.title);
        doc.set(KEY_TITLE_EDITED, format_bool(self.title_edited));
        doc.set(KEY_DATE, &format_date(&self.started_at));
        doc.set(KEY_DURATION, &format_duration(self.duration_secs));
        doc.set(KEY_STATUS, self.status.as_str());
        doc.set(KEY_MODEL, &format_optional(self.model.as_deref()));
        doc.set(KEY_LANGUAGE, &format_optional(self.language.as_deref()));
        doc
    }
}

/// The `date` value. Always quoted: unquoted, YAML 1.1 tools read it as a
/// timestamp and may write it back in UTC, losing the offset that the folder
/// name depends on.
pub(crate) fn format_date(start: &StartTime) -> String {
    quote(&start.to_rfc3339())
}

pub(crate) fn format_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Seconds as digits, or empty (null) while there is no duration.
pub(crate) fn format_duration(duration_secs: Option<u64>) -> String {
    duration_secs
        .map(|secs| secs.to_string())
        .unwrap_or_default()
}

fn format_optional(value: Option<&str>) -> String {
    match value {
        Some(text) if !text.is_empty() => crate::frontmatter::format_string(text),
        _ => String::new(),
    }
}

/// A key that must be there and hold a value.
fn required(doc: &NotesFile, key: &'static str) -> Result<Scalar, FrontmatterError> {
    match doc.get(key)? {
        None => Err(FrontmatterError::MissingKey { key }),
        Some(scalar) if scalar.is_null() || scalar.text.is_empty() => Err(invalid(key, "is empty")),
        Some(scalar) => Ok(scalar),
    }
}

/// A key that may be absent, null or empty, all of which read as `None`.
fn optional(doc: &NotesFile, key: &str) -> Result<Option<Scalar>, FrontmatterError> {
    Ok(doc
        .get(key)?
        .filter(|scalar| !scalar.is_null() && !scalar.text.is_empty()))
}

fn invalid(key: &str, reason: &str) -> FrontmatterError {
    FrontmatterError::InvalidValue {
        key: key.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Booleans as people and YAML 1.1 tools write them.
fn parse_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary_of(frontmatter: &str) -> Result<MeetingSummary, FrontmatterError> {
        let text = format!("---\n{frontmatter}---\n");
        MeetingSummary::from_notes(&NotesFile::parse(&text).unwrap(), Path::new("/root/folder"))
    }

    const MINIMAL: &str = "id: abc\ndate: 2026-10-27T14:30:00+01:00\nstatus: complete\n";

    #[test]
    fn reads_every_key() {
        let summary = summary_of(
            "id: 7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f\n\
             title: \"Q4: planning\"\n\
             title_edited: true\n\
             date: \"2026-10-27T14:30:00+01:00\"\n\
             duration: 1800\n\
             status: complete\n\
             model: large-v3-turbo\n\
             language: \"no\"\n",
        )
        .unwrap();

        assert_eq!(summary.id.as_str(), "7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f");
        assert_eq!(summary.title, "Q4: planning");
        assert!(summary.title_edited);
        assert_eq!(summary.started_at.to_rfc3339(), "2026-10-27T14:30:00+01:00");
        assert_eq!(summary.duration_secs, Some(1800));
        assert_eq!(summary.status, MeetingStatus::Complete);
        assert_eq!(summary.model.as_deref(), Some("large-v3-turbo"));
        assert_eq!(summary.language.as_deref(), Some("no"));
        assert_eq!(summary.folder, Path::new("/root/folder"));
    }

    #[test]
    fn optional_keys_have_defaults() {
        let summary = summary_of(MINIMAL).unwrap();

        assert_eq!(summary.title, "Meeting, Oct 27 2:30 PM");
        assert!(!summary.title_edited);
        assert_eq!(summary.duration_secs, None);
        assert_eq!(summary.model, None);
        assert_eq!(summary.language, None);

        let summary = summary_of(&format!(
            "{MINIMAL}title:\ntitle_edited: ~\nduration: null\nmodel: \"\"\nlanguage:  # unset\n"
        ))
        .unwrap();
        assert_eq!(summary.title, "Meeting, Oct 27 2:30 PM");
        assert!(!summary.title_edited);
        assert_eq!(summary.duration_secs, None);
        assert_eq!(summary.model, None);
        assert_eq!(summary.language, None);
    }

    #[test]
    fn dates_are_read_quoted_or_not_and_in_utc_form() {
        for (written, expected) in [
            ("2026-10-27T14:30:00+01:00", "2026-10-27T14:30:00+01:00"),
            ("'2026-10-27T14:30:00+01:00'", "2026-10-27T14:30:00+01:00"),
            ("2026-10-27T13:30:00Z", "2026-10-27T13:30:00+00:00"),
            ("2026-10-27T13:30:00.000Z", "2026-10-27T13:30:00+00:00"),
            ("2026-10-27 14:30:00+01:00", "2026-10-27T14:30:00+01:00"),
        ] {
            let summary = summary_of(&format!("id: a\ndate: {written}\nstatus: complete\n"));
            assert_eq!(
                summary.map(|s| s.started_at.to_rfc3339()).as_deref(),
                Ok(expected),
                "{written}"
            );
        }
    }

    #[test]
    fn booleans_accept_the_usual_spellings() {
        for (written, expected) in [
            ("true", true),
            ("True", true),
            ("yes", true),
            ("false", false),
            ("FALSE", false),
            ("no", false),
            ("\"true\"", true),
        ] {
            let summary = summary_of(&format!("{MINIMAL}title_edited: {written}\n")).unwrap();
            assert_eq!(summary.title_edited, expected, "{written}");
        }
    }

    #[test]
    fn missing_required_keys_are_named() {
        for key in ["id", "date", "status"] {
            let frontmatter: String = MINIMAL
                .lines()
                .filter(|line| !line.starts_with(key))
                .map(|line| format!("{line}\n"))
                .collect();
            assert_eq!(
                summary_of(&frontmatter),
                Err(FrontmatterError::MissingKey { key })
            );
        }
        assert_eq!(
            summary_of(""),
            Err(FrontmatterError::MissingKey { key: "id" })
        );
    }

    #[test]
    fn unusable_values_are_errors_that_name_the_key() {
        for (line, key) in [
            ("id:\n", "id"),
            ("id: \"\"\n", "id"),
            ("date: yesterday\n", "date"),
            ("date: 2026-10-27\n", "date"),
            ("date: 2026-10-27T14:30:00\n", "date"),
            ("status: done\n", "status"),
            ("status: Recording\n", "status"),
            ("status:\n", "status"),
            ("title_edited: maybe\n", "title_edited"),
            ("duration: 12.5\n", "duration"),
            ("duration: -3\n", "duration"),
            ("duration: 30m\n", "duration"),
            ("title: |\n  block\n", "title"),
            ("model: [a, b]\n", "model"),
            ("language:\n  - en\n", "language"),
        ] {
            let frontmatter: String = MINIMAL
                .lines()
                .filter(|existing| !existing.starts_with(&format!("{key}:")))
                .map(|existing| format!("{existing}\n"))
                .chain([line.to_owned()])
                .collect();
            let err = summary_of(&frontmatter).unwrap_err();
            assert!(
                matches!(&err, FrontmatterError::InvalidValue { key: k, .. } if k == key),
                "{line:?}: {err:?}"
            );
        }
    }

    #[test]
    fn a_new_meeting_writes_every_key_in_order_and_reads_back_the_same() {
        let summary = MeetingSummary {
            id: MeetingId::new("7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f"),
            title: "Meeting, Oct 27 2:30 PM".to_owned(),
            title_edited: false,
            started_at: "2026-10-27T14:30:00+01:00".parse().unwrap(),
            duration_secs: None,
            status: MeetingStatus::Recording,
            model: Some("large-v3-turbo".to_owned()),
            language: Some("auto".to_owned()),
            folder: PathBuf::from("/root/folder"),
        };

        let doc = summary.to_new_notes();

        assert_eq!(
            doc.render(),
            "---\n\
             id: 7b1c2f9e-5a34-4c1d-9e0f-2a6b8c4d1e3f\n\
             title: Meeting, Oct 27 2:30 PM\n\
             title_edited: false\n\
             date: \"2026-10-27T14:30:00+01:00\"\n\
             duration:\n\
             status: recording\n\
             model: large-v3-turbo\n\
             language: auto\n\
             ---\n"
        );
        assert_eq!(
            MeetingSummary::from_notes(&doc, Path::new("/root/folder")),
            Ok(summary)
        );
    }

    #[test]
    fn missing_model_and_language_are_written_empty_and_odd_ones_quoted() {
        let mut summary = summary_of(MINIMAL).unwrap();
        let rendered = summary.to_new_notes().render();
        assert!(
            rendered.contains("\nmodel:\nlanguage:\n---\n"),
            "{rendered}"
        );

        summary.model = Some("ggml: custom #1".to_owned());
        summary.language = Some("no".to_owned());
        let doc = summary.to_new_notes();
        let rendered = doc.render();
        assert!(
            rendered.contains("\nmodel: \"ggml: custom #1\"\nlanguage: \"no\"\n"),
            "{rendered}"
        );
        assert_eq!(
            MeetingSummary::from_notes(&doc, Path::new("/root/folder")),
            Ok(summary)
        );
    }

    #[test]
    fn ids_are_random_lowercase_uuids() {
        let a = MeetingId::random();
        let b = MeetingId::random();

        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 36);
        assert_eq!(a.as_str(), a.as_str().to_lowercase());
        let parsed = uuid::Uuid::try_parse(a.as_str()).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_eq!(MeetingId::from(a.to_string()), a);
    }

    #[test]
    fn expected_folder_name_uses_meeting_until_the_title_is_edited() {
        let mut summary = summary_of(MINIMAL).unwrap();
        assert_eq!(summary.expected_folder_name(), "2026-10-27 1430 Meeting");

        // A title typed into the file by hand does not count as edited.
        summary.title = "Typed by hand".to_owned();
        assert_eq!(summary.expected_folder_name(), "2026-10-27 1430 Meeting");

        summary.title_edited = true;
        summary.title = "Q3/Q4: review".to_owned();
        assert_eq!(
            summary.expected_folder_name(),
            "2026-10-27 1430 Q3 Q4 review"
        );
    }

    #[test]
    fn a_folder_only_has_a_pending_name_when_an_edited_title_asks_for_another() {
        let mut summary = summary_of(MINIMAL).unwrap();

        // Title never edited: any folder name is left alone.
        for name in [
            "2026-10-27 1430 Meeting",
            "2026-10-27 1430 Meeting 2",
            "2026-10-27 1430 Meeting 999",
            "2026-10-27 1430 Meeting (important)",
            "Renamed in Finder",
        ] {
            summary.folder = Path::new("/root").join(name);
            assert_eq!(summary.pending_folder_name(), None, "{name}");
        }

        // Title edited: only the exact name will do, and it has no number.
        summary.title_edited = true;
        summary.title = "Standup".to_owned();
        for name in [
            "2026-10-27 1430 Meeting",
            "2026-10-27 1430 Meeting 2",
            "2026-10-27 1430 Standup 2",
            "2026-10-27 1430 standup",
            "Renamed in Finder",
        ] {
            summary.folder = Path::new("/root").join(name);
            assert_eq!(
                summary.pending_folder_name().as_deref(),
                Some("2026-10-27 1430 Standup"),
                "{name}"
            );
        }
        summary.folder = PathBuf::from("/root/2026-10-27 1430 Standup");
        assert_eq!(summary.pending_folder_name(), None);
    }

    #[test]
    fn a_title_that_ends_in_a_number_is_a_title_not_a_numbered_default() {
        let mut summary = summary_of(MINIMAL).unwrap();
        summary.title_edited = true;

        for (title, name) in [
            ("Sprint 2", "2026-10-27 1430 Sprint 2"),
            ("Meeting 2", "2026-10-27 1430 Meeting 2"),
            ("2", "2026-10-27 1430 2"),
        ] {
            summary.title = title.to_owned();
            assert_eq!(summary.expected_folder_name(), name);

            // In its own folder it stays; the number is not stripped.
            summary.folder = Path::new("/root").join(name);
            assert_eq!(summary.pending_folder_name(), None, "{title}");

            // In the default folder it asks for the whole title.
            summary.folder = PathBuf::from("/root/2026-10-27 1430 Meeting");
            assert_eq!(summary.pending_folder_name().as_deref(), Some(name));
        }

        // A meeting titled "Meeting" sitting in a numbered default folder
        // asks for the plain name: an edited title never keeps the number.
        summary.title = "Meeting".to_owned();
        summary.folder = PathBuf::from("/root/2026-10-27 1430 Meeting 2");
        assert_eq!(
            summary.pending_folder_name().as_deref(),
            Some("2026-10-27 1430 Meeting")
        );
    }

    #[test]
    fn status_words_round_trip() {
        for status in [
            MeetingStatus::Recording,
            MeetingStatus::Complete,
            MeetingStatus::Interrupted,
        ] {
            assert_eq!(MeetingStatus::parse(status.as_str()), Some(status));
            assert_eq!(status.to_string(), status.as_str());
        }
        assert_eq!(MeetingStatus::parse("Complete"), None);
        assert_eq!(MeetingStatus::parse(""), None);
    }
}
