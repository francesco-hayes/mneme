//! `transcript.md`: one line per transcribed segment.
//!
//! ```text
//! [00:01:23] Them: Let's start.
//! [00:01:27] Me: Sure, one second.
//! ```
//!
//! The file grows one line at a time while the meeting runs
//! ([`TranscriptWriter::append`]), so a crash loses at most the line being
//! written. Two channels are transcribed by one worker and can finish out of
//! time order, so the file is sorted once at the end
//! ([`TranscriptWriter::finalize`]) through [`safe_write`].
//!
//! A line is complete once it ends in a newline. Whatever follows the last
//! newline is a line that was cut off mid-write: the parser skips it, and the
//! next append starts on a fresh line.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mneme_core::{Segment, Speaker, format_timestamp, parse_timestamp};

use crate::append::{is_file_at, is_linked, open_for_append, write_lines};
use crate::error::{Error, Result};
use crate::meeting::{MeetingId, TRANSCRIPT_FILE};
use crate::safe_write::PendingWrite;
use crate::store::{self, Store};

/// One line of a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    /// Meeting time the line starts at, in milliseconds. The file stores
    /// whole seconds, so this is always a multiple of 1000.
    pub timestamp_ms: u64,
    pub speaker: Speaker,
    /// The text after `Me: ` or `Them: `, exactly as it is in the file.
    pub text: String,
}

impl TranscriptLine {
    /// The timestamp as it is written in the file: `hh:mm:ss`.
    pub fn timestamp(&self) -> String {
        format_timestamp(self.timestamp_ms)
    }
}

/// A transcript read from disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The well-formed lines, in file order.
    pub lines: Vec<TranscriptLine>,
    /// How many lines were left out because they are not in the transcript
    /// format: malformed lines, and a last line that was cut off before its
    /// newline. Blank lines are not counted.
    pub skipped: usize,
}

impl Transcript {
    /// Parses the contents of a transcript file. Never fails: anything that
    /// is not a transcript line, including bytes that are not UTF-8, is
    /// skipped and counted in [`skipped`](Self::skipped).
    pub fn parse(bytes: &[u8]) -> Self {
        let mut transcript = Self::default();
        let (lines, tail) = split_lines(strip_bom(bytes).1);
        for line in lines {
            if is_blank(line) {
                continue;
            }
            match parse_line(line) {
                Some(parsed) => transcript.lines.push(TranscriptLine {
                    timestamp_ms: parsed.timestamp_ms,
                    speaker: parsed.speaker,
                    text: parsed.text.to_owned(),
                }),
                None => transcript.skipped += 1,
            }
        }
        if !is_blank(tail) {
            transcript.skipped += 1;
        }
        transcript
    }

    /// Reads and parses the transcript at `path`. A file that does not exist
    /// is an empty transcript; any other failure to read it is an error.
    pub fn read(path: &Path) -> Result<Self> {
        Ok(Self::parse(&read_or_empty(path)?))
    }

    /// The latest timestamp of any line, or `None` for an empty transcript.
    /// After the file is sorted this is the last line's.
    pub fn last_timestamp_ms(&self) -> Option<u64> {
        self.lines.iter().map(|line| line.timestamp_ms).max()
    }
}

/// How many well-formed lines the transcript at `path` has: the length of
/// [`Transcript::lines`], without building them. A file that does not exist
/// has none.
pub fn count_transcript_lines(path: &Path) -> Result<usize> {
    Ok(count_lines(&read_or_empty(path)?))
}

/// What [`TranscriptWriter::finalize`] left in the file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptStats {
    /// Well-formed lines, now in time order.
    pub lines: usize,
    /// Lines that are not in the transcript format, kept after the others.
    pub malformed: usize,
    /// Timestamp of the last well-formed line, in milliseconds.
    pub last_timestamp_ms: Option<u64>,
}

