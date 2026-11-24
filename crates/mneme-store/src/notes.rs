//! The notes body of `notes.md`, and `notes-timeline.jsonl`.
//!
//! # The body
//!
//! The body is everything after the line that closes the frontmatter. It is
//! read and saved as one string, byte for byte: nothing is trimmed, no line
//! ending is changed, and no newline is added at the end. Saving a body
//! replaces only the body; the frontmatter goes back exactly as it was read.
//! Every save goes through the safe write helper, so a crash leaves the old
//! `notes.md` or the new one, never a mix.
//!
//! # The title
//!
//! A save can also set the meeting's title from the notes: see
//! [`heading_title`] and [`Store::save_notes`].
//!
//! # The timeline
//!
//! While a meeting is recording, each save also records which lines were
//! typed since the save before it, with the meeting time:
//!
//! ```text
//! {"t":"00:04:12","text":"- ask Ana about the API"}
//! ```
//!
//! The file is only ever appended to. It is a record of what was typed when,
//! not a pointer into `notes.md`: later edits to the notes never change it.
//! Nothing reads it in v1; the v2 enhancer uses it to match each note line to
//! the conversation around it.
//!
//! The store keeps nothing in memory between saves. "The save before" is
//! whatever body is on disk, so the first save after a restart records only
//! what changed, like any other save. Which lines count as typed is decided
//! by [`typed_lines`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mneme_core::format_timestamp;
use serde::Serialize;

use crate::append::{open_for_append, write_lines};
use crate::error::{Error, Result};
use crate::meeting::{
    KEY_TITLE, KEY_TITLE_EDITED, MeetingId, MeetingStatus, MeetingSummary, TIMELINE_FILE,
    format_bool,
};
use crate::naming::normalize_title;
use crate::safe_write::PendingWrite;
use crate::store::{self, Store};

/// What a save did besides saving the notes.
#[derive(Debug)]
pub struct SavedNotes {
    /// The meeting as it is after the save. Its title is the notes' first
    /// heading when the save took the title from it, and its folder is the
    /// renamed one when that rename was done.
    pub meeting: MeetingSummary,
    /// True when this save gave the meeting a new title from its notes.
    pub title_changed: bool,
    /// Set when the title changed but the folder could not be renamed to
    /// match (a folder with that name exists, say). The notes and the title
    /// are saved; the folder keeps its name until a later rename or the end
    /// of the recording tries again.
    pub rename_error: Option<Error>,
    /// How many entries this save appended to the note timeline. Zero when
    /// no line was new or changed, and when the meeting is not recording.
    pub timeline_entries: usize,
    /// Set when the notes were saved but the timeline could not be appended
    /// to. The notes are the user's words and are never held back for the
    /// timeline; the entries of this save are lost, because the next save
    /// compares against the notes that were just written.
    pub timeline_error: Option<Error>,
}

impl Store {
    /// The body of a meeting's `notes.md`: everything after the frontmatter,
    /// exactly as it is on disk. Empty for a new meeting.
    ///
    /// What [`save_notes`](Self::save_notes) was given is what this returns,
    /// byte for byte, and saving what this returned changes nothing on disk.
    /// A file edited by hand may begin with a blank line after the
    /// frontmatter; that line is part of the body and is returned too.
    pub fn notes(&self, id: &MeetingId) -> Result<String> {
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        Ok(store::load(&folder)?.doc.body)
    }

