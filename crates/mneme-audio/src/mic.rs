//! The microphone source: an input device through `cpal`, labeled
//! [`Speaker::Me`]. By default the system default input device; or one
//! chosen by its id (see [`input_devices`] and [`MicChoice`]), with the
//! default standing in while the chosen one is not connected. The choice can
//! change while a source records: it moves to the new device at its next
//! device check, within about half a second.
//!
//! The stream is opened in the device's own format (whatever rate, channel
//! count and sample format it currently runs at) and converted here. Asking
//! the device for 16 kHz instead would make cpal reconfigure the hardware for
//! every other app using it.
//!
//! # Device changes
//!
//! The capture thread (see [`crate::capture`]) re-opens the stream on the
//! device to use now (the chosen one if connected, else the default) when:
//!
//! - that device changes: the default changed, the chosen device was
//!   plugged in or out, or another device was chosen (checked twice a
//!   second; cpal's Core Audio backend only follows default *output*
//!   changes by itself);
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

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat, SizedSample};
use mneme_core::Speaker;

use crate::capture::{Backend, Capture, OpenStream, StreamEvents, Timing, make_ready};
use crate::convert::{ToF32, i24_to_f32};
use crate::permission::{PermissionStatus, microphone_permission_status};
use crate::source::SourceState;
#[cfg(target_os = "macos")]
use crate::system::OBJECT_NAME as SYSTEM_AUDIO_DEVICE_NAME;
/// No system audio device elsewhere: nothing to leave out.
#[cfg(not(target_os = "macos"))]
const SYSTEM_AUDIO_DEVICE_NAME: &str = "";
use crate::{AudioError, AudioSource, FrameSender};

/// An input device the microphone can be set to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    /// Stable across launches and reconnections: the Core Audio UID where
    /// available. What [`MicSource::with_device`] takes.
    pub id: String,
    /// The name macOS shows, such as "MacBook Pro Microphone".
    pub name: String,
    /// The system default input device right now.
    pub is_default: bool,
}

/// The input devices connected now, in the order Core Audio lists them.
/// Empty when there are none or they cannot be listed. Needs no permission
/// and opens nothing. Mneme's own system audio device is left out.
pub fn input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().map(|d| device_identity(&d));
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut list: Vec<InputDevice> = Vec::new();
    for device in devices {
        let id = device_identity(&device);
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| device.to_string());
        if name == SYSTEM_AUDIO_DEVICE_NAME || list.iter().any(|d| d.id == id) {
            continue;
        }
        list.push(InputDevice {
            is_default: default.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    list
}

/// The chosen input device, shared between whoever sets it (the app's
/// settings) and every [`MicSource`] made with it. A change reaches a source
/// that is recording at its next device check. Cloning shares the choice.
#[derive(Debug, Clone, Default)]
pub struct MicChoice(Arc<Mutex<Option<String>>>);

impl MicChoice {
    /// An [`InputDevice::id`], or `None` for the system default.
    pub fn new(device: Option<String>) -> Self {
        Self(Arc::new(Mutex::new(device)))
    }

    /// Chooses another device, or the default with `None`.
    pub fn set(&self, device: Option<String>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = device;
    }

    /// The device chosen now.
    pub fn get(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Which device to record: the chosen one if it is connected, otherwise
/// the default.
fn device_to_use(
    chosen: Option<&str>,
    connected: impl IntoIterator<Item = String>,
    default: Option<String>,
) -> Option<String> {
    chosen
        .and_then(|id| connected.into_iter().find(|c| c == id))
        .or(default)
}

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
    /// The chosen input device, read at every device check.
    choice: MicChoice,
}

impl MicSource {
    /// Creates a source for the default input device. Nothing is opened
    /// until [`start`](AudioSource::start).
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a source for the input device with this id (an
    /// [`InputDevice::id`]), or the default for `None`. While that device is
    /// not connected the default is recorded instead, and the source moves
    /// to the chosen one when it is plugged in.
    pub fn with_device(device: Option<String>) -> Self {
        Self::with_choice(MicChoice::new(device))
    }

    /// Creates a source that records whatever `choice` names, and follows
    /// it when it changes during the recording.
    pub fn with_choice(choice: MicChoice) -> Self {
        Self {
            choice,
            ..Self::default()
        }
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

        let choice = self.choice.clone();
        self.capture = Some(Capture::start(
            "mneme-mic",
            move || CpalBackend {
                host: cpal::default_host(),
                choice,
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
    /// The chosen device, read each time the device is looked up.
    choice: MicChoice,
}

impl CpalBackend {
    /// The device to record now: the chosen one if connected, else the
    /// default.
    fn device(&self) -> Option<cpal::Device> {
        let default = self.host.default_input_device();
        let Some(chosen) = self.choice.get() else {
            return default;
        };
        let connected: Vec<cpal::Device> = self
            .host
            .input_devices()
            .map(Iterator::collect)
            .unwrap_or_default();
        let id = device_to_use(
            Some(&chosen),
            connected.iter().map(device_identity),
            default.as_ref().map(device_identity),
        )?;
        connected
            .into_iter()
            .find(|d| device_identity(d) == id)
            .or(default)
    }
}

impl Backend for CpalBackend {
    type Stream = cpal::Stream;

    fn current_device(&mut self) -> Option<String> {
        self.device().map(|d| device_identity(&d))
    }

    fn open(&mut self, events: StreamEvents) -> Result<OpenStream<cpal::Stream>, AudioError> {
        let device = self.device().ok_or(AudioError::NoInputDevice)?;
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
            choice: MicChoice::default(),
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
    fn the_chosen_device_is_used_while_connected_and_the_default_otherwise() {
        let connected = || ["built-in".to_owned(), "usb".to_owned()];
        let default = || Some("built-in".to_owned());
        assert_eq!(
            device_to_use(Some("usb"), connected(), default()).as_deref(),
            Some("usb")
        );
        assert_eq!(
            device_to_use(Some("headset"), connected(), default()).as_deref(),
            Some("built-in")
        );
        assert_eq!(
            device_to_use(None, connected(), default()).as_deref(),
            Some("built-in")
        );
        assert_eq!(device_to_use(Some("headset"), connected(), None), None);
    }

    /// Lists the real devices: nothing is opened and no prompt is shown.
    #[test]
    fn input_devices_are_listed_with_at_most_one_default() {
        let devices = input_devices();
        println!("{devices:#?}");
        assert!(devices.iter().filter(|d| d.is_default).count() <= 1);
        assert!(
            devices
                .iter()
                .all(|d| !d.id.is_empty() && !d.name.is_empty())
        );
        assert!(devices.iter().all(|d| d.name != SYSTEM_AUDIO_DEVICE_NAME));
        // A chosen device that is connected is the one the backend opens,
        // and a new choice is followed without a new backend.
        if let Some(first) = devices.first() {
            let choice = MicChoice::new(Some(first.id.clone()));
            let mut backend = CpalBackend {
                host: cpal::default_host(),
                choice: choice.clone(),
            };
            assert_eq!(backend.current_device().as_deref(), Some(first.id.as_str()));
            if let Some(other) = devices.get(1) {
                choice.set(Some(other.id.clone()));
                assert_eq!(backend.current_device().as_deref(), Some(other.id.as_str()));
            }
            choice.set(None);
            let default = devices.iter().find(|d| d.is_default).map(|d| d.id.as_str());
            assert_eq!(backend.current_device().as_deref(), default);
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
