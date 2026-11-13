//! The microphone source: the system default input device through `cpal`,
//! labeled [`Speaker::Me`].
//!
//! The stream is opened in the device's own format (whatever rate, channel
//! count and sample format it currently runs at) and converted here. Asking
//! the device for 16 kHz instead would make cpal reconfigure the hardware for
//! every other app using it.
//!
//! # Device changes
//!
//! The capture thread (see [`crate::capture`]) re-opens the stream on the
//! current default input device when:
//!
//! - the default input device changes (checked twice a second; cpal's Core
//!   Audio backend only follows default *output* changes by itself);
//! - cpal reports a stream error, such as the device being unplugged or its
//!   sample rate being changed in Audio MIDI Setup;
//! - the stream delivers nothing for three seconds.
//!
//! The meeting clock carries on across the gap. If no input device exists at
//! that moment, the error is queued once and the source keeps trying every
//! second until one appears or it is stopped.
//!
//! Buffer overruns reported by the device are not queued as errors: a short
//! one is inaudible to the transcriber, and a long one shows up as a gap in
//! `start_ms`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat, SizedSample};
use mneme_core::Speaker;

use crate::capture::{Backend, Capture, OpenStream, StreamEvents, Timing, make_ready};
use crate::convert::{ToF32, i24_to_f32};
use crate::permission::{PermissionStatus, microphone_permission_status};
use crate::source::SourceState;
use crate::{AudioError, AudioSource, FrameSender};

/// Captures the system default microphone as [`Speaker::Me`] frames.
///
/// The process needs microphone permission (see
/// [`microphone_permission_status`]). If permission was never asked for,
/// macOS shows its prompt when capture starts and delivers silence until the
/// user answers; if it was denied, [`start`](AudioSource::start) fails with
/// [`AudioError::PermissionDenied`].
#[derive(Default)]
pub struct MicSource {
    state: Arc<SourceState>,
    capture: Option<Capture>,
}

impl MicSource {
    /// Creates a source for the default input device. Nothing is opened
    /// until [`start`](AudioSource::start).
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for MicSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicSource")
            .field("running", &self.is_running())
            .finish()
    }
}

impl AudioSource for MicSource {
    fn speaker(&self) -> Speaker {
        Speaker::Me
    }

    fn start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError> {
        // Refuses while capturing; clears away a capture thread that ended
        // by itself.
        make_ready(&mut self.capture, &self.state)?;

        match microphone_permission_status() {
            PermissionStatus::Denied => {
                return Err(AudioError::PermissionDenied(
                    "microphone access is turned off for this app in System Settings".to_owned(),
                ));
            }
            PermissionStatus::Restricted => {
                return Err(AudioError::PermissionDenied(
                    "microphone access is restricted on this Mac".to_owned(),
                ));
            }
            PermissionStatus::Granted | PermissionStatus::NotDetermined => {}
        }

        self.capture = Some(Capture::start(
            "mneme-mic",
            || CpalBackend {
                host: cpal::default_host(),
            },
            Speaker::Me,
            start,
            sink,
            Arc::clone(&self.state),
            Timing::default(),
        )?);
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut capture) = self.capture.take() {
            capture.stop();
        }
    }

    fn is_running(&self) -> bool {
        self.state.is_running()
    }

    fn take_errors(&mut self) -> Vec<AudioError> {
        self.state.take_errors()
    }
}

struct CpalBackend {
    host: cpal::Host,
}

impl Backend for CpalBackend {
    type Stream = cpal::Stream;

    fn current_device(&mut self) -> Option<String> {
        self.host
            .default_input_device()
            .map(|d| device_identity(&d))
    }