/// The line a segment is stored as, without the newline:
/// `[hh:mm:ss] Me: text` or `[hh:mm:ss] Them: text`.
///
/// The timestamp is the segment's start, cut to whole seconds. Line breaks,
/// tabs and other runs of whitespace in the text become single spaces, so a
/// segment is always one line. Returns `None` when no text is left; such a
/// segment is not written.
pub fn format_transcript_line(segment: &Segment) -> Option<String> {
    let text = flatten(&segment.text);
    if text.is_empty() {
        return None;
    }
    Some(format!(
        "[{}] {}: {}",
        format_timestamp(segment.start_ms),
        segment.speaker,
        text
    ))
}

/// Appends lines to one meeting's `transcript.md`.
///
/// Get one from [`Store::transcript_writer`]. It is a plain value with no
/// borrow of the store, so it can be moved to the thread that transcribes.
///
/// # What a crash can lose
///
/// [`append`](Self::append) returns after the line is written and the file
/// is synced to the disk itself (`F_FULLFSYNC` on macOS). A line whose
/// append returned is still there after a crash of the app, a forced quit, a
/// kernel panic or a power cut. Only the line being written at that moment
/// can be lost or cut short. Nothing is buffered in the writer, so dropping
/// it without [`finalize`](Self::finalize) loses nothing; the file is then
/// merely unsorted.
///
/// # Renamed folders
///
/// The writer is tied to the meeting's id, not to a path. It keeps the file
/// open, and an open file stays the same file when its folder is renamed, so
/// lines keep landing in the right place after [`Store::finish_meeting`] or
/// the user moves the folder. When the writer needs the path again (to sort,
/// or because the file was replaced or deleted by another program) it looks
/// the folder up by id. Appending, finalizing and finishing the meeting
/// therefore work in any order.
#[derive(Debug)]
pub struct TranscriptWriter {
    root: PathBuf,
    /// The lock of the store this writer came from. Held for each append and
    /// for the sort, so neither can interleave with a folder rename or with
    /// another writer's sort of the same file.
    lock: Arc<Mutex<()>>,
    id: MeetingId,
    /// Where the transcript was when the writer last looked.
    path: PathBuf,
    file: File,
}

impl TranscriptWriter {
    /// The meeting this writer appends to.
    pub fn meeting_id(&self) -> &MeetingId {
        &self.id
    }

    /// Where the transcript was when the writer last looked. The folder can
    /// have been renamed since; ask the store for the meeting to be sure.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one segment as one line and syncs it to disk before returning.
    ///
    /// Returns `Ok(false)`, writing nothing, when the segment has no text.
    /// Lines are stored in the order they arrive; [`finalize`](Self::finalize)
    /// puts them in time order.
    ///
    /// On an error the line may or may not be in the file (the write can
    /// succeed and the sync fail), so do not append the same segment again.
    pub fn append(&mut self, segment: &Segment) -> Result<bool> {
        let Some(mut line) = format_transcript_line(segment) else {
            return Ok(false);
        };
        line.push('\n');

        {
            let lock = Arc::clone(&self.lock);
            let _guard = store::lock(&lock);

            if let Err(err) = self.follow_file()
                && !is_linked(&self.file)
            {
                // The file is gone and the meeting cannot be found again.
                // (While the open file still exists somewhere, writing to it
                // keeps the line, wherever the folder went.)
                return Err(err);
            }
            // One write for the whole line, on a fresh line if the last one
            // was cut off by a crash or a full disk.
            write_lines(&mut self.file, &line).map_err(|err| Error::io(&self.path, err))?;
        }

        // Outside the lock: the sync is the slow part (a few milliseconds),
        // and a sort that starts now already sees the line and syncs its own
        // copy.
        self.file
            .sync_data()
            .map_err(|err| Error::io(&self.path, err))?;
        Ok(true)
    }

