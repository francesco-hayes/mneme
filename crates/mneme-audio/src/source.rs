//! The [`AudioSource`] trait every capture source implements.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use mneme_core::{AudioFrame, Speaker};

use crate::AudioError;

/// Where a source sends its frames. Clone one sender into several sources to
/// get a single stream carrying both speakers.
pub type FrameSender = mpsc::Sender<AudioFrame>;

/// The receiving end of a [`FrameSender`].
pub type FrameReceiver = mpsc::Receiver<AudioFrame>;

/// Creates the channel sources deliver frames on.
pub fn frame_channel() -> (FrameSender, FrameReceiver) {
    mpsc::channel()
}

/// One channel of meeting audio: the microphone, system audio, or a fake.
///
/// Code that consumes audio only ever sees this trait, so it never knows
/// which OS API is behind a source.
///
/// # Contract
///
/// - Frames are 16 kHz mono `f32`, labeled with [`speaker`](Self::speaker),
///   with `start_ms` measured from the `start` instant passed to
///   [`start`](Self::start). Two sources started with the same instant share
///   one clock. Within a source `start_ms` never goes backwards; a gap in
///   `start_ms` is a real gap in the audio (the device changed or stalled).
///   Frames are [`FRAME_SAMPLES`](crate::FRAME_SAMPLES) long except the last
///   one before a gap or the end.
/// - A source drops its [`FrameSender`] when it stops, whether through
///   [`stop`](Self::stop), by failing, or by running out of input. Once every
///   sender is gone the receiver's iterator ends.
/// - Errors after a successful `start` never panic and never block; they are
///   queued and collected with [`take_errors`](Self::take_errors). A source
///   recovers by itself where it can (re-opening on a device change), so an
///   error does not by itself mean capture ended:
///   [`is_running`](Self::is_running) tells.
pub trait AudioSource: Send {
    /// The label this source puts on its frames.
    fn speaker(&self) -> Speaker;

    /// Starts capturing. `start` is the meeting's shared start instant, the
    /// zero of `start_ms`; it may be slightly in the past or the future.
    ///
    /// Returns once capture is running, or with the reason it could not
    /// start. Fails with [`AudioError::AlreadyRunning`] if already started.
    fn start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError>;

    /// Stops capturing and blocks until the last frame has been sent and the
    /// source's [`FrameSender`] is dropped. Does nothing if not running.
    fn stop(&mut self);

    /// `true` from a successful [`start`](Self::start) until the source
    /// stops, fails for good, or runs out of input.
    fn is_running(&self) -> bool;

    /// Removes and returns the errors that happened since the last call, in
    /// order. Empty when all is well.
    fn take_errors(&mut self) -> Vec<AudioError>;
}

/// State shared between a source handle and its worker thread: the running
/// flag behind [`AudioSource::is_running`] and the error queue behind
/// [`AudioSource::take_errors`].
#[derive(Debug, Default)]
pub(crate) struct SourceState {
    running: AtomicBool,
    errors: Mutex<Vec<AudioError>>,
}

impl SourceState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub(crate) fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::SeqCst);
    }

    pub(crate) fn report(&self, error: AudioError) {
        self.lock_errors().push(error);
    }

    pub(crate) fn take_errors(&self) -> Vec<AudioError> {
        std::mem::take(&mut *self.lock_errors())
    }

    fn lock_errors(&self) -> std::sync::MutexGuard<'_, Vec<AudioError>> {
        // The queue is a plain Vec, valid even if a holder panicked.
        self.errors.lock().unwrap_or_else(|e| e.into_inner())
    }
}