    fn open(&mut self, events: StreamEvents) -> Result<OpenStream<cpal::Stream>, AudioError> {
        let device = self
            .host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;
        let identity = device_identity(&device);
        let supported = device.default_input_config().map_err(open_error)?;
        let config = supported.config();

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, events),
            SampleFormat::I16 => build::<i16>(&device, config, events),
            SampleFormat::I32 => build::<i32>(&device, config, events),
            SampleFormat::I24 => build::<cpal::I24>(&device, config, events),
            SampleFormat::I8 => build::<i8>(&device, config, events),
            SampleFormat::I64 => build::<i64>(&device, config, events),
            SampleFormat::U8 => build::<u8>(&device, config, events),
            SampleFormat::U16 => build::<u16>(&device, config, events),
            SampleFormat::U24 => build::<cpal::U24>(&device, config, events),
            SampleFormat::U32 => build::<u32>(&device, config, events),
            SampleFormat::U64 => build::<u64>(&device, config, events),
            SampleFormat::F64 => build::<f64>(&device, config, events),
            other => Err(AudioError::UnsupportedFormat(format!(
                "input device delivers {other} samples"
            ))),
        }?;
        stream.play().map_err(open_error)?;

        Ok(OpenStream {
            stream,
            device: identity,
            sample_rate: config.sample_rate,
        })
    }
}

impl ToF32 for cpal::I24 {
    #[inline]
    fn to_f32(self) -> f32 {
        i24_to_f32(self.inner())
    }
}

impl ToF32 for cpal::U24 {
    #[inline]
    fn to_f32(self) -> f32 {
        (self.inner() as f32 - 8_388_608.0) / 8_388_608.0
    }
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    events: StreamEvents,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + ToF32,
{
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    let error_events = events.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let timestamp = info.timestamp();
                let reported = timestamp
                    .callback
                    .saturating_duration_since(timestamp.capture);
                let delay = capture_delay(reported, data.len() / channels.max(1), sample_rate);
                let now = Instant::now();
                let first_sample_at = now.checked_sub(delay).unwrap_or(now);
                events.data(data, channels, first_sample_at);
            },
            move |err: cpal::Error| {
                if let Some(error) = stream_error(&err) {
                    error_events.error(error);
                }
            },
            None,
        )
        .map_err(open_error)
}

/// A stable name for a device: its Core Audio UID where available.
fn device_identity(device: &cpal::Device) -> String {
    device
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| device.to_string())
}

/// How long before the callback ran the first sample of its buffer was
/// captured.
///
/// cpal reports the device's latency including the buffer itself. That
/// figure is used when it is plausible; otherwise the buffer's own duration,
/// which is the least it can be.
fn capture_delay(reported: Duration, frames: usize, sample_rate: u32) -> Duration {
    let buffer = Duration::from_micros(frames as u64 * 1_000_000 / u64::from(sample_rate.max(1)));
    if reported >= buffer && reported <= Duration::from_secs(1) {
        reported
    } else {
        buffer
    }
}

/// Maps an error from opening a device or stream.
fn open_error(err: cpal::Error) -> AudioError {
    match err.kind() {
        ErrorKind::PermissionDenied => AudioError::PermissionDenied(err.to_string()),
        ErrorKind::UnsupportedConfig => AudioError::UnsupportedFormat(err.to_string()),
        _ => AudioError::Device(err.to_string()),
    }
}