    /// Sorts the file by timestamp. Call it when recording stops.
    ///
    /// Lines with the same timestamp keep the order they arrived in. Lines
    /// that are not in the transcript format are kept, in their order, after
    /// all the others; a last line that was cut off counts as one of those
    /// unless what is left of it still reads as a full line. Blank lines are
    /// removed.
    ///
    /// The sorted text goes through the safe write helper, so a crash at any
    /// point leaves either the unsorted file or the sorted one, each with
    /// every line. A file that is already in order is not rewritten.
    ///
    /// The writer stays usable: a segment that arrives late can still be
    /// appended, and `finalize` can be called again.
    pub fn finalize(&mut self) -> Result<TranscriptStats> {
        let lock = Arc::clone(&self.lock);
        let _guard = store::lock(&lock);

        self.follow_file()?;
        let stats = finalize_file(&self.path)?;
        // The sort put a new file in place of the one this writer has open.
        // If reopening fails here, the next append finds the file by id.
        if !is_file_at(&self.file, &self.path)
            && let Ok(file) = open_for_append(&self.path)
        {
            self.file = file;
        }
        Ok(stats)
    }

    /// Makes `path` and `file` the meeting's transcript as it is now.
    ///
    /// Nothing to do while `path` still names the open file. Otherwise the
    /// folder was renamed, or another program replaced or deleted the file:
    /// the folder is found again by id and the file there is opened (and
    /// created if it is missing). The caller holds the store lock.
    fn follow_file(&mut self) -> Result<()> {
        if is_file_at(&self.file, &self.path) {
            return Ok(());
        }
        let path = store::find_folder(&self.root, &self.id)?.join(TRANSCRIPT_FILE);
        self.file = open_for_append(&path)?;
        self.path = path;
        Ok(())
    }
}

impl Store {
    /// Opens a writer that appends to this meeting's transcript.
    ///
    /// The writer holds no borrow of the store and is `Send`. See
    /// [`TranscriptWriter`] for what it guarantees. Fails with
    /// [`Error::MeetingNotFound`] if no folder has this id.
    pub fn transcript_writer(&self, id: &MeetingId) -> Result<TranscriptWriter> {
        let _guard = self.lock();
        let path = self.find_folder(id)?.join(TRANSCRIPT_FILE);
        let file = open_for_append(&path)?;
        Ok(TranscriptWriter {
            root: self.root().to_owned(),
            lock: self.lock_handle(),
            id: id.clone(),
            path,
            file,
        })
    }

    /// Sorts a meeting's transcript without a writer, exactly as
    /// [`TranscriptWriter::finalize`] does. For a transcript that a crash
    /// left unsorted. A missing transcript file is left missing.
    pub fn finalize_transcript(&self, id: &MeetingId) -> Result<TranscriptStats> {
        let _guard = self.lock();
        finalize_file(&self.find_folder(id)?.join(TRANSCRIPT_FILE))
    }

    /// Reads a meeting's transcript. Lines are in file order; see
    /// [`Transcript`]. A missing transcript file reads as empty.
    pub fn transcript(&self, id: &MeetingId) -> Result<Transcript> {
        let _guard = self.lock();
        Transcript::read(&self.find_folder(id)?.join(TRANSCRIPT_FILE))
    }

    /// How many lines [`transcript`](Self::transcript) would return, without
    /// building them.
    pub fn transcript_line_count(&self, id: &MeetingId) -> Result<usize> {
        let _guard = self.lock();
        count_transcript_lines(&self.find_folder(id)?.join(TRANSCRIPT_FILE))
    }
}

/// Sorts the transcript at `path` in place through a safe write. The caller
/// holds the store lock. A missing file is left missing.
pub(crate) fn finalize_file(path: &Path) -> Result<TranscriptStats> {
    let (pending, stats) = stage_sorted(path)?;
    if let Some(pending) = pending {
        pending.commit().map_err(|err| Error::io(path, err))?;
    }
    Ok(stats)
}

