//! Crash-safe file replacement: temp file in the same folder, sync, rename.
//!
//! Every write of a whole file in this crate goes through [`safe_write`], so
//! a crash at any point leaves either the old file or the new one, never a
//! half-written mix. Files that grow one line at a time (the transcript, the
//! note timeline) are appended to directly instead.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Replaces the file at `path` with `contents`, or creates it.
///
/// The bytes are written to a hidden temp file next to `path`, flushed to
/// disk, and the temp file is then renamed over `path`. The rename is atomic,
/// so readers and a crash see the old contents or the new, nothing between.
/// On error the temp file is removed and `path` is left as it was.
pub fn safe_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    PendingWrite::stage(path, contents)?.commit()
}

/// A fully written and synced temp file that has not replaced its target yet.
///
/// Split from [`safe_write`] so tests can stop between the two steps, which
/// is what a crash at the worst moment looks like. Dropping it without
/// [`commit`](PendingWrite::commit) removes the temp file.
#[derive(Debug)]
pub(crate) struct PendingWrite {
    temp: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl PendingWrite {
    /// Writes `contents` to a new temp file beside `target` and syncs it.
    pub(crate) fn stage(target: &Path, contents: &[u8]) -> io::Result<Self> {
        let (temp, mut file) = create_temp(target)?;
        // From here on, drop removes the temp file if anything fails.
        let pending = Self {
            temp,
            target: target.to_owned(),
            committed: false,
        };
        file.write_all(contents)?;
        // On macOS this is F_FULLFSYNC: the bytes are on the disk itself.
        file.sync_all()?;
        Ok(pending)
    }

    /// Renames the temp file over the target.
    pub(crate) fn commit(mut self) -> io::Result<()> {
        fs::rename(&self.temp, &self.target)?;
        self.committed = true;
        sync_dir(parent_dir(&self.target));
        Ok(())
    }

    /// Where the temp file is.
    #[cfg(test)]
    pub(crate) fn temp_path(&self) -> &Path {
        &self.temp
    }
}

impl Drop for PendingWrite {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

/// True for file names [`safe_write`] uses for its temp files. A crash can
/// leave one behind; it is hidden, never read, and safe to delete.
pub fn is_temp_file_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(TEMP_SUFFIX)
}

const TEMP_SUFFIX: &str = ".tmp";

/// Flushes a folder's own entry list (new files, renames) to disk. Best
/// effort: the data is already safe, this only makes the rename itself
/// survive a power cut sooner.
pub(crate) fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Creates a new, empty temp file next to `target` and returns it with its
/// path: `.<file name>.<pid>-<n>.tmp`. The name is unique within the process
/// (counter) and across processes (pid); `create_new` catches leftovers.
fn create_temp(target: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "safe_write needs a path that ends in a file name",
        )
    })?;
    let dir = parent_dir(target);
    let pid = std::process::id();

    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(
            ".{}.{pid}-{n}{TEMP_SUFFIX}",
            name.to_string_lossy()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not find a free temp file name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_a_file_that_was_not_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");

        safe_write(&path, b"hello\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"hello\n");
        assert_eq!(names_in(dir.path()), ["notes.md"]);
    }

    #[test]
    fn replaces_the_contents_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        fs::write(&path, "a much longer first version\n").unwrap();

        safe_write(&path, b"short\n").unwrap();
        safe_write(&path, b"").unwrap();
        safe_write(&path, b"third\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"third\n");
        assert_eq!(names_in(dir.path()), ["notes.md"]);
    }

    #[test]
    fn a_stop_before_the_rename_leaves_the_old_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        fs::write(&path, "old\n").unwrap();

        let pending = PendingWrite::stage(&path, b"new\n").unwrap();
        let temp = pending.temp_path().to_owned();
        // A crash here never runs drop: the temp file stays, the target is
        // untouched.
        std::mem::forget(pending);

        assert_eq!(fs::read(&path).unwrap(), b"old\n");
        assert_eq!(fs::read(&temp).unwrap(), b"new\n");
        let temp_name = temp.file_name().unwrap().to_str().unwrap();
        assert!(is_temp_file_name(temp_name), "{temp_name}");
        assert!(!is_temp_file_name("notes.md"));

        // The next save still works with the leftover in place.
        safe_write(&path, b"newer\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"newer\n");
    }

    #[test]
    fn dropping_a_staged_write_removes_the_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.md");
        fs::write(&path, "old\n").unwrap();

        drop(PendingWrite::stage(&path, b"new\n").unwrap());

        assert_eq!(fs::read(&path).unwrap(), b"old\n");
        assert_eq!(names_in(dir.path()), ["notes.md"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("notes.md");

        let err = safe_write(&path, b"x").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(names_in(dir.path()).is_empty());
    }

    #[test]
    fn a_path_without_a_file_name_is_rejected() {
        let err = safe_write(Path::new("/"), b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
