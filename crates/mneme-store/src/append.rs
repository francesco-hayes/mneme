//! Files that grow one line at a time: the transcript and the note timeline.
//!
//! Both are appended to directly instead of going through
//! [`safe_write`](crate::safe_write::safe_write), so a crash loses at most
//! the lines being written at that moment. A line is complete once it ends
//! in a newline. Whatever follows the last newline was cut off mid-write, and
//! the next append starts on a fresh line after it.
//!
//! The caller decides when to sync: [`write_lines`] only writes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Error, Result};

/// Opens a file for appending, creating it if it is missing. Also readable,
/// to look at how the file ends.
pub(crate) fn open_for_append(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(|err| Error::io(path, err))
}

/// Appends `lines`, which must be whole lines each ending in a newline, to a
/// file opened with [`open_for_append`].
///
/// If the file ends in a line that was cut off by a crash or a full disk, a
/// newline goes in first so the new lines start on a line of their own. If
/// that check itself fails, the spare newline is written anyway; it does no
/// harm. Everything goes out in one write: the file is in append mode, so it
/// lands at the end in one piece.
///
/// Nothing is synced here.
pub(crate) fn write_lines(file: &mut File, lines: &str) -> io::Result<()> {
    if ends_with_newline(file).unwrap_or(false) {
        file.write_all(lines.as_bytes())
    } else {
        let mut fresh = String::with_capacity(lines.len() + 1);
        fresh.push('\n');
        fresh.push_str(lines);
        file.write_all(fresh.as_bytes())
    }
}

/// Whether the file is empty or its last byte is a newline, so that the next
/// line can go straight after it.
fn ends_with_newline(file: &mut File) -> io::Result<bool> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(true);
    }
    // In append mode the position only matters for reading; every write goes
    // to the end.
    file.seek(SeekFrom::Start(len - 1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] == b'\n')
}

/// Whether `path` names the very file that `file` has open. False after the
/// folder was renamed, or the file was replaced or deleted: a value that
/// keeps a file open uses this to notice and find the file again.
#[cfg(unix)]
pub(crate) fn is_file_at(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    match (file.metadata(), fs::metadata(path)) {
        (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub(crate) fn is_file_at(_file: &File, path: &Path) -> bool {
    path.is_file()
}

/// Whether the open file still has a name somewhere. False once it has been
/// deleted or replaced: writes to it would then go nowhere a person can see.
#[cfg(unix)]
pub(crate) fn is_linked(file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;

    file.metadata().is_ok_and(|meta| meta.nlink() > 0)
}

#[cfg(not(unix))]
pub(crate) fn is_linked(_file: &File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_go_to_the_end_of_a_file_that_is_created_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");

        let mut file = open_for_append(&path).unwrap();
        write_lines(&mut file, "one\n").unwrap();
        write_lines(&mut file, "two\nthree\n").unwrap();
        // A second handle appends after the first one's lines.
        let mut other = open_for_append(&path).unwrap();
        write_lines(&mut other, "four\n").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "one\ntwo\nthree\nfour\n"
        );
    }

    #[test]
    fn lines_after_a_cut_off_line_start_on_a_line_of_their_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        fs::write(&path, "whole\ncut of").unwrap();

        let mut file = open_for_append(&path).unwrap();
        write_lines(&mut file, "next\n").unwrap();
        write_lines(&mut file, "last\n").unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "whole\ncut of\nnext\nlast\n"
        );
    }

    #[test]
    fn an_open_file_is_followed_by_identity_not_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        let moved = dir.path().join("moved");
        let file = open_for_append(&path).unwrap();
        assert!(is_file_at(&file, &path));
        assert!(is_linked(&file));

        fs::rename(&path, &moved).unwrap();
        assert!(!is_file_at(&file, &path));
        assert!(is_file_at(&file, &moved));
        assert!(is_linked(&file));

        // Another file under the old name is not the open one.
        fs::write(&path, "").unwrap();
        assert!(!is_file_at(&file, &path));

        fs::remove_file(&moved).unwrap();
        assert!(!is_file_at(&file, &moved));
        #[cfg(unix)]
        assert!(!is_linked(&file));
    }
}