    /// Replaces the body of a meeting's `notes.md` and leaves the timeline
    /// alone. For saves when the meeting is not recording.
    ///
    /// The body is stored exactly as given: it is not trimmed, its line
    /// endings (`\n`, `\r\n`, or a mix) are kept, and no final newline is
    /// added. The frontmatter is written back byte for byte, unknown keys
    /// and comments included. The write is a temp file and a rename, so a
    /// crash leaves the old file or the new one. A body equal to the one on
    /// disk writes nothing.
    ///
    /// While a meeting is recording use [`save_notes_at`](Self::save_notes_at)
    /// instead: lines saved through this method are never added to the
    /// timeline later, because later saves find them already on disk.
    ///
    /// # The title follows the first heading
    ///
    /// The meeting takes its title from the notes' first `# ` heading (see
    /// [`heading_title`]) while the title still follows the notes: while it
    /// has never been edited, or while it is the same as the heading of the
    /// body on disk before this save. So typing `# Standup` names the
    /// meeting, and editing that heading renames it again; a title typed in
    /// the title field that differs from the heading stops it. A save whose
    /// notes have no heading leaves the title as it is.
    ///
    /// A title taken this way counts as edited (`title_edited: true`), and
    /// the folder is renamed to match as [`rename_meeting`](Self::rename_meeting)
    /// does, waiting for the end of the recording while the meeting records.
    /// A rename that fails does not fail the save: see
    /// [`SavedNotes::rename_error`].
    pub fn save_notes(&self, id: &MeetingId, body: &str) -> Result<SavedNotes> {
        self.save_notes_inner(id, body, None)
    }

    /// Saves the notes as [`save_notes`](Self::save_notes) does and, if the
    /// meeting is recording, appends one entry to `notes-timeline.jsonl` for
    /// each line that is new or changed since the body on disk:
    /// `{"t":"hh:mm:ss","text":"<line>"}`, with `meeting_time_ms` on the
    /// transcript's clock as `t`.
    ///
    /// Which lines are recorded:
    /// - A line is compared by its text without the whitespace around it,
    ///   and that trimmed text is what is recorded. Indenting a line or
    ///   changing its line ending is not a change.
    /// - Lines are counted, not located. A text that is in the new body more
    ///   times than in the old one is recorded once per extra copy. So a new
    ///   line and an edited line are recorded; a line that only moved, a
    ///   deleted line, an unchanged line and a blank line are not.
    /// - A line that is still being typed is recorded each time a save finds
    ///   it changed, so it can appear as `- ask`, `- ask Ana` and then in
    ///   full. The earliest of those is when the line was started.
    ///
    /// The status in frontmatter decides, not the caller: if the meeting is
    /// not recording (it was finished a moment ago, say), the time is
    /// ignored, the notes are saved, and the timeline is not touched.
    ///
    /// The entries are appended and synced to disk before `notes.md` is
    /// replaced. A crash between the two leaves the old notes and a timeline
    /// that already has the entries; saving again then records those lines a
    /// second time with the later time. So the timeline can repeat a text,
    /// but a crash never leaves a line in the notes without its entry. Read
    /// the earliest entry for a text as the time it was typed.
    ///
    /// An error means the notes were not saved. A failure to append to the
    /// timeline is not an error: the notes are saved and the failure is in
    /// [`SavedNotes::timeline_error`]. That is the one case in which a line
    /// saved through this method during a recording has no entry.
    pub fn save_notes_at(
        &self,
        id: &MeetingId,
        body: &str,
        meeting_time_ms: u64,
    ) -> Result<SavedNotes> {
        self.save_notes_inner(id, body, Some(meeting_time_ms))
    }

    fn save_notes_inner(
        &self,
        id: &MeetingId,
        body: &str,
        meeting_time_ms: Option<u64>,
    ) -> Result<SavedNotes> {
        // The lock covers the whole read-change-write, so a rename or a
        // finish cannot land between the read and the write and be undone.
        let _guard = self.lock();
        let folder = self.find_folder(id)?;
        let staged = stage_save(&folder, body, meeting_time_ms)?;
        let mut saved = staged.commit()?;
        // A folder is never renamed while files in it are being appended
        // to; the end of the recording applies it.
        if saved.title_changed && saved.meeting.status != MeetingStatus::Recording {
            saved.rename_error = self.apply_pending_rename(&mut saved.meeting).err();
        }
        Ok(saved)
    }
}

