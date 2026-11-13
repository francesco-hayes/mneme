//! The error type shared by every part of the crate.

use thiserror::Error;

/// Anything that can go wrong while capturing, converting or writing audio.
///
/// Variants carry text rather than the underlying error values so the type is
/// `Clone` and comparable: sources queue these for later collection, and the
/// recorder and the live session copy them into their summaries.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AudioError {
    /// The system has no default input device right now.
    #[error("no default input device")]
    NoInputDevice,

    /// The OS refused access to the device (microphone or audio capture
    /// permission).
    #[error("audio permission denied: {0}")]
    PermissionDenied(String),

    /// A device could not be opened or queried.
    #[error("audio device error: {0}")]
    Device(String),

    /// The device delivers a format this crate cannot convert.
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    /// A running stream reported an error or stopped delivering audio.
    #[error("audio stream error: {0}")]
    Stream(String),

    /// The resampler could not be built or failed while processing.
    #[error("resampler error: {0}")]
    Resample(String),

    /// `start` was called on a source that is already running.
    #[error("audio source is already running")]
    AlreadyRunning,

    /// The receiving end of the frame channel was dropped, so there is nowhere
    /// to deliver audio.
    #[error("the frame receiver was dropped")]
    SinkClosed,

    /// Reading or writing a WAV file failed.
    #[error("WAV error: {0}")]
    Wav(String),

    /// Any other failure, for example a source told to fail by a test.
    #[error("{0}")]
    Other(String),
}

impl From<hound::Error> for AudioError {
    fn from(err: hound::Error) -> Self {
        AudioError::Wav(err.to_string())
    }
}
