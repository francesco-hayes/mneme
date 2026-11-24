//! The system audio source: everything the Mac is playing, captured with a
//! Core Audio process tap and labeled [`Speaker::Them`].
//!
//! # How it captures
//!
//! macOS 14.2 added *process taps*: an audio object that receives a copy of
//! what processes send to the output hardware. A tap cannot be read
//! directly. It is placed in a private *aggregate device*, where it appears
//! as an input stream, and that device is read with an IO proc like any
//! input device. So starting capture means, in order:
//!
//! 1. create a tap described by a `CATapDescription`: every process mixed
//!    down to stereo, minus this process, private, **unmuted** (the user
//!    keeps hearing the call);
//! 2. create a private aggregate device whose only member is the tap;
//! 3. register an IO proc on the aggregate device and start it.
//!
//! [`session`] owns those resources and guarantees their teardown; [`hal`]
//! holds the Core Audio calls; [`ioproc`] is the real-time callback;
//! [`format`] reads the tap's stream format. The audio then takes the same
//! road as the microphone's: the callback downmixes to mono and hands the
//! buffer to the capture thread (see [`crate::capture`]), which resamples to
//! 16 kHz and stamps frames on the meeting clock.
//!
//! # Output device changes
//!
//! The tap is global: it is not tied to an output device. What a running
//! global tap does when the output device or its sample rate changes is not
//! documented, and it has not been observed here. Chromium rebuilds its tap
//! on every change of the default output device, and a developer forum
//! report says a tap that stopped delivering real audio only recovered when
//! both the tap and the aggregate device were destroyed and created again.
//! So the tap is not kept across a change. The capture thread checks the
//! default output device and its sample rate twice a second; when either
//! changes (headphones plugged in, AirPods connecting or switching to their
//! call profile) it destroys the tap and the aggregate device and builds new
//! ones. The meeting clock carries on: frames after the switch are stamped
//! with the time they were really captured, and the gap in `start_ms` is the
//! gap in the audio. A stream that stops delivering for three seconds, or
//! reports an error, is rebuilt the same way.
//!
//! # Permission
//!
//! Reading a tap needs the *System Audio Recording* permission, and the
//! app's `Info.plist` must carry `NSAudioCaptureUsageDescription`. Apple's
//! documentation says the system prompts "the first time you start recording
//! from an aggregate device that contains a tap"; Chromium's source says the
//! prompt appears when the IO proc is registered and that the call waits for
//! the answer. **Without the permission nothing fails**: every call succeeds
//! and the tap delivers silence. The one thing reported to be refused is
//! updating the tap's description (Chromium probes for the permission that
//! way), so [`start`](AudioSource::start) tries exactly that, after IO has
//! been started so that the prompt has had its chance either way, and fails
//! with [`AudioError::PermissionDenied`] if it is refused. See
//! [`system_audio_permission_status`](crate::system_audio_permission_status)
//! for what can be known beforehand.
//!
//! # Swapping the API
//!
//! Nothing outside this module knows that taps are used: consumers see
//! [`AudioSource`]. A ScreenCaptureKit source (the fallback named in PRD 01)
//! would be another implementation of [`capture::Backend`](crate::capture::Backend)
//! used by the same [`Capture`] thread, and would need to supply:
//!
//! - `open`: build and start an `SCStream` with `capturesAudio` and
//!   `excludesCurrentProcessAudio` set on its `SCStreamConfiguration`, and
//!   an output handler for `SCStreamOutputTypeAudio`. The handle it returns
//!   must stop the stream when dropped.
//! - a callback that pulls the `AudioBufferList` out of each sample buffer
//!   and passes it to [`StreamEvents::data`] or
//!   [`StreamEvents::data_planar`], with the buffer's presentation time
//!   converted to an `Instant` as the capture time. The stream's
//!   `sampleRate` and `channelCount` are chosen by the caller, so the rate
//!   to report from `open` is known.
//! - stream errors (the delegate's stopped-with-error callback) forwarded to
//!   [`StreamEvents::error`], which makes the capture thread re-open.
//! - `current_device`: a constant. ScreenCaptureKit follows output devices by
//!   itself, so there is nothing to re-open for.
//! - permission helpers on `CGPreflightScreenCaptureAccess` and
//!   `CGRequestScreenCaptureAccess`. That is the Screen Recording
//!   permission, which macOS re-confirms periodically; avoiding it is why
//!   taps were chosen.
//!
//! The `Framer`, the conversion code, the capture thread and every consumer
//! would stay as they are.

