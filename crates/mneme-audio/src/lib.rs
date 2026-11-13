//! Microphone and system audio capture as 16 kHz mono frames (PRD 01).
//!
//! Everything that consumes audio works against one trait, [`AudioSource`]:
//! start it with the meeting's shared start instant and a [`FrameSender`],
//! and it delivers [`AudioFrame`](mneme_core::AudioFrame)s (16 kHz mono
//! `f32`, labeled with a speaker, stamped in meeting time) until stopped.
//!
//! | Piece | What it is |
//! | --- | --- |
//! | [`AudioSource`] | The trait. Implemented by [`MicSource`], [`FakeSource`], and the system audio source. |
//! | [`MicSource`] | The default microphone via `cpal`, as `Speaker::Me`. Follows device changes. |
//! | [`FakeSource`] | Plays a `Vec<f32>` or a WAV file, in real time or at full speed. For tests. |
//! | [`Framer`] | The meeting clock: device buffers in, stamped fixed-size frames out. Every source uses it. |
//! | [`convert`] | Sample formats to `f32`, downmix to mono, resampling to 16 kHz. Pure functions. |
//! | [`WavWriter`] | Streaming 16 kHz mono 16-bit WAV writer; [`read_wav_16k_mono`] reads any WAV. |
//! | [`microphone_permission_status`], [`request_microphone_permission`] | Microphone permission. |
//!
//! ```
//! use std::time::Instant;
//! use mneme_audio::{AudioSource, FakeSource, frame_channel};
//! use mneme_core::Speaker;
//!
//! // Two sources, one clock, one stream of frames.
//! let mut me = FakeSource::from_samples(Speaker::Me, vec![0.0; 3_200]);
//! let mut them = FakeSource::from_samples(Speaker::Them, vec![0.0; 3_200]);
//! let (tx, rx) = frame_channel();
//! let start = Instant::now();
//! me.start(start, tx.clone())?;
//! them.start(start, tx)?;
//! let frames: Vec<_> = rx.iter().collect(); // ends once both have stopped
//! assert_eq!(frames.len(), 20);
//! # Ok::<(), mneme_audio::AudioError>(())
//! ```

mod capture;
pub mod convert;
mod error;
mod fake;
mod framer;
mod mic;
mod permission;
mod source;
#[cfg(test)]
mod testutil;
pub mod wav;

pub use convert::{StreamResampler, ToF32};
pub use error::AudioError;
pub use fake::{FakeSource, Pacing};
pub use framer::{FRAME_MS, FRAME_SAMPLES, Framer};
pub use mic::MicSource;
pub use permission::{
    PermissionStatus, microphone_permission_status, request_microphone_permission,
};
pub use source::{AudioSource, FrameReceiver, FrameSender, frame_channel};
pub use wav::{WavWriter, read_wav_16k_mono};