/// Maps an error from a running stream. `None` for conditions that need no
/// action: the stream is still delivering audio.
fn stream_error(err: &cpal::Error) -> Option<AudioError> {
    match err.kind() {
        ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged => None,
        ErrorKind::PermissionDenied => Some(AudioError::PermissionDenied(err.to_string())),
        _ => Some(AudioError::Stream(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Framer;

    #[test]
    fn a_new_source_is_idle_and_labeled_me() {
        let mut mic = MicSource::new();
        assert_eq!(mic.speaker(), Speaker::Me);
        assert!(!mic.is_running());
        assert!(mic.take_errors().is_empty());
        // Stopping a source that never started is a no-op.
        mic.stop();
    }

    #[test]
    fn mic_source_is_a_boxable_audio_source() {
        let boxed: Box<dyn AudioSource> = Box::new(MicSource::new());
        assert_eq!(boxed.speaker(), Speaker::Me);
    }

    /// Reads the default input device's identity and format the way `open`
    /// does, without building a stream: nothing is captured and macOS shows
    /// no prompt. Passes trivially on a machine with no input device.
    #[test]
    fn the_default_input_device_can_be_described() {
        let mut backend = CpalBackend {
            host: cpal::default_host(),
        };
        let Some(identity) = backend.current_device() else {
            println!("no default input device");
            return;
        };
        assert!(!identity.is_empty());
        // Asking twice gives the same answer, which is what change detection
        // relies on.
        assert_eq!(backend.current_device().as_deref(), Some(identity.as_str()));

        let device = backend.host.default_input_device().unwrap();
        match device.default_input_config() {
            Ok(config) => {
                println!(
                    "default input {identity}: {} Hz, {} channel(s), {}",
                    config.sample_rate(),
                    config.channels(),
                    config.sample_format()
                );
                assert!(config.sample_rate() > 0);
                assert!(config.channels() > 0);
                // The framer accepts whatever the device runs at.
                let (tx, _rx) = crate::frame_channel();
                Framer::new(Speaker::Me, Instant::now(), tx, config.sample_rate(), 1).unwrap();
            }
            Err(err) => println!("default input {identity} has no usable format: {err}"),
        }
    }

    #[test]
    fn capture_delay_uses_plausible_device_latency() {
        // 512 frames at 48 kHz last 10.67 ms.
        let reported = Duration::from_millis(18);
        assert_eq!(capture_delay(reported, 512, 48_000), reported);
    }

    #[test]
    fn capture_delay_falls_back_to_the_buffer_duration() {
        let buffer = Duration::from_micros(512 * 1_000_000 / 48_000);
        assert_eq!(capture_delay(Duration::ZERO, 512, 48_000), buffer);
        assert_eq!(capture_delay(Duration::from_millis(2), 512, 48_000), buffer);
        assert_eq!(
            capture_delay(Duration::from_secs(9_000), 512, 48_000),
            buffer
        );
        assert_eq!(
            capture_delay(Duration::ZERO, 512, 0),
            Duration::from_secs(512)
        );
    }

    #[test]
    fn packed_24_bit_samples_convert() {
        assert_eq!(cpal::I24::new(-8_388_608).unwrap().to_f32(), -1.0);
        assert_eq!(cpal::I24::new(4_194_304).unwrap().to_f32(), 0.5);
        assert_eq!(cpal::U24::new(8_388_608).unwrap().to_f32(), 0.0);
        assert_eq!(cpal::U24::new(0).unwrap().to_f32(), -1.0);
    }

    #[test]
    fn benign_stream_conditions_are_not_errors() {
        for kind in [
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
            ErrorKind::DeviceChanged,
        ] {
            assert_eq!(stream_error(&cpal::Error::new(kind)), None);
        }
        assert!(matches!(
            stream_error(&cpal::Error::new(ErrorKind::DeviceNotAvailable)),
            Some(AudioError::Stream(_))
        ));
        assert!(matches!(
            stream_error(&cpal::Error::new(ErrorKind::StreamInvalidated)),
            Some(AudioError::Stream(_))
        ));
    }

    #[test]
    fn open_errors_keep_their_meaning() {
        assert!(matches!(
            open_error(cpal::Error::new(ErrorKind::PermissionDenied)),
            AudioError::PermissionDenied(_)
        ));
        assert!(matches!(
            open_error(cpal::Error::new(ErrorKind::UnsupportedConfig)),
            AudioError::UnsupportedFormat(_)
        ));
        assert!(matches!(
            open_error(cpal::Error::new(ErrorKind::DeviceBusy)),
            AudioError::Device(_)
        ));
    }
}