/// The title the notes ask for: the text of the first level-one ATX heading
/// (`# Title`), tidied to one line as titles are. `None` when there is none.
///
/// The heading rules are CommonMark's, as far as a title needs them: up to
/// three spaces before the `#`, then a space or tab (so `#tag` and `##` are
/// not one), and an optional closing run of `#` after a space. Lines inside
/// fenced code blocks (``` or ~~~) are skipped, so a `# comment` in a shell
/// snippet is not a title. A heading with no text is skipped too, and the
/// next one counts. Setext headings (`Title` over `===`) are not read.
pub fn heading_title(body: &str) -> Option<String> {
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let rest = line.trim_start_matches(' ');
        let indent = line.len() - rest.len();
        if indent > 3 {
            continue;
        }
        if let Some((mark, len)) = fence_marker(rest) {
            match fence {
                None => fence = Some((mark, len)),
                Some((open, open_len))
                    if open == mark
                        && len >= open_len
                        && rest.trim_start_matches(mark).trim().is_empty() =>
                {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let Some(after) = rest.strip_prefix('#') else {
            continue;
        };
        if !(after.is_empty() || after.starts_with([' ', '\t'])) {
            continue;
        }
        let title = normalize_title(without_closing_hashes(after.trim()));
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

/// The fence character and how many of it start the line, for a line that
/// opens or closes a fenced code block: three or more backticks or tildes.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let mark = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = line.chars().take_while(|&c| c == mark).count();
    (len >= 3).then_some((mark, len))
}

/// A heading's text without its optional closing sequence: `Title ##`
/// becomes `Title`, while `C#` stays `C#`.
fn without_closing_hashes(text: &str) -> &str {
    let stripped = text.trim_end_matches('#');
    if stripped.is_empty() {
        ""
    } else if stripped.ends_with([' ', '\t']) {
        stripped.trim_end()
    } else {
        text
    }
}

/// A save that has done everything but replace `notes.md`: the new file is
/// in a temp file beside it and the timeline entries are on disk. Separate
/// from the last step so a test can stop here, which is what a crash at the
/// worst moment looks like.
struct StagedSave {
    /// `None` when the body on disk is already the one being saved.
    pending: Option<PendingWrite>,
    notes_path: PathBuf,
    saved: SavedNotes,
}

impl StagedSave {
    fn commit(self) -> Result<SavedNotes> {
        if let Some(pending) = self.pending {
            pending
                .commit()
                .map_err(|err| Error::io(&self.notes_path, err))?;
        }
        Ok(self.saved)
    }
}

/// Everything in a save up to, but not including, the rename that puts the
/// new `notes.md` in place. The caller holds the store lock.
///
/// The order is: write the new `notes.md` to its temp file, append the
/// timeline entries and sync them, and only then (in [`StagedSave::commit`])
/// rename. The temp file comes first because it is the step most likely to
/// fail (a full disk, a folder that cannot be written), and a save that
/// fails there must not have logged anything. The timeline comes before the
/// rename so that a crash can repeat an entry but never drop one: the time a
/// line was typed cannot be worked out afterwards.
fn stage_save(folder: &Path, body: &str, meeting_time_ms: Option<u64>) -> Result<StagedSave> {
    let mut open = store::load(folder)?;

    // The title follows the notes while it is untouched or still the old
    // heading. See `Store::save_notes`.
    let follows = !open.summary.title_edited
        || heading_title(&open.doc.body).as_deref() == Some(open.summary.title.as_str());
    let mut title_changed = false;
    if follows
        && let Some(title) = heading_title(body)
        && title != open.summary.title
    {
        open.doc.set_string(KEY_TITLE, &title);
        open.doc.set(KEY_TITLE_EDITED, format_bool(true));
        open.summary.title = title;
        open.summary.title_edited = true;
        title_changed = true;
    }

    let typed = match meeting_time_ms {
        Some(_) if open.summary.status == MeetingStatus::Recording => {
            typed_lines(&open.doc.body, body)
        }
        _ => Vec::new(),
    };

    open.doc.body = body.to_owned();
    let pending = open.stage()?;

    let mut saved = SavedNotes {
        meeting: open.summary,
        title_changed,
        rename_error: None,
        timeline_entries: 0,
        timeline_error: None,
    };
    if let (Some(time_ms), false) = (meeting_time_ms, typed.is_empty()) {
        match append_entries(&folder.join(TIMELINE_FILE), time_ms, &typed) {
            Ok(()) => saved.timeline_entries = typed.len(),
            Err(err) => saved.timeline_error = Some(err),
        }
    }

    Ok(StagedSave {
        pending,
        notes_path: open.path,
        saved,
    })
}

/// The lines of `new` that were typed since `old`: the texts the timeline
/// records for a save that replaces body `old` with body `new`, in the order
/// they appear in `new`.
///
/// Each line is reduced to its text without surrounding whitespace (which
/// also drops a `\r` left by a Windows line ending). Lines with no text left
/// are ignored on both sides. Then the two bodies are compared as bags of
/// lines, with no regard for position: each line of `new` is paired with one
/// unused line of `old` that has the same text, and the lines of `new` left
/// without a partner are the result. When a text is in `new` more often than
/// in `old`, the first copies are the paired ones and the later ones are the
/// result.
///
/// This is linear in the size of the two bodies, so a large paste costs no
/// more than reading it.
fn typed_lines<'a>(old: &str, new: &'a str) -> Vec<&'a str> {
    let mut unused: HashMap<&str, usize> = HashMap::new();
    for text in texts(old) {
        *unused.entry(text).or_default() += 1;
    }
    texts(new)
        .filter(|text| match unused.get_mut(text) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .collect()
}

/// The non-blank lines of a notes body, each without the whitespace around
/// it.
fn texts(body: &str) -> impl Iterator<Item = &str> {
    body.lines().map(str::trim).filter(|text| !text.is_empty())
}

/// One line of `notes-timeline.jsonl`. The field order here is the order in
/// the file.
#[derive(Serialize)]
struct TimelineEntry<'a> {
    /// Meeting time as `hh:mm:ss`, on the transcript's clock.
    t: &'a str,
    /// The line as typed, without surrounding whitespace.
    text: &'a str,
}

/// The lines a save adds to the timeline: one JSON object per text, each on
/// its own line, all with the same time.
///
/// `serde_json` does the escaping: quotes, backslashes and control
/// characters (a tab or a stray `\r` inside a line) are escaped, so an entry
/// never contains a raw newline. Everything else, non-ASCII included, is
/// written as it is in UTF-8.
fn format_entries(meeting_time_ms: u64, texts: &[&str]) -> serde_json::Result<String> {
    let t = format_timestamp(meeting_time_ms);
    let mut lines = String::new();
    for text in texts {
        lines.push_str(&serde_json::to_string(&TimelineEntry { t: &t, text })?);
        lines.push('\n');
    }
    Ok(lines)
}

/// Appends a save's entries to the timeline at `path` in one write and syncs
/// the file before returning (`F_FULLFSYNC` on macOS, as for the
/// transcript). The file is created if it is missing. If it ends in an entry
/// cut off by an earlier crash, the new entries start on a fresh line and
/// the cut-off one stays behind as a line that is not valid JSON.
fn append_entries(path: &Path, meeting_time_ms: u64, texts: &[&str]) -> Result<()> {
    let lines =
        format_entries(meeting_time_ms, texts).map_err(|err| Error::io(path, err.into()))?;
    let mut file = open_for_append(path)?;
    write_lines(&mut file, &lines).map_err(|err| Error::io(path, err))?;
    file.sync_data().map_err(|err| Error::io(path, err))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::meeting::{MeetingSummary, NOTES_FILE};
    use crate::safe_write::is_temp_file_name;
    use crate::store::NewMeeting;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("Mneme")).unwrap();
        (dir, store)
    }

    fn create(store: &Store) -> MeetingSummary {
        store
            .create_meeting(NewMeeting {
                started_at: "2026-10-27T14:30:00+01:00".parse().unwrap(),
                model: None,
                language: None,
            })
            .unwrap()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // --- the diff ------------------------------------------------------------

    #[test]
    fn new_lines_are_typed_lines_in_the_order_of_the_new_body() {
        assert_eq!(typed_lines("", "- a"), ["- a"]);
        assert_eq!(typed_lines("", "- a\n- b\n- c\n"), ["- a", "- b", "- c"]);
        assert_eq!(typed_lines("- a\n", "- a\n- b\n"), ["- b"]);
        assert_eq!(typed_lines("- b\n", "- a\n- b\n- c"), ["- a", "- c"]);
    }

    #[test]
    fn a_changed_line_is_typed_with_its_new_text() {
        assert_eq!(
            typed_lines("- a\n- b\n- c\n", "- a\n- b, edited\n- c\n"),
            ["- b, edited"]
        );
        // Each state of a line that is still growing is a change.
        assert_eq!(typed_lines("- ask\n", "- ask Ana\n"), ["- ask Ana"]);
        // So is a change of marker or of case.
        assert_eq!(typed_lines("- [ ] call\n", "- [x] call\n"), ["- [x] call"]);
        assert_eq!(typed_lines("api\n", "API\n"), ["API"]);
    }

    #[test]
    fn moved_deleted_unchanged_and_blank_lines_are_not_typed() {
        let old = "# Title\n\n- a\n- b\n- c\n";
        let none: [&str; 0] = [];
        // Unchanged.
        assert_eq!(typed_lines(old, old), none);
        // Reordered.
        assert_eq!(typed_lines(old, "- c\n- a\n\n# Title\n- b\n"), none);
        // Deleted.
        assert_eq!(typed_lines(old, "# Title\n- b\n"), none);
        assert_eq!(typed_lines(old, ""), none);
        // Blank lines added, of every kind.
        assert_eq!(
            typed_lines(old, "\n\n# Title\n \n\t\n- a\n\u{a0}\n- b\n\r\n- c\n\n\n"),
            none
        );
        assert_eq!(typed_lines("", "\n \n\t\r\n"), none);
        // Moved and deleted together with one real change.
        assert_eq!(typed_lines(old, "- c\n- new\n# Title\n"), ["- new"]);
    }

    #[test]
    fn whitespace_around_a_line_is_not_part_of_its_text() {
        let none: [&str; 0] = [];
        // Indenting, trailing spaces and Windows line endings change nothing.
        assert_eq!(typed_lines("- a\n- b\n", "- a\n    - b  \n"), none);
        assert_eq!(typed_lines("- a\n- b\n", "- a\r\n- b\r\n"), none);
        assert_eq!(typed_lines("- a\r\n- b", "- a\n- b\n"), none);
        assert_eq!(typed_lines("\t- a", "- a\t"), none);
        // What is recorded is the trimmed text.
        assert_eq!(typed_lines("", "  \t- a  \r\n"), ["- a"]);
        // Whitespace inside a line is text.
        assert_eq!(typed_lines("- a b\n", "- a  b\n"), ["- a  b"]);
        // A lone carriage return does not end a line; it stays in the text.
        assert_eq!(typed_lines("", "one\rtwo\n"), ["one\rtwo"]);
    }

    #[test]
    fn copies_of_a_line_are_counted() {
        let none: [&str; 0] = [];
        // Two copies, then a third: one typed line.
        assert_eq!(
            typed_lines("- todo\n- todo\n", "- todo\n- todo\n- todo\n"),
            ["- todo"]
        );
        // One copy fewer: nothing.
        assert_eq!(typed_lines("- todo\n- todo\n", "- todo\n"), none);
        // The same two copies in other places: nothing.
        assert_eq!(
            typed_lines("- todo\nx\n- todo\n", "- todo\n- todo\nx\n"),
            none
        );
        // Three copies pasted at once: three.
        assert_eq!(typed_lines("", "a\na\na\n"), ["a", "a", "a"]);
        // A copy deleted in one place and typed in another looks like a move.
        assert_eq!(typed_lines("a\nb\n", "b\na\n"), none);
        // A copy that differs only in indentation is still a copy.
        assert_eq!(typed_lines("- todo\n", "- todo\n  - todo\n"), ["- todo"]);
    }

    #[test]
    fn a_line_edited_back_to_an_earlier_text_is_typed_again() {
        // The store compares with the body on disk only, so it does not
        // know the first text was there two saves ago.
        assert_eq!(typed_lines("- a\n", "- b\n"), ["- b"]);
        assert_eq!(typed_lines("- b\n", "- a\n"), ["- a"]);
    }

    #[test]
    fn splitting_and_joining_lines_counts_as_typing() {
        // Known imperfection: no character was typed, but the lines are new.
        assert_eq!(typed_lines("one two\n", "one\ntwo\n"), ["one", "two"]);
        assert_eq!(typed_lines("one\ntwo\n", "one two\n"), ["one two"]);
    }

    #[test]
    fn a_large_paste_is_one_typed_line_per_pasted_line() {
        let old: String = (0..20_000).map(|n| format!("- old line {n}\n")).collect();
        let pasted: String = (0..20_000).map(|n| format!("- pasted {n}\n")).collect();
        let new = format!("{pasted}{old}");

        let typed = typed_lines(&old, &new);

        assert_eq!(typed.len(), 20_000);
        assert_eq!(typed[0], "- pasted 0");
        assert_eq!(typed[19_999], "- pasted 19999");
    }

    // --- the entries -----------------------------------------------------------

    #[test]
    fn an_entry_is_the_time_then_the_text_as_one_json_line() {
        assert_eq!(
            format_entries(252_000, &["- ask Ana about the API"]).unwrap(),
            "{\"t\":\"00:04:12\",\"text\":\"- ask Ana about the API\"}\n"
        );
        assert_eq!(
            format_entries(100 * 3_600_000 + 999, &["a", "b"]).unwrap(),
            "{\"t\":\"100:00:00\",\"text\":\"a\"}\n{\"t\":\"100:00:00\",\"text\":\"b\"}\n"
        );
        assert_eq!(format_entries(0, &[]).unwrap(), "");
    }

    #[test]
    fn awkward_text_is_escaped_onto_one_line_and_reads_back() {
        let texts = [
            r#"she said "ship it""#,
            r"back\slash and \n that is not a newline",
            "tab\tand a lone\rcarriage return",
            "control \u{0} \u{1b} \u{7f}",
            "ünïcödé, 日本語, עברית, emoji 🎙️👩‍💻",
            "line\u{2028}separator and next\u{85}line",
            r#"{"t":"00:00:00","text":"json inside json"}"#,
            "'single' `code` <tag> & /slash/",
        ];

        let lines = format_entries(61_000, &texts).unwrap();

        assert_eq!(lines.matches('\n').count(), texts.len());
        for (line, text) in lines.split_terminator('\n').zip(texts) {
            assert!(!line.contains('\r'), "{line:?}");
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["t"], "00:01:01");
            assert_eq!(value["text"], text);
            assert_eq!(value.as_object().unwrap().len(), 2);
        }
    }

    // --- crashes ---------------------------------------------------------------

    #[test]
    fn a_crash_before_the_rename_leaves_the_old_notes_intact() {
        let (_dir, store) = store();
        let meeting = create(&store);
        store.save_notes(&meeting.id, "- first\n").unwrap();
        let notes_path = meeting.folder.join(NOTES_FILE);
        let before = fs::read(&notes_path).unwrap();

        // A save that stops after the temp file is written and before it is
        // renamed over notes.md. A crash runs no destructor.
        let staged = stage_save(&meeting.folder, "- first\n- second\n", None).unwrap();
        let temp = staged.pending.as_ref().unwrap().temp_path().to_owned();
        std::mem::forget(staged);

        // notes.md is the old file, whole.
        assert_eq!(fs::read(&notes_path).unwrap(), before);
        assert_eq!(store.notes(&meeting.id).unwrap(), "- first\n");
        // The new text only ever existed in a hidden temp file.
        assert!(is_temp_file_name(
            temp.file_name().unwrap().to_str().unwrap()
        ));
        assert!(
            fs::read_to_string(&temp)
                .unwrap()
                .ends_with("---\n- first\n- second\n")
        );
        assert_eq!(names_in(&meeting.folder).len(), 4);

        // The meeting is still readable and the next save goes through.
        assert_eq!(store.meeting(&meeting.id).unwrap(), meeting);
        store
            .save_notes(&meeting.id, "- first\n- second\n")
            .unwrap();
        assert_eq!(store.notes(&meeting.id).unwrap(), "- first\n- second\n");
    }

    #[test]
    fn a_crash_between_the_timeline_and_the_notes_repeats_entries_and_drops_none() {
        let (_dir, store) = store();
        let meeting = create(&store);
        store
            .save_notes_at(&meeting.id, "- first\n", 1_000)
            .unwrap();
        let notes_before = fs::read(meeting.notes_path()).unwrap();

        // Crash after the timeline entry is on disk, before notes.md is
        // replaced.
        let staged = stage_save(&meeting.folder, "- first\n- second\n", Some(5_000)).unwrap();
        assert_eq!(staged.saved.timeline_entries, 1);
        std::mem::forget(staged);

        assert_eq!(fs::read(meeting.notes_path()).unwrap(), notes_before);
        assert_eq!(
            fs::read_to_string(meeting.timeline_path()).unwrap(),
            "{\"t\":\"00:00:01\",\"text\":\"- first\"}\n\
             {\"t\":\"00:00:05\",\"text\":\"- second\"}\n"
        );

        // The app comes back with the old notes; the line is typed again and
        // saved. It is compared with the notes on disk, so it is recorded
        // again: the timeline wins nothing back, but loses nothing either.
        let saved = store
            .save_notes_at(&meeting.id, "- first\n- second\n", 9_000)
            .unwrap();
        assert_eq!(saved.timeline_entries, 1);
        assert_eq!(
            fs::read_to_string(meeting.timeline_path()).unwrap(),
            "{\"t\":\"00:00:01\",\"text\":\"- first\"}\n\
             {\"t\":\"00:00:05\",\"text\":\"- second\"}\n\
             {\"t\":\"00:00:09\",\"text\":\"- second\"}\n"
        );
        assert_eq!(store.notes(&meeting.id).unwrap(), "- first\n- second\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_save_that_cannot_write_its_temp_file_logs_nothing() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, store) = store();
        let meeting = create(&store);
        store
            .save_notes_at(&meeting.id, "- first\n", 1_000)
            .unwrap();
        let timeline_before = fs::read(meeting.timeline_path()).unwrap();

        // A folder that cannot be written to: no temp file can be made, but
        // the timeline, which already exists, could still be appended to.
        fs::set_permissions(&meeting.folder, fs::Permissions::from_mode(0o555)).unwrap();
        let result = store.save_notes_at(&meeting.id, "- first\n- second\n", 5_000);
        fs::set_permissions(&meeting.folder, fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::Io { path, .. } if *path == meeting.notes_path()),
            "{err:?}"
        );
        assert_eq!(store.notes(&meeting.id).unwrap(), "- first\n");
        assert_eq!(fs::read(meeting.timeline_path()).unwrap(), timeline_before);
    }
}