/// The first half of [`finalize_file`]: writes the sorted text to a temp file
/// next to `path` without replacing it yet. `None` when the file is already
/// sorted or does not exist. Separate so a test can stop here, which is what
/// a crash during the sort looks like.
fn stage_sorted(path: &Path) -> Result<(Option<PendingWrite>, TranscriptStats)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((None, TranscriptStats::default()));
        }
        Err(err) => return Err(Error::io(path, err)),
    };
    let (sorted, stats) = sort_transcript(&bytes);
    if sorted == bytes {
        return Ok((None, stats));
    }
    let pending = PendingWrite::stage(path, &sorted).map_err(|err| Error::io(path, err))?;
    Ok((Some(pending), stats))
}

/// The sorted form of a transcript file's contents.
///
/// Well-formed lines come first, by timestamp, with ties in file order. The
/// other lines follow in file order. Every line keeps its exact bytes and
/// ends in a newline; only blank lines are dropped.
fn sort_transcript(bytes: &[u8]) -> (Vec<u8>, TranscriptStats) {
    let (bom, body) = strip_bom(bytes);
    let (mut lines, tail) = split_lines(body);
    // A cut-off last line is kept like any other line.
    lines.push(tail);

    let mut timed: Vec<(u64, &[u8])> = Vec::new();
    let mut malformed: Vec<&[u8]> = Vec::new();
    for line in lines {
        if is_blank(line) {
            continue;
        }
        match parse_line(line) {
            Some(parsed) => timed.push((parsed.timestamp_ms, line)),
            None => malformed.push(line),
        }
    }
    // `sort_by_key` is stable: equal timestamps stay in arrival order.
    timed.sort_by_key(|&(timestamp_ms, _)| timestamp_ms);

    let stats = TranscriptStats {
        lines: timed.len(),
        malformed: malformed.len(),
        last_timestamp_ms: timed.last().map(|&(timestamp_ms, _)| timestamp_ms),
    };

    let mut sorted = Vec::with_capacity(bytes.len() + 1);
    sorted.extend_from_slice(bom);
    for line in timed.into_iter().map(|(_, line)| line).chain(malformed) {
        sorted.extend_from_slice(line);
        sorted.push(b'\n');
    }
    (sorted, stats)
}

/// One well-formed line, borrowing its text.
struct ParsedLine<'a> {
    timestamp_ms: u64,
    speaker: Speaker,
    text: &'a str,
}

/// Parses one line without its newline. `None` for anything that is not
/// exactly `[hh:mm:ss] Me: text` or `[hh:mm:ss] Them: text` with some text.
///
/// The timestamp ends at the first `] ` and the speaker at the first `: `
/// after it, so brackets and colons in the text are just text. A carriage
/// return left by a Windows editor is not part of the line.
fn parse_line(line: &[u8]) -> Option<ParsedLine<'_>> {
    let line = std::str::from_utf8(line).ok()?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (timestamp, rest) = line.strip_prefix('[')?.split_once("] ")?;
    let timestamp_ms = parse_timestamp(timestamp).ok()?;
    let (speaker, text) = rest.split_once(": ")?;
    let speaker = speaker.parse().ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(ParsedLine {
        timestamp_ms,
        speaker,
        text,
    })
}

/// [`Transcript::parse`] without building the lines.
fn count_lines(bytes: &[u8]) -> usize {
    let (lines, _tail) = split_lines(strip_bom(bytes).1);
    lines
        .into_iter()
        .filter(|line| parse_line(line).is_some())
        .count()
}

/// Splits file contents into the complete lines (each without its newline)
/// and what follows the last newline: empty for a file that ends cleanly,
/// otherwise a line that was cut off.
fn split_lines(bytes: &[u8]) -> (Vec<&[u8]>, &[u8]) {
    match bytes.iter().rposition(|&byte| byte == b'\n') {
        Some(end) => (
            bytes[..end].split(|&byte| byte == b'\n').collect(),
            &bytes[end + 1..],
        ),
        None => (Vec::new(), bytes),
    }
}