#[cfg(target_os = "macos")]
mod format;
#[cfg(target_os = "macos")]
mod hal;

/// The name of the private aggregate device a system audio capture creates.
#[cfg(target_os = "macos")]
pub(crate) use hal::OBJECT_NAME;
#[cfg(target_os = "macos")]
mod ioproc;
#[cfg(target_os = "macos")]
mod session;

use std::sync::Arc;
use std::time::Instant;

use mneme_core::Speaker;

use crate::capture::{Capture, make_ready};
use crate::source::SourceState;
use crate::{AudioError, AudioSource, FrameSender};

/// Captures everything the Mac plays, except this process's own sound, as
/// [`Speaker::Them`] frames. macOS 14.4 or later.
///
/// The user keeps hearing what is captured.
///
/// # Output device changes
///
/// When the default output device or its sample rate changes (headphones
/// plugged in, AirPods connecting), the source rebuilds its tap on the new
/// device and carries on. Nothing is reported as an error. The meeting clock
/// is kept: the short stretch without audio shows as a gap in `start_ms`.
/// With no output device at all, [`AudioError::NoOutputDevice`] is queued
/// once and the source keeps trying until one appears or it is stopped.
///
/// # Permission
///
/// The process needs the System Audio Recording permission, and the app's
/// `Info.plist` needs `NSAudioCaptureUsageDescription`. The first
/// [`start`](AudioSource::start) makes macOS show its prompt and may **block
/// while the prompt is on screen**, so call it off the UI thread. If the
/// permission is missing, `start` fails with
/// [`AudioError::PermissionDenied`]; if macOS has not been asked before, it
/// may fail that way while the prompt is still open, and succeed when called
/// again after the user allowed it.
///
/// A recording made without the permission is not an error to macOS: it is
/// silence. `start` detects the missing permission; silence that begins
/// later (the permission revoked mid-recording) cannot be told from a quiet
/// system.
///
/// # Not this process, but its helpers
///
/// Only the calling process is left out. An app whose sound is played by a
/// helper process (as a web view's may be) must name those with
/// [`excluding_process`](Self::excluding_process).
#[derive(Default)]
pub struct SystemAudioSource {
    state: Arc<SourceState>,
    capture: Option<Capture>,
    excluded: Vec<u32>,
    skip_access_check: bool,
}

impl SystemAudioSource {
    /// Creates a source for all system audio except this process's. Nothing
    /// is created in the audio system until [`start`](AudioSource::start).
    pub fn new() -> Self {
        Self::default()
    }

    /// Also leaves out the audio of the process with this PID.
    ///
    /// The process is looked up each time the tap is built (at start and
    /// after every device change). One that is not playing or recording
    /// audio at that moment is unknown to Core Audio and cannot be excluded.
    pub fn excluding_process(mut self, pid: u32) -> Self {
        self.excluded.push(pid);
        self
    }

    /// Starts capture without checking that macOS lets this process read the
    /// tap.
    ///
    /// A diagnostic switch. The check is what turns a missing permission into
    /// an error; without it a missing permission gives a silent recording.
    /// Use it only to find out whether the check itself is at fault.
    pub fn without_access_check(mut self) -> Self {
        self.skip_access_check = true;
        self
    }
}

impl std::fmt::Debug for SystemAudioSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemAudioSource")
            .field("running", &self.is_running())
            .field("excluded", &self.excluded)
            .finish()
    }
}

impl AudioSource for SystemAudioSource {
    fn speaker(&self) -> Speaker {
        Speaker::Them
    }

