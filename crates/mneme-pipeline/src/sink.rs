//! [`TranscriptSink`]: where a session puts its lines.

use std::error::Error;
use std::fmt;

use mneme_core::Segment;

/// Why a sink could not take a line or could not finalize.
///
/// It carries text only, so it can be copied into the session's status and
/// summary whatever the sink is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkError {
    message: String,
}

impl SinkError {
    /// An error with this message.
    pub fn new(message: impl fmt::Display) -> Self {
        SinkError {
            message: message.to_string(),
        }
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for SinkError {}

/// Receives the lines of one meeting's transcript as they are transcribed.
///
/// The session calls [`append`](Self::append) once per line from its
/// transcription thread, in the order the lines were transcribed (which is
/// not time order: two speakers share one worker), and
/// [`finalize`](Self::finalize) exactly once, after the last line, when the
/// session ends. The sink is dropped on that thread before
/// [`Session::stop`](crate::Session::stop) returns.
///
/// # What the session does with errors
///
/// - **`append` fails.** The line is not offered again: a sink that writes to
///   disk may have written it before failing, and a second copy would be
///   worse than a note that something went wrong. The session records the
///   segment and the error in its summary
///   ([`SessionSummary::sink_errors`](crate::SessionSummary::sink_errors))
///   and carries on with the next line.
/// - **`finalize` fails.** Not retried either; the error is in
///   [`SessionSummary::finalize_error`](crate::SessionSummary::finalize_error).
///   Every line whose `append` returned is still where the sink put it.
pub trait TranscriptSink: Send {
    /// Takes one line. `Ok(true)` if it was written, `Ok(false)` if the sink
    /// had nothing to write for it (a segment with no text). Only written
    /// lines count towards the session's line count.
    fn append(&mut self, segment: &Segment) -> Result<bool, SinkError>;

    /// Called once when no more lines will come. A sink that stores lines in
    /// arrival order puts them in time order here.
    fn finalize(&mut self) -> Result<(), SinkError>;
}

impl<T: TranscriptSink + ?Sized> TranscriptSink for Box<T> {
    fn append(&mut self, segment: &Segment) -> Result<bool, SinkError> {
        (**self).append(segment)
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        (**self).finalize()
    }
}

/// The store's transcript writer is a sink as it is: `append` writes and
/// syncs one line of `transcript.md`, `finalize` sorts the file.
///
/// This adapter is the only reason the crate depends on `mneme-store`. It
/// lives here and not with the caller because Rust only lets the crate that
/// owns the trait or the type write this `impl`; anywhere else it would need
/// a wrapper type, and every caller of the session wants exactly this one.
impl TranscriptSink for mneme_store::TranscriptWriter {
    fn append(&mut self, segment: &Segment) -> Result<bool, SinkError> {
        mneme_store::TranscriptWriter::append(self, segment).map_err(SinkError::new)
    }

    fn finalize(&mut self) -> Result<(), SinkError> {
        mneme_store::TranscriptWriter::finalize(self)
            .map(|_stats| ())
            .map_err(SinkError::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sink_error_shows_its_message() {
        let error = SinkError::new("disk full");
        assert_eq!(error.to_string(), "disk full");
        assert_eq!(error.message(), "disk full");
    }
}