/// Splits off a UTF-8 byte order mark, which some editors put at the start
/// of a file. Sorting must not move it into the middle.
fn strip_bom(bytes: &[u8]) -> (&[u8], &[u8]) {
    const BOM: &[u8] = "\u{feff}".as_bytes();
    match bytes.strip_prefix(BOM) {
        Some(rest) => (&bytes[..BOM.len()], rest),
        None => (&[], bytes),
    }
}

fn is_blank(line: &[u8]) -> bool {
    line.iter().all(u8::is_ascii_whitespace)
}

/// Text as it goes on a line: every run of whitespace or control characters
/// (line breaks of every kind, tabs, no-break spaces) becomes one space, and
/// none is left at either end.
fn flatten(text: &str) -> String {
    let mut flat = String::with_capacity(text.len());
    let words = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty());
    for word in words {
        if !flat.is_empty() {
            flat.push(' ');
        }
        flat.push_str(word);
    }
    flat
}

fn read_or_empty(path: &Path) -> Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(Error::io(path, err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(speaker: Speaker, start_ms: u64, text: &str) -> Segment {
        Segment {
            speaker,
            start_ms,
            end_ms: start_ms + 1500,
            text: text.to_owned(),
        }
    }

    fn sorted(text: &str) -> (String, TranscriptStats) {
        let (bytes, stats) = sort_transcript(text.as_bytes());
        (String::from_utf8(bytes).unwrap(), stats)
    }

    /// The non-blank lines of some file contents, in a fixed order, to
    /// compare what two versions of a file hold.
    fn line_bag(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut lines: Vec<Vec<u8>> = bytes
            .split(|&byte| byte == b'\n')
            .filter(|line| !is_blank(line))
            .map(<[u8]>::to_vec)
            .collect();
        lines.sort();
        lines
    }

    #[test]
    fn a_segment_is_formatted_as_one_strict_line() {
        assert_eq!(
            format_transcript_line(&segment(Speaker::Them, 83_000, "Let's start.")).as_deref(),
            Some("[00:01:23] Them: Let's start.")
        );
        assert_eq!(
            format_transcript_line(&segment(Speaker::Me, 0, "Hi")).as_deref(),
            Some("[00:00:00] Me: Hi")
        );
    }

    #[test]
    fn the_timestamp_is_the_start_cut_to_whole_seconds() {
        let line = |ms| format_transcript_line(&segment(Speaker::Me, ms, "x")).unwrap();
        assert_eq!(line(999), "[00:00:00] Me: x");
        assert_eq!(line(59_999), "[00:00:59] Me: x");
        assert_eq!(line(3_600_000), "[01:00:00] Me: x");
        assert_eq!(line(100 * 3_600_000 + 5_000), "[100:00:05] Me: x");
    }

    #[test]
    fn whitespace_of_every_kind_becomes_single_spaces() {
        for (input, expected) in [
            ("one\ntwo", "one two"),
            ("one\r\ntwo\rthree", "one two three"),
            ("  padded \t\t text  ", "padded text"),
            (
                "line\u{2028}separator\u{2029}paragraph",
                "line separator paragraph",
            ),
            ("next\u{85}line", "next line"),
            ("no\u{a0}break", "no break"),
            ("form\u{c}feed\u{b}tab", "form feed tab"),
            ("nul\0escape\u{1b}bell\u{7}", "nul escape bell"),
            ("\n\n first\n\n\n second \n\n", "first second"),
            (
                "untouched: [brackets] and ünïcödé",
                "untouched: [brackets] and ünïcödé",
            ),
        ] {
            assert_eq!(flatten(input), expected, "{input:?}");
            let line = format_transcript_line(&segment(Speaker::Me, 0, input)).unwrap();
            assert!(!line.contains(['\n', '\r']), "{line:?}");
        }
    }

    #[test]
    fn a_segment_without_text_has_no_line() {
        for empty in ["", " ", "\n", " \t\r\n ", "\u{a0}\u{2028}", "\0"] {
            assert_eq!(
                format_transcript_line(&segment(Speaker::Me, 5_000, empty)),
                None,
                "{empty:?}"
            );
        }
    }

    #[test]
    fn a_formatted_line_parses_back_to_the_same_values() {
        for text in [
            "plain words",
            "[00:00:01] Me: a line inside a line",
            "] Them: starts like the end of a timestamp",
            "note: colons: everywhere: ",
            "[brackets] [more [nested]] ]] [[",
            "time is 10:30:00, ratio 3:1",
            "Me: Them: Me:",
            "ünïcödé, 日本語, emoji 🎙️",
            ":",
            "]",
        ] {
            for speaker in [Speaker::Me, Speaker::Them] {
                let line = format_transcript_line(&segment(speaker, 3_725_400, text)).unwrap();
                let parsed = parse_line(line.as_bytes()).unwrap_or_else(|| panic!("{line:?}"));
                assert_eq!(parsed.timestamp_ms, 3_725_000, "{line:?}");
                assert_eq!(parsed.speaker, speaker, "{line:?}");
                assert_eq!(parsed.text, flatten(text), "{line:?}");
            }
        }
    }

    #[test]
    fn lines_that_are_not_strict_are_rejected() {
        for line in [
            "",
            "just some words",
            "# Transcript",
            "[00:00:01]",
            "[00:00:01] ",
            "[00:00:01] Me",
            "[00:00:01] Me:",
            "[00:00:01] Me: ",
            "[00:00:01] Me:    ",
            "[00:00:01] Me:no space",
            "[00:00:01]Me: no space",
            "[00:00:01]  Me: two spaces",
            "[00:00:01] me: lower case",
            "[00:00:01] You: unknown speaker",
            "[00:00:01] Me : space before colon",
            " [00:00:01] Me: leading space",
            "00:00:01 Me: no brackets",
            "[0:00:01] Me: short hours",
            "[00:00:61] Me: bad seconds",
            "[00:01] Me: no hours",
            "[00:00:01.5] Me: fraction",
            "[00:00:0",
            "[00:00:01] Th",
        ] {
            assert!(parse_line(line.as_bytes()).is_none(), "{line:?}");
        }
        // Not UTF-8.
        assert!(parse_line(b"[00:00:01] Me: caf\xe9").is_none());
        assert!(parse_line(b"\xff\xfe").is_none());
    }

    #[test]
    fn a_carriage_return_before_the_newline_is_not_part_of_the_text() {
        let parsed = parse_line(b"[00:00:01] Me: saved on Windows\r").unwrap();
        assert_eq!(parsed.text, "saved on Windows");
    }

    #[test]
    fn parse_returns_lines_in_file_order_and_counts_the_rest() {
        let text = "[00:00:09] Them: second in time, first in file\n\
                    not a line\n\
                    \n\
                    [00:00:03] Me: first in time\n\
                    [00:00:03] Nobody: unknown speaker\n\
                    [00:00:12] Me: last\n";
        let transcript = Transcript::parse(text.as_bytes());

        let got: Vec<(u64, Speaker, &str)> = transcript
            .lines
            .iter()
            .map(|line| (line.timestamp_ms, line.speaker, line.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (9_000, Speaker::Them, "second in time, first in file"),
                (3_000, Speaker::Me, "first in time"),
                (12_000, Speaker::Me, "last"),
            ]
        );
        assert_eq!(transcript.skipped, 2);
        assert_eq!(transcript.last_timestamp_ms(), Some(12_000));
        assert_eq!(transcript.lines[0].timestamp(), "00:00:09");
        assert_eq!(count_lines(text.as_bytes()), 3);
    }

    #[test]
    fn a_last_line_without_its_newline_is_skipped_even_if_it_looks_whole() {
        let whole = "[00:00:01] Me: one\n[00:00:02] Them: two\n";
        assert_eq!(Transcript::parse(whole.as_bytes()).lines.len(), 2);
        assert_eq!(Transcript::parse(whole.as_bytes()).skipped, 0);

        // Every way of cutting the file short gives the complete lines
        // before the cut and nothing else.
        for cut in 0..whole.len() {
            let bytes = &whole.as_bytes()[..cut];
            let transcript = Transcript::parse(bytes);
            let complete = bytes.iter().filter(|&&byte| byte == b'\n').count();
            assert_eq!(transcript.lines.len(), complete, "cut at {cut}");
            assert_eq!(count_lines(bytes), complete, "cut at {cut}");
            let cut_mid_line = !bytes.is_empty() && !bytes.ends_with(b"\n");
            assert_eq!(
                transcript.skipped,
                usize::from(cut_mid_line),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn parse_never_fails_on_junk() {
        let inputs: [&[u8]; 9] = [
            b"",
            b"\n",
            b"\n\n\n",
            b"\xff\xfe\x00\x01\n\x80\x80",
            b"[[[[]]]]::::\n",
            b"[00:00:01] Me: fine\n\xf0\x9f\n[00:00:02] Them: also fine\n",
            b"\r\n\r\n",
            b"[",
            b"\0\0\0\0",
        ];
        for bytes in inputs {
            let transcript = Transcript::parse(bytes);
            assert_eq!(transcript.lines.len(), count_lines(bytes), "{bytes:?}");
        }
        let mixed = Transcript::parse(inputs[5]);
        assert_eq!((mixed.lines.len(), mixed.skipped), (2, 1));
        assert_eq!(Transcript::parse(b"").skipped, 0);
        assert_eq!(Transcript::parse(b"\n\n\n").skipped, 0);
    }

    #[test]
    fn windows_line_endings_and_a_byte_order_mark_are_read() {
        let text = "\u{feff}[00:00:01] Me: one\r\n[00:00:02] Them: two\r\n";
        let transcript = Transcript::parse(text.as_bytes());
        assert_eq!(transcript.skipped, 0);
        assert_eq!(transcript.lines[0].text, "one");
        assert_eq!(transcript.lines[1].text, "two");
        assert_eq!(count_lines(text.as_bytes()), 2);
    }

    #[test]
    fn sort_orders_by_time_and_keeps_arrival_order_for_ties() {
        let (text, stats) = sorted(
            "[00:00:09] Them: c\n\
             [00:00:03] Me: a, arrived first\n\
             [00:00:12] Me: d\n\
             [00:00:03] Them: a, arrived second\n\
             [00:00:03] Me: a, arrived third\n\
             [00:00:01] Them: start\n",
        );
        assert_eq!(
            text,
            "[00:00:01] Them: start\n\
             [00:00:03] Me: a, arrived first\n\
             [00:00:03] Them: a, arrived second\n\
             [00:00:03] Me: a, arrived third\n\
             [00:00:09] Them: c\n\
             [00:00:12] Me: d\n"
        );
        assert_eq!(
            stats,
            TranscriptStats {
                lines: 6,
                malformed: 0,
                last_timestamp_ms: Some(12_000),
            }
        );
    }

    #[test]
    fn sort_compares_times_not_text() {
        // As text, "100:00:00" sorts before "99:00:00".
        let (text, _) = sorted("[100:00:00] Me: later\n[99:00:00] Me: earlier\n");
        assert_eq!(text, "[99:00:00] Me: earlier\n[100:00:00] Me: later\n");
    }

    #[test]
    fn sort_keeps_malformed_lines_at_the_end_in_their_order() {
        let (text, stats) = sorted(
            "# My heading\n\
             [00:00:09] Them: b\n\
             \n\
             a stray note\n\
             [00:00:03] Me: a\n\
             [00:00:0",
        );
        assert_eq!(
            text,
            "[00:00:03] Me: a\n\
             [00:00:09] Them: b\n\
             # My heading\n\
             a stray note\n\
             [00:00:0\n"
        );
        assert_eq!(stats.lines, 2);
        assert_eq!(stats.malformed, 3);
        assert_eq!(stats.last_timestamp_ms, Some(9_000));
    }

    #[test]
    fn sort_of_an_empty_or_blank_file_is_empty() {
        assert_eq!(sorted(""), (String::new(), TranscriptStats::default()));
        assert_eq!(
            sorted("\n\n  \n"),
            (String::new(), TranscriptStats::default())
        );
    }

    #[test]
    fn sort_keeps_a_byte_order_mark_at_the_start_and_line_endings_as_they_are() {
        let (text, stats) = sorted("\u{feff}[00:00:02] Me: two\r\n[00:00:01] Them: one\r\n");
        assert_eq!(
            text,
            "\u{feff}[00:00:01] Them: one\r\n[00:00:02] Me: two\r\n"
        );
        assert_eq!(stats.lines, 2);
    }

    #[test]
    fn sort_never_loses_or_changes_a_line() {
        let inputs: [&[u8]; 6] = [
            b"[00:00:05] Me: b\n[00:00:01] Them: a\njunk\n\n[00:00:03] Me: cut of",
            b"\xff\xfe broken bytes\n[00:00:02] Me: fine\n[00:00:01] Me: caf\xe9\n",
            b"[00:00:02] Me: same\n[00:00:02] Me: same\n[00:00:01] Me: same\n",
            b"no newline at all",
            b"[00:10:00] Them: z\r\n[00:00:10] Me: y\r\n",
            b"",
        ];
        for bytes in inputs {
            let (sorted, stats) = sort_transcript(bytes);
            assert_eq!(line_bag(&sorted), line_bag(bytes));
            assert_eq!(stats.lines + stats.malformed, line_bag(bytes).len());
            // Sorting what is sorted changes nothing.
            assert_eq!(sort_transcript(&sorted).0, sorted);
            assert!(sorted.is_empty() || sorted.ends_with(b"\n"));
        }
    }

    #[test]
    fn a_crash_during_the_sort_leaves_the_unsorted_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TRANSCRIPT_FILE);
        let unsorted = "[00:00:09] Them: second\n[00:00:03] Me: first\nnot a line\n";
        fs::write(&path, unsorted).unwrap();

        // The sorted copy is written and synced, and then the process dies
        // before the rename: nothing runs, not even the cleanup.
        let (pending, stats) = stage_sorted(&path).unwrap();
        std::mem::forget(pending.expect("an unsorted file is staged"));
        assert_eq!(stats.lines, 2);

        assert_eq!(fs::read_to_string(&path).unwrap(), unsorted);
        assert_eq!(Transcript::read(&path).unwrap().lines.len(), 2);

        // The next sort, as recovery would run it, finishes the job.
        let stats = finalize_file(&path).unwrap();
        assert_eq!((stats.lines, stats.malformed), (2, 1));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[00:00:03] Me: first\n[00:00:09] Them: second\nnot a line\n"
        );
    }

    #[test]
    fn a_sorted_file_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TRANSCRIPT_FILE);
        fs::write(&path, "[00:00:03] Me: first\n[00:00:09] Them: second\n").unwrap();

        let (pending, stats) = stage_sorted(&path).unwrap();

        assert!(pending.is_none());
        assert_eq!(stats.lines, 2);
    }

    #[test]
    fn a_missing_file_is_empty_and_stays_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(TRANSCRIPT_FILE);

        assert_eq!(Transcript::read(&path).unwrap(), Transcript::default());
        assert_eq!(count_transcript_lines(&path).unwrap(), 0);
        assert_eq!(finalize_file(&path).unwrap(), TranscriptStats::default());
        assert!(!path.exists());
    }

    #[test]
    fn a_path_that_cannot_be_read_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be.
        let path = dir.path().join(TRANSCRIPT_FILE);
        fs::create_dir(&path).unwrap();

        assert!(matches!(Transcript::read(&path), Err(Error::Io { .. })));
        assert!(matches!(
            count_transcript_lines(&path),
            Err(Error::Io { .. })
        ));
        assert!(matches!(finalize_file(&path), Err(Error::Io { .. })));
    }
}