    fn start(&mut self, start: Instant, sink: FrameSender) -> Result<(), AudioError> {
        // Refuses while capturing; clears away a capture thread that ended
        // by itself.
        make_ready(&mut self.capture, &self.state)?;
        self.capture = Some(imp::start(
            &self.excluded,
            !self.skip_access_check,
            start,
            sink,
            Arc::clone(&self.state),
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

/// Opens a tap for a moment and closes it again: the only way, with public
/// API, to make macOS ask for the System Audio Recording permission and to
/// learn whether it is there. See
/// [`request_system_audio_permission`](crate::request_system_audio_permission).
pub(crate) fn probe_access() -> Result<(), AudioError> {
    imp::probe_access()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::hal::CoreAudioHal;
    use super::session::{Hal, TapConfig, TapSession};
    use super::*;
    use crate::capture::{Backend, OpenStream, StreamEvents, Timing};

    /// Opens tap sessions for the capture thread.
    pub(super) struct TapBackend<H: Hal> {
        pub hal: H,
        pub config: TapConfig,
    }

    impl<H: Hal> Backend for TapBackend<H> {
        type Stream = TapSession<H>;

        fn current_device(&mut self) -> Option<String> {
            self.hal.default_output().map(|device| device.identity())
        }

        fn open(&mut self, events: StreamEvents) -> Result<OpenStream<TapSession<H>>, AudioError> {
            // Read before the tap is built: if the device changes while it
            // is being built, the next check sees a difference and rebuilds.
            let device = self
                .hal
                .default_output()
                .ok_or(AudioError::NoOutputDevice)?
                .identity();
            let stream = TapSession::open(self.hal.clone(), &self.config, events)?;
            Ok(OpenStream {
                device,
                sample_rate: stream.sample_rate(),
                stream,
            })
        }
    }

    pub(super) fn config(excluded: &[u32], check_access: bool) -> TapConfig {
        let mut excluded_pids = vec![std::process::id() as i32];
        excluded_pids.extend(excluded.iter().map(|pid| *pid as i32));
        TapConfig {
            excluded_pids,
            check_access,
        }
    }

    pub(super) fn start_with<H: Hal + Send + 'static>(
        hal: H,
        config: TapConfig,
        start: Instant,
        sink: FrameSender,
        state: Arc<SourceState>,
        timing: Timing,
    ) -> Result<Capture, AudioError> {
        Capture::start(
            "mneme-system-audio",
            move || TapBackend { hal, config },
            Speaker::Them,
            start,
            sink,
            state,
            timing,
        )
    }

    pub(super) fn start(
        excluded: &[u32],
        check_access: bool,
        start: Instant,
        sink: FrameSender,
        state: Arc<SourceState>,
    ) -> Result<Capture, AudioError> {
        start_with(
            CoreAudioHal,
            config(excluded, check_access),
            start,
            sink,
            state,
            Timing::default(),
        )
    }

    pub(super) fn probe_access() -> Result<(), AudioError> {
        TapSession::open(CoreAudioHal, &config(&[], true), StreamEvents::detached()).map(drop)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    fn unavailable() -> AudioError {
        AudioError::Device("system audio capture is only implemented for macOS".to_owned())
    }

    pub(super) fn start(
        _excluded: &[u32],
        _check_access: bool,
        _start: Instant,
        _sink: FrameSender,
        _state: Arc<SourceState>,
    ) -> Result<Capture, AudioError> {
        Err(unavailable())
    }

    pub(super) fn probe_access() -> Result<(), AudioError> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_source_is_idle_and_labeled_them() {
        let mut source = SystemAudioSource::new();
        assert_eq!(source.speaker(), Speaker::Them);
        assert!(!source.is_running());
        assert!(source.take_errors().is_empty());
        // Stopping a source that never started is a no-op.
        source.stop();
    }

    #[test]
    fn system_audio_source_is_a_boxable_audio_source() {
        let boxed: Box<dyn AudioSource> = Box::new(
            SystemAudioSource::new()
                .excluding_process(4_242)
                .without_access_check(),
        );
        assert_eq!(boxed.speaker(), Speaker::Them);
        assert!(!boxed.is_running());
    }

    #[cfg(target_os = "macos")]
    mod with_a_fake_hal {
        use std::thread;
        use std::time::Duration;

        use mneme_core::AudioFrame;

        use super::super::imp::{config, start_with};
        use super::super::ioproc::tests::{BufferList, bytes_of, call_io_proc, timestamp};
        use super::super::session::tests::FakeHal;
        use super::*;
        use crate::capture::Timing;
        use crate::testutil::{sine, zero_crossing_frequency};
        use crate::{FRAME_SAMPLES, FrameReceiver, frame_channel};

        fn fast() -> Timing {
            Timing {
                poll: Duration::from_millis(5),
                stall: Duration::from_secs(60),
                retry: Duration::from_millis(5),
            }
        }

        fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !condition() {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                thread::sleep(Duration::from_millis(2));
            }
        }

        fn start_capture(
            hal: &FakeHal,
            timing: Timing,
        ) -> (Result<Capture, AudioError>, Arc<SourceState>, FrameReceiver) {
            let (tx, rx) = frame_channel();
            let state = SourceState::new();
            let capture = start_with(
                hal.clone(),
                config(&[], true),
                Instant::now(),
                tx,
                Arc::clone(&state),
                timing,
            );
            (capture, state, rx)
        }

        /// Plays `seconds` of a 440 Hz tone into the running IO proc as the
        /// interleaved stereo float buffers of 10 ms a tap delivers. Returns
        /// how many callbacks found an IO proc running.
        fn play(hal: &FakeHal, rate: u32, seconds: f64) -> usize {
            let tone = sine(440.0, rate, (f64::from(rate) * seconds) as usize);
            let mut delivered = 0;
            for chunk in tone.chunks(rate as usize / 100) {
                let stereo: Vec<f32> = chunk.iter().flat_map(|s| [*s, *s]).collect();
                let bytes = bytes_of(&stereo, f32::to_ne_bytes);
                let list = BufferList::over([&bytes[..]], 2);
                let time = timestamp(Duration::ZERO);
                if hal.with_running_io_proc(|context| call_io_proc(context, list.as_ptr(), &time)) {
                    delivered += 1;
                }
            }
            delivered
        }

        fn total_samples(frames: &[AudioFrame]) -> usize {
            frames.iter().map(|f| f.samples.len()).sum()
        }

        const TEARDOWN_ORDER: [&str; 4] = [
            "stop",
            "destroy_io_proc",
            "destroy_aggregate",
            "destroy_tap",
        ];

        /// The calls in `log` that tear something down, without their IDs.
        fn teardown_calls(log: &[String]) -> Vec<&str> {
            log.iter()
                .map(|call| call.split(' ').next().unwrap())
                .filter(|name| TEARDOWN_ORDER.contains(name))
                .collect()
        }

        #[test]
        fn tap_audio_reaches_the_sink_as_them_frames() {
            let hal = FakeHal::new();
            let (capture, state, rx) = start_capture(&hal, fast());
            let mut capture = capture.unwrap();
            assert!(state.is_running());
            assert_eq!(hal.live(), [1, 1, 1, 1]);

            assert_eq!(play(&hal, 48_000, 1.0), 100);
            capture.stop();

            assert!(!state.is_running());
            assert_eq!(hal.live(), [0, 0, 0, 0], "stop left something behind");
            assert_eq!(teardown_calls(&hal.log()), TEARDOWN_ORDER);

            let frames: Vec<AudioFrame> = rx.iter().collect();
            assert_eq!(total_samples(&frames), 16_000);
            assert!(frames.iter().all(|f| f.speaker == Speaker::Them));
            for pair in frames.windows(2) {
                assert_eq!(pair[1].start_ms, pair[0].start_ms + 20);
            }
            let samples: Vec<f32> = frames.iter().flat_map(|f| f.samples.clone()).collect();
            let frequency = zero_crossing_frequency(&samples[1_600..14_400], 16_000);
            assert!((frequency - 440.0).abs() < 1.0, "{frequency} Hz");
            assert!(state.take_errors().is_empty());
        }

        #[test]
        fn this_process_is_left_out_of_the_tap() {
            let hal = FakeHal::new();
            let own_pid = std::process::id() as i32;
            hal.state()
                .process_objects
                .extend([(own_pid, 301), (77, 302)]);
            let (tx, _rx) = frame_channel();
            let capture = start_with(
                hal.clone(),
                config(&[77, 78], true),
                Instant::now(),
                tx,
                SourceState::new(),
                fast(),
            )
            .unwrap();
            // 78 is not an audio client, so there is nothing to exclude.
            assert_eq!(hal.state().excluded, [301, 302]);
            drop(capture);
        }

        #[test]
        fn an_output_device_change_rebuilds_the_tap_and_keeps_the_clock() {
            let hal = FakeHal::new();
            let (capture, state, rx) = start_capture(&hal, fast());
            let mut capture = capture.unwrap();
            assert_eq!(play(&hal, 48_000, 0.5), 50);

            // Headphones running at 44.1 kHz become the default output.
            hal.state().log.clear();
            hal.set_output("AirPods", 44_100.0);
            wait_until("the rebuilt tap", || {
                hal.log().iter().filter(|c| c.starts_with("start")).count() == 1
            });
            // The old tap and device were destroyed, in order, before the
            // new ones were created; exactly one of each exists.
            let log = hal.log();
            assert_eq!(teardown_calls(&log), TEARDOWN_ORDER);
            let last_destroy = log
                .iter()
                .rposition(|c| c.starts_with("destroy_tap"))
                .unwrap();
            let first_create = log
                .iter()
                .position(|c| c.starts_with("create_tap"))
                .unwrap();
            assert!(last_destroy < first_create, "{log:?}");
            assert_eq!(hal.live(), [1, 1, 1, 1]);

            thread::sleep(Duration::from_millis(30));
            assert_eq!(play(&hal, 44_100, 0.5), 50);
            capture.stop();
            assert_eq!(hal.live(), [0, 0, 0, 0]);

            let frames: Vec<AudioFrame> = rx.iter().collect();
            // Half a second from each device, both at 16 kHz now.
            assert_eq!(total_samples(&frames), 16_000);
            // One gap, at the switch; time never runs backwards.
            let mut gaps = 0;
            for pair in frames.windows(2) {
                let end = pair[0].start_ms + (pair[0].samples.len() as u64).div_ceil(16);
                assert!(pair[1].start_ms >= end, "frames overlap");
                if pair[1].start_ms > end || pair[0].samples.len() < FRAME_SAMPLES {
                    gaps += 1;
                }
            }
            assert!(gaps <= 1, "{gaps} gaps");
            // The second half sits where it was captured on the meeting
            // clock (the meeting began under five seconds ago), not at zero
            // and not directly after the first half's last sample.
            let after_switch = frames
                .iter()
                .find(|f| f.start_ms > 500)
                .expect("a later frame");
            assert!(after_switch.start_ms < 5_000);
            // Following a device is not an error.
            assert!(state.take_errors().is_empty());
        }

        #[test]
        fn a_sample_rate_change_on_the_same_device_rebuilds_the_tap() {
            let hal = FakeHal::new();
            let (capture, _state, rx) = start_capture(&hal, fast());
            let mut capture = capture.unwrap();
            play(&hal, 48_000, 0.2);

            // AirPods dropping to their call profile: same device, new rate.
            hal.state().log.clear();
            hal.set_output("BuiltInSpeakerDevice", 24_000.0);
            wait_until("the rebuilt tap", || {
                hal.log().iter().any(|c| c.starts_with("start"))
            });
            play(&hal, 24_000, 0.2);
            capture.stop();

            assert_eq!(total_samples(&rx.iter().collect::<Vec<_>>()), 6_400);
            assert_eq!(hal.live(), [0, 0, 0, 0]);
        }

        #[test]
        fn a_missing_permission_fails_start_and_leaves_nothing_behind() {
            let hal = FakeHal::new();
            hal.state().fail.insert("check_tap_access", 560_492_391); // '!hog'
            let (capture, state, rx) = start_capture(&hal, fast());
            let error = capture.err().expect("start must fail");
            assert!(
                matches!(&error, AudioError::PermissionDenied(m) if m.contains("System Settings"))
            );
            assert!(!state.is_running());
            assert_eq!(hal.live(), [0, 0, 0, 0]);
            assert_eq!(teardown_calls(&hal.log()), TEARDOWN_ORDER);
            // The sender is gone, so a consumer is not left waiting.
            assert!(rx.recv().is_err());
        }

        #[test]
        fn no_output_device_fails_start() {
            let hal = FakeHal::new();
            hal.state().default_output = None;
            let (capture, state, _rx) = start_capture(&hal, fast());
            assert_eq!(capture.err(), Some(AudioError::NoOutputDevice));
            assert!(!state.is_running());
            assert!(hal.log().is_empty(), "nothing may be created");
        }

        #[test]
        fn a_failed_start_destroys_what_it_created() {
            for failing in ["create_aggregate", "create_io_proc", "start"] {
                let hal = FakeHal::new();
                hal.state().fail.insert(failing, -50);
                let (capture, _state, _rx) = start_capture(&hal, fast());
                assert!(
                    matches!(capture.err(), Some(AudioError::Device(_))),
                    "{failing}"
                );
                assert_eq!(hal.live(), [0, 0, 0, 0], "{failing}");
            }
        }

        #[test]
        fn dropping_the_capture_without_stopping_tears_everything_down() {
            let hal = FakeHal::new();
            let (capture, state, _rx) = start_capture(&hal, fast());
            let capture = capture.unwrap();
            play(&hal, 48_000, 0.1);
            drop(capture);
            assert!(!state.is_running());
            assert_eq!(hal.live(), [0, 0, 0, 0]);
            assert_eq!(teardown_calls(&hal.log()), TEARDOWN_ORDER);
        }

        #[test]
        fn a_consumer_that_goes_away_ends_capture_and_tears_down() {
            let hal = FakeHal::new();
            let (capture, state, rx) = start_capture(&hal, fast());
            let _capture = capture.unwrap();
            drop(rx);
            play(&hal, 48_000, 0.1);
            wait_until("the capture thread to end", || !state.is_running());
            assert_eq!(hal.live(), [0, 0, 0, 0]);
            assert_eq!(state.take_errors(), [AudioError::SinkClosed]);
        }

        /// The output device disappears, rebuilding fails for a while, then
        /// a device is back. Every failed attempt cleans up after itself.
        #[test]
        fn capture_survives_failed_rebuilds_without_leaking() {
            let hal = FakeHal::new();
            let (capture, state, rx) = start_capture(&hal, fast());
            let mut capture = capture.unwrap();
            play(&hal, 48_000, 0.1);

            hal.state().default_output = None;
            wait_until("the old tap to go", || hal.live() == [0, 0, 0, 0]);
            thread::sleep(Duration::from_millis(30));
            assert!(state.is_running());
            assert_eq!(state.take_errors(), [AudioError::NoOutputDevice]);

            // A device is back but the aggregate device cannot be created:
            // each attempt creates a tap and must destroy it again.
            hal.state().fail.insert("create_aggregate", -50);
            hal.set_output("usb-dac", 48_000.0);
            wait_until("a few failed attempts", || {
                hal.log()
                    .iter()
                    .filter(|c| c.starts_with("create_aggregate"))
                    .count()
                    >= 3
            });
            assert_eq!(hal.live()[0], 0, "a failed attempt left its tap behind");
            let errors = state.take_errors();
            assert_eq!(
                errors.len(),
                1,
                "reported once, not per attempt: {errors:?}"
            );

            hal.state().fail.clear();
            wait_until("capture to resume", || hal.live() == [1, 1, 1, 1]);
            play(&hal, 48_000, 0.1);
            capture.stop();

            assert_eq!(hal.live(), [0, 0, 0, 0]);
            assert_eq!(total_samples(&rx.iter().collect::<Vec<_>>()), 3_200);
            // Every tap that was created was destroyed.
            let log = hal.log();
            let count = |name: &str| log.iter().filter(|c| c.starts_with(name)).count();
            assert_eq!(count("create_tap"), count("destroy_tap"));
            // Two aggregate devices came to exist: the first and the last.
            assert_eq!(count("destroy_aggregate"), 2);
        }

        /// A stream that goes quiet is torn down and rebuilt, as the forum
        /// reports of stalled taps require: both objects, not just the proc.
        #[test]
        fn a_stalled_tap_is_rebuilt_from_scratch() {
            let hal = FakeHal::new();
            let timing = Timing {
                stall: Duration::from_millis(40),
                ..fast()
            };
            let (capture, state, _rx) = start_capture(&hal, timing);
            let mut capture = capture.unwrap();
            wait_until("two rebuilds", || {
                hal.log()
                    .iter()
                    .filter(|c| c.starts_with("create_tap"))
                    .count()
                    >= 3
            });
            capture.stop();
            assert_eq!(hal.live(), [0, 0, 0, 0]);
            let log = hal.log();
            let count = |name: &str| log.iter().filter(|c| c.starts_with(name)).count();
            assert_eq!(count("create_tap"), count("destroy_tap"));
            assert_eq!(count("create_aggregate"), count("destroy_aggregate"));
            assert_eq!(count("create_io_proc"), count("destroy_io_proc"));
            assert_eq!(count("start"), count("stop"));
            let errors = state.take_errors();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(&errors[0], AudioError::Stream(m) if m.contains("no audio")));
        }
    }
}
