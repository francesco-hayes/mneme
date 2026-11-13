//! One open tap and everything created around it, with guaranteed teardown.
//!
//! Capturing through a tap takes four system resources, created in this
//! order:
//!
//! 1. the **tap** (`AudioHardwareCreateProcessTap`),
//! 2. a private **aggregate device** that contains the tap
//!    (`AudioHardwareCreateAggregateDevice`),
//! 3. an **IO proc** registered on the aggregate device
//!    (`AudioDeviceCreateIOProcID`),
//! 4. **running IO** (`AudioDeviceStart`).
//!
//! [`TapSession`] owns all four and releases them in reverse order when it
//! is dropped, however far opening got and for whatever reason it is
//! dropped: a normal stop, a re-open after a device change, an error half way
//! through opening, or a panic unwinding the capture thread. A step that
//! fails during teardown does not stop the remaining steps.
//!
//! The Core Audio calls sit behind the [`Hal`] trait so that this ordering is
//! proven by tests with a recording fake, without creating a real tap.
//!
//! What this cannot cover is a process that dies without unwinding (a crash,
//! `kill -9`, `panic = "abort"`). The tap and the aggregate device are both
//! created private to the process, and Core Audio removes a process's private
//! objects when the process goes away.

use std::fmt;
use std::ptr::NonNull;

use objc2_core_audio_types::AudioStreamBasicDescription;

use super::format::{TapFormat, four_char_code, sample_rate_hz};
use super::ioproc::IoContext;
use crate::AudioError;
use crate::capture::StreamEvents;

/// A Core Audio object ID.
pub(crate) type ObjectId = u32;

/// A failed Core Audio call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HalError {
    /// What was being done, as a phrase: "creating the process tap".
    pub call: &'static str,
    /// The `OSStatus`.
    pub status: i32,
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed with status {}",
            self.call,
            four_char_code(self.status as u32)
        )
    }
}

impl From<HalError> for AudioError {
    fn from(error: HalError) -> Self {
        AudioError::Device(error.to_string())
    }
}

/// The system default output device, as far as capture cares.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputDevice {
    pub uid: String,
    /// Nominal sample rate in Hz; `0.0` if it could not be read.
    pub sample_rate: f64,
}

impl OutputDevice {
    /// What the capture thread compares to notice that audio now goes
    /// somewhere else or at another rate. Either means the tap has to be
    /// rebuilt.
    pub(crate) fn identity(&self) -> String {
        format!("{}@{:.0}", self.uid, self.sample_rate)
    }
}

/// A tap that was just created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedTap {
    pub id: ObjectId,
    /// The UID the aggregate device refers to the tap by.
    pub uid: String,
}

/// The Core Audio calls a tap session makes. Implemented by
/// [`CoreAudioHal`](super::hal::CoreAudioHal) and by a fake in tests.
pub(crate) trait Hal: Clone {
    /// Handle of a registered IO proc.
    type IoProc;

    /// The default output device, or `None` if there is none.
    fn default_output(&self) -> Option<OutputDevice>;

    /// The audio process object of a running process, or `None` if the
    /// process is not a Core Audio client.
    fn process_object(&self, pid: i32) -> Option<ObjectId>;

    /// Creates a private, unmuted, stereo tap of every process except
    /// `excluded`.
    fn create_tap(&self, excluded: &[ObjectId]) -> Result<CreatedTap, HalError>;

    /// The format a tap delivers.
    fn tap_format(&self, tap: ObjectId) -> Result<AudioStreamBasicDescription, HalError>;

    /// Creates a private aggregate device whose only input is the tap.
    fn create_aggregate(&self, tap_uid: &str) -> Result<ObjectId, HalError>;

    /// A device's nominal sample rate.
    fn device_sample_rate(&self, device: ObjectId) -> Result<f64, HalError>;

    fn set_device_sample_rate(&self, device: ObjectId, rate: f64) -> Result<(), HalError>;

    /// Registers the IO proc on a device, with `context` as its client data.
    fn create_io_proc(
        &self,
        device: ObjectId,
        context: NonNull<IoContext>,
    ) -> Result<Self::IoProc, HalError>;

    fn start(&self, device: ObjectId, io_proc: &Self::IoProc) -> Result<(), HalError>;

    /// Fails if the process may not read the tap (see
    /// [`TapConfig::check_access`]).
    fn check_tap_access(&self, tap: ObjectId) -> Result<(), HalError>;

    fn stop(&self, device: ObjectId, io_proc: &Self::IoProc) -> Result<(), HalError>;

    fn destroy_io_proc(&self, device: ObjectId, io_proc: Self::IoProc) -> Result<(), HalError>;

    fn destroy_aggregate(&self, device: ObjectId) -> Result<(), HalError>;

    fn destroy_tap(&self, tap: ObjectId) -> Result<(), HalError>;
}

/// How to set a tap up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TapConfig {
    /// Processes whose audio is left out of the tap.
    pub excluded_pids: Vec<i32>,
    /// After starting IO, check that the tap accepts a description update.
    ///
    /// Without the System Audio Recording permission every call succeeds and
    /// the tap delivers silence; the one call that is refused is updating
    /// the tap's description. Chromium uses this to detect a missing
    /// permission, and so does this. With the check off, a missing
    /// permission goes unnoticed and the recording is silent.
    pub check_access: bool,
}

/// What [`TapSession::teardown`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TeardownReport {
    /// Every teardown call that failed, in the order they were made.
    pub errors: Vec<HalError>,
    /// The IO proc's context was kept alive on purpose (see
    /// [`TapSession::teardown`]).
    pub context_leaked: bool,
}

/// A tap that is capturing. Dropping it stops IO and destroys the IO proc,
/// the aggregate device and the tap, in that order.
pub(crate) struct TapSession<H: Hal> {
    hal: H,
    /// For reporting teardown failures.
    events: StreamEvents,
    tap: Option<ObjectId>,
    aggregate: Option<ObjectId>,
    io_proc: Option<H::IoProc>,
    started: bool,
    /// Client data of the IO proc: a leaked `Box`, reclaimed in teardown.
    context: Option<NonNull<IoContext>>,
    sample_rate: u32,
}

impl<H: Hal> TapSession<H> {
    /// Creates the tap, wraps it in an aggregate device and starts reading
    /// it. Buffers go to `events`.
    ///
    /// On any failure everything created so far is destroyed before this
    /// returns.
    pub(crate) fn open(
        hal: H,
        config: &TapConfig,
        events: StreamEvents,
    ) -> Result<Self, AudioError> {
        // From here on `session` owns whatever gets created: an early return
        // drops it, and the drop tears down exactly what exists.
        let mut session = Self {
            hal,
            events: events.clone(),
            tap: None,
            aggregate: None,
            io_proc: None,
            started: false,
            context: None,
            sample_rate: 0,
        };
        let hal = session.hal.clone();

        let excluded: Vec<ObjectId> = config
            .excluded_pids
            .iter()
            .filter_map(|pid| hal.process_object(*pid))
            .collect();
        let tap = hal.create_tap(&excluded)?;
        session.tap = Some(tap.id);

        let format = TapFormat::from_description(&hal.tap_format(tap.id)?)?;

        let aggregate = hal.create_aggregate(&tap.uid)?;
        session.aggregate = Some(aggregate);
        session.sample_rate = settle_sample_rate(&hal, aggregate, format.sample_rate);

        let context = Box::new(IoContext::new(events, format.kind, session.sample_rate));
        let context = NonNull::from(Box::leak(context));
        session.context = Some(context);
        session.io_proc = Some(hal.create_io_proc(aggregate, context)?);

        if let Some(io_proc) = &session.io_proc {
            hal.start(aggregate, io_proc)?;
            session.started = true;
        }

        if config.check_access {
            hal.check_tap_access(tap.id).map_err(|error| {
                AudioError::PermissionDenied(format!(
                    "macOS does not let this app record system audio ({error}); allow it under \
                     System Settings > Privacy & Security > Screen & System Audio Recording > \
                     System Audio Recording Only"
                ))
            })?;
        }
        Ok(session)
    }

    /// Rate of the audio the IO proc delivers, in Hz.
    pub(crate) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[cfg(test)]
    pub(crate) fn tap_id(&self) -> Option<ObjectId> {
        self.tap
    }

    #[cfg(test)]
    pub(crate) fn aggregate_id(&self) -> Option<ObjectId> {
        self.aggregate
    }

    /// Releases everything the session holds, newest first: stop IO, destroy
    /// the IO proc, destroy the aggregate device, destroy the tap. Every step
    /// runs even if an earlier one failed. Calling it again does nothing.
    ///
    /// The IO proc's context is freed last, and only if Core Audio confirmed
    /// that the proc is stopped and gone. If stopping or destroying the proc
    /// failed, a callback could still arrive, so the context is leaked
    /// instead (a few dozen bytes) rather than freed under it. Chromium does
    /// the same.
    pub(crate) fn teardown(&mut self) -> TeardownReport {
        let mut report = TeardownReport::default();
        let mut proc_may_still_run = false;

        if let Some(context) = self.context {
            // SAFETY: the context is alive until it is freed below.
            unsafe { context.as_ref() }.deactivate();
        }
        if let Some(io_proc) = self.io_proc.take() {
            // An IO proc only ever exists on the aggregate device.
            if let Some(device) = self.aggregate {
                if std::mem::take(&mut self.started)
                    && let Err(error) = self.hal.stop(device, &io_proc)
                {
                    report.errors.push(error);
                    proc_may_still_run = true;
                }
                if let Err(error) = self.hal.destroy_io_proc(device, io_proc) {
                    report.errors.push(error);
                    proc_may_still_run = true;
                }
            }
        }
        if let Some(device) = self.aggregate.take()
            && let Err(error) = self.hal.destroy_aggregate(device)
        {
            report.errors.push(error);
        }
        if let Some(tap) = self.tap.take()
            && let Err(error) = self.hal.destroy_tap(tap)
        {
            report.errors.push(error);
        }
        if let Some(context) = self.context.take() {
            if proc_may_still_run {
                report.context_leaked = true;
            } else {
                // SAFETY: created by `Box::leak` in `open`, taken out of
                // `self.context` so it is freed once, and no IO proc that
                // could use it exists any more.
                drop(unsafe { Box::from_raw(context.as_ptr()) });
            }
        }
        report
    }
}

impl<H: Hal> Drop for TapSession<H> {
    fn drop(&mut self) {
        // The capture thread reads these after it has dropped the stream, and
        // queues them for the source's owner.
        for error in self.teardown().errors {
            self.events.error(AudioError::Device(format!(
                "system audio capture was not shut down cleanly: {error}"
            )));
        }
    }
}

/// Decides the rate of the audio the IO proc will deliver.
///
/// IO runs at the aggregate device's nominal rate. It should equal the tap's;
/// if it does not, the aggregate is asked to run at the tap's rate (macOS 14
/// does not convert between the two, as Chromium found), and whatever the
/// aggregate then reports is used. If the aggregate reports nothing usable,
/// the tap's rate is.
fn settle_sample_rate<H: Hal>(hal: &H, aggregate: ObjectId, tap_rate: u32) -> u32 {
    let read = |hal: &H| {
        hal.device_sample_rate(aggregate)
            .ok()
            .and_then(|rate| sample_rate_hz(rate).ok())
    };
    match read(hal) {
        Some(rate) if rate == tap_rate => rate,
        Some(_) => {
            let _ = hal.set_device_sample_rate(aggregate, f64::from(tap_rate));
            read(hal).unwrap_or(tap_rate)
        }
        None => tap_rate,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex, MutexGuard};

    use objc2_core_audio_types::kAudioFormatFlagsNativeFloatPacked;

    use super::*;
    use crate::capture::tests::logging_events;
    use crate::system::format::tests::description;

    /// Everything the fake HAL knows and has been asked to do.
    pub(crate) struct FakeState {
        /// Every call that creates, starts, stops or destroys something, in
        /// order: `"create_tap 10"`, `"destroy_tap 10"`.
        pub log: Vec<String>,
        /// Calls that fail, by name, with the status they return. A call
        /// stays failing until removed.
        pub fail: BTreeMap<&'static str, i32>,
        pub default_output: Option<OutputDevice>,
        pub process_objects: BTreeMap<i32, ObjectId>,
        pub tap_format: AudioStreamBasicDescription,
        /// What the aggregate device reports as its rate; `None` follows the
        /// tap format.
        pub aggregate_rate: Option<f64>,
        /// Whether setting the aggregate's rate takes effect.
        pub aggregate_accepts_rate: bool,
        /// Process objects excluded from the most recent tap.
        pub excluded: Vec<ObjectId>,
        pub taps: BTreeSet<ObjectId>,
        pub aggregates: BTreeSet<ObjectId>,
        pub io_procs: BTreeSet<u32>,
        pub running: BTreeSet<u32>,
        /// Client data of the registered IO proc, as an address.
        context: Option<usize>,
        next_id: u32,
    }

    /// A [`Hal`] that creates nothing and records everything.
    #[derive(Clone)]
    pub(crate) struct FakeHal(Arc<Mutex<FakeState>>);

    impl FakeHal {
        /// A machine with one 48 kHz output device and a tap that delivers
        /// interleaved stereo float.
        pub(crate) fn new() -> Self {
            Self(Arc::new(Mutex::new(FakeState {
                log: Vec::new(),
                fail: BTreeMap::new(),
                default_output: Some(OutputDevice {
                    uid: "BuiltInSpeakerDevice".to_owned(),
                    sample_rate: 48_000.0,
                }),
                process_objects: BTreeMap::new(),
                tap_format: description(48_000.0, 2, kAudioFormatFlagsNativeFloatPacked, 32, true),
                aggregate_rate: None,
                aggregate_accepts_rate: true,
                excluded: Vec::new(),
                taps: BTreeSet::new(),
                aggregates: BTreeSet::new(),
                io_procs: BTreeSet::new(),
                running: BTreeSet::new(),
                context: None,
                next_id: 10,
            })))
        }

        pub(crate) fn state(&self) -> MutexGuard<'_, FakeState> {
            self.0.lock().unwrap()
        }

        pub(crate) fn log(&self) -> Vec<String> {
            self.state().log.clone()
        }

        /// Number of taps, aggregate devices, IO procs and running IO procs
        /// that exist right now.
        pub(crate) fn live(&self) -> [usize; 4] {
            let state = self.state();
            [
                state.taps.len(),
                state.aggregates.len(),
                state.io_procs.len(),
                state.running.len(),
            ]
        }

        /// The default output device changes to one with this UID and rate;
        /// the tap follows its rate.
        pub(crate) fn set_output(&self, uid: &str, rate: f64) {
            let mut state = self.state();
            state.default_output = Some(OutputDevice {
                uid: uid.to_owned(),
                sample_rate: rate,
            });
            state.tap_format.mSampleRate = rate;
        }

        /// Runs `call` with the client data of the running IO proc, the way
        /// Core Audio's IO thread would call the proc. Returns `false`
        /// without calling if no IO proc is running.
        ///
        /// The fake's lock is held meanwhile, so stopping the proc waits for
        /// the call to return, as `AudioDeviceStop` does.
        pub(crate) fn with_running_io_proc(&self, call: impl FnOnce(*const IoContext)) -> bool {
            let state = self.state();
            match state.context {
                Some(address) if !state.running.is_empty() => {
                    call(address as *const IoContext);
                    true
                }
                _ => false,
            }
        }

        /// The fake checks that teardown calls arrive in a workable order.
        /// Once a test has scripted a call to fail, state no longer follows
        /// the calls, and the test compares the log instead.
        fn check_order(state: &FakeState, holds: bool, rule: &str) {
            assert!(holds || !state.fail.is_empty(), "{rule}");
        }

        fn call(
            state: &mut FakeState,
            name: &'static str,
            id: impl fmt::Display,
        ) -> Result<(), HalError> {
            state.log.push(format!("{name} {id}"));
            match state.fail.get(name) {
                Some(status) => Err(HalError {
                    call: name,
                    status: *status,
                }),
                None => Ok(()),
            }
        }
    }

    impl Hal for FakeHal {
        type IoProc = u32;

        fn default_output(&self) -> Option<OutputDevice> {
            self.state().default_output.clone()
        }

        fn process_object(&self, pid: i32) -> Option<ObjectId> {
            self.state().process_objects.get(&pid).copied()
        }

        fn create_tap(&self, excluded: &[ObjectId]) -> Result<CreatedTap, HalError> {
            let mut state = self.state();
            let id = state.next_id;
            Self::call(&mut state, "create_tap", id)?;
            state.next_id += 1;
            state.excluded = excluded.to_vec();
            state.taps.insert(id);
            Ok(CreatedTap {
                id,
                uid: format!("tap-uid-{id}"),
            })
        }

        fn tap_format(&self, tap: ObjectId) -> Result<AudioStreamBasicDescription, HalError> {
            let mut state = self.state();
            assert!(state.taps.contains(&tap), "format of a tap that is gone");
            Self::call(&mut state, "tap_format", tap)?;
            Ok(state.tap_format)
        }

        fn create_aggregate(&self, tap_uid: &str) -> Result<ObjectId, HalError> {
            let mut state = self.state();
            let id = state.next_id;
            Self::call(&mut state, "create_aggregate", format!("{id} of {tap_uid}"))?;
            state.next_id += 1;
            state.aggregates.insert(id);
            Ok(id)
        }

        fn device_sample_rate(&self, device: ObjectId) -> Result<f64, HalError> {
            let state = self.state();
            assert!(state.aggregates.contains(&device));
            if let Some(status) = state.fail.get("device_sample_rate") {
                return Err(HalError {
                    call: "device_sample_rate",
                    status: *status,
                });
            }
            Ok(state.aggregate_rate.unwrap_or(state.tap_format.mSampleRate))
        }

        fn set_device_sample_rate(&self, device: ObjectId, rate: f64) -> Result<(), HalError> {
            let mut state = self.state();
            Self::call(
                &mut state,
                "set_device_sample_rate",
                format!("{device} {rate}"),
            )?;
            if state.aggregate_accepts_rate {
                state.aggregate_rate = Some(rate);
            }
            Ok(())
        }

        fn create_io_proc(
            &self,
            device: ObjectId,
            context: NonNull<IoContext>,
        ) -> Result<u32, HalError> {
            let mut state = self.state();
            assert!(state.aggregates.contains(&device));
            let id = state.next_id;
            Self::call(&mut state, "create_io_proc", format!("{id} on {device}"))?;
            state.next_id += 1;
            state.io_procs.insert(id);
            state.context = Some(context.as_ptr() as usize);
            Ok(id)
        }

        fn start(&self, device: ObjectId, io_proc: &u32) -> Result<(), HalError> {
            let mut state = self.state();
            assert!(state.aggregates.contains(&device) && state.io_procs.contains(io_proc));
            Self::call(&mut state, "start", io_proc)?;
            state.running.insert(*io_proc);
            Ok(())
        }

        fn check_tap_access(&self, tap: ObjectId) -> Result<(), HalError> {
            let mut state = self.state();
            assert!(state.taps.contains(&tap));
            Self::call(&mut state, "check_tap_access", tap)
        }

        fn stop(&self, device: ObjectId, io_proc: &u32) -> Result<(), HalError> {
            let mut state = self.state();
            Self::check_order(
                &state,
                state.aggregates.contains(&device) && state.io_procs.contains(io_proc),
                "IO stopped after its proc or device was destroyed",
            );
            Self::call(&mut state, "stop", io_proc)?;
            state.running.remove(io_proc);
            Ok(())
        }

        fn destroy_io_proc(&self, device: ObjectId, io_proc: u32) -> Result<(), HalError> {
            let mut state = self.state();
            Self::check_order(
                &state,
                state.aggregates.contains(&device) && !state.running.contains(&io_proc),
                "IO proc destroyed while running or after its device",
            );
            Self::call(&mut state, "destroy_io_proc", io_proc)?;
            state.io_procs.remove(&io_proc);
            state.context = None;
            Ok(())
        }

        fn destroy_aggregate(&self, device: ObjectId) -> Result<(), HalError> {
            let mut state = self.state();
            Self::check_order(
                &state,
                state.io_procs.is_empty(),
                "aggregate device destroyed before its IO proc",
            );
            Self::call(&mut state, "destroy_aggregate", device)?;
            state.aggregates.remove(&device);
            Ok(())
        }

        fn destroy_tap(&self, tap: ObjectId) -> Result<(), HalError> {
            let mut state = self.state();
            Self::check_order(
                &state,
                state.aggregates.is_empty(),
                "tap destroyed before the aggregate device that contains it",
            );
            Self::call(&mut state, "destroy_tap", tap)?;
            state.taps.remove(&tap);
            Ok(())
        }
    }

    pub(crate) fn checked() -> TapConfig {
        TapConfig {
            excluded_pids: Vec::new(),
            check_access: true,
        }
    }

    fn open(hal: &FakeHal) -> Result<TapSession<FakeHal>, AudioError> {
        TapSession::open(hal.clone(), &checked(), StreamEvents::detached())
    }

    const OPENING: [&str; 5] = [
        "create_tap 10",
        "tap_format 10",
        "create_aggregate 11 of tap-uid-10",
        "create_io_proc 12 on 11",
        "start 12",
    ];
    const TEARDOWN: [&str; 4] = [
        "stop 12",
        "destroy_io_proc 12",
        "destroy_aggregate 11",
        "destroy_tap 10",
    ];

    #[test]
    fn opening_creates_everything_in_order() {
        let hal = FakeHal::new();
        let session = open(&hal).unwrap();
        let mut expected = OPENING.to_vec();
        expected.push("check_tap_access 10");
        assert_eq!(hal.log(), expected);
        assert_eq!(hal.live(), [1, 1, 1, 1]);
        assert_eq!(session.sample_rate(), 48_000);
        assert_eq!(session.tap_id(), Some(10));
        assert_eq!(session.aggregate_id(), Some(11));
    }

    #[test]
    fn dropping_a_session_tears_down_in_reverse_order() {
        let hal = FakeHal::new();
        let (events, log) = logging_events();
        let session = TapSession::open(hal.clone(), &checked(), events).unwrap();
        hal.state().log.clear();
        assert!(!log.senders_gone());
        drop(session);
        assert_eq!(hal.log(), TEARDOWN);
        assert_eq!(hal.live(), [0, 0, 0, 0]);
        // The IO proc's context, which held the last handle, was freed.
        assert!(log.senders_gone());
        assert!(log.errors().is_empty());
    }

    #[test]
    fn teardown_happens_once() {
        let hal = FakeHal::new();
        let mut session = open(&hal).unwrap();
        hal.state().log.clear();
        assert_eq!(session.teardown(), TeardownReport::default());
        assert_eq!(session.teardown(), TeardownReport::default());
        drop(session);
        assert_eq!(hal.log(), TEARDOWN);
    }

    /// Opening can fail at every step. Whatever exists by then is destroyed,
    /// newest first, and nothing else is touched.
    #[test]
    fn a_failure_at_any_step_releases_exactly_what_was_created() {
        // (failing call, calls made before it fails, teardown expected)
        let cases: [(&str, usize, &[&str]); 6] = [
            ("create_tap", 0, &[]),
            ("tap_format", 1, &["destroy_tap 10"]),
            ("create_aggregate", 2, &["destroy_tap 10"]),
            (
                "create_io_proc",
                3,
                &["destroy_aggregate 11", "destroy_tap 10"],
            ),
            (
                "start",
                4,
                // Never started, so not stopped.
                &[
                    "destroy_io_proc 12",
                    "destroy_aggregate 11",
                    "destroy_tap 10",
                ],
            ),
            ("check_tap_access", 5, &TEARDOWN),
        ];
        for (failing, made, teardown) in cases {
            let hal = FakeHal::new();
            hal.state().fail.insert(failing, -50);
            let error = open(&hal)
                .err()
                .unwrap_or_else(|| panic!("{failing} did not fail"));

            let log = hal.log();
            assert_eq!(&log[..made], &OPENING[..made], "{failing}");
            assert!(log[made].starts_with(failing), "{failing}: {log:?}");
            assert_eq!(&log[made + 1..], teardown, "{failing}");
            assert_eq!(hal.live(), [0, 0, 0, 0], "{failing}");

            if failing == "check_tap_access" {
                assert!(matches!(error, AudioError::PermissionDenied(_)), "{error}");
            } else {
                assert!(
                    matches!(&error, AudioError::Device(m) if m.contains(failing) && m.contains("-50")),
                    "{failing}: {error}"
                );
            }
        }
    }

    #[test]
    fn a_format_that_cannot_be_converted_destroys_the_tap() {
        let hal = FakeHal::new();
        hal.state().tap_format.mFormatID = 0x6161_6320; // 'aac '
        assert!(matches!(open(&hal), Err(AudioError::UnsupportedFormat(_))));
        assert_eq!(
            hal.log(),
            ["create_tap 10", "tap_format 10", "destroy_tap 10"]
        );
        assert_eq!(hal.live(), [0, 0, 0, 0]);
    }

    /// One teardown call failing must not leave the rest undone.
    #[test]
    fn teardown_carries_on_past_failures_and_reports_them() {
        for failing in [
            "stop",
            "destroy_io_proc",
            "destroy_aggregate",
            "destroy_tap",
        ] {
            let hal = FakeHal::new();
            let (events, log) = logging_events();
            let session = TapSession::open(hal.clone(), &checked(), events).unwrap();
            {
                let mut state = hal.state();
                state.log.clear();
                state.fail.insert(failing, 560_947_818); // '!obj'
            }
            drop(session);

            assert_eq!(hal.log(), TEARDOWN, "{failing}");
            let errors = log.errors();
            assert_eq!(errors.len(), 1, "{failing}: {errors:?}");
            assert!(
                matches!(&errors[0], AudioError::Device(m)
                    if m.contains("not shut down cleanly") && m.contains(failing) && m.contains("'!obj'")),
                "{failing}: {:?}",
                errors[0]
            );
        }
    }

    /// If Core Audio will not confirm that the IO proc is gone, a callback
    /// may still come: its context must stay valid.
    #[test]
    fn the_io_context_outlives_a_proc_that_could_not_be_stopped() {
        for (failing, leaked) in [
            ("stop", true),
            ("destroy_io_proc", true),
            ("destroy_aggregate", false),
            ("destroy_tap", false),
        ] {
            let hal = FakeHal::new();
            let (events, log) = logging_events();
            let mut session = TapSession::open(hal.clone(), &checked(), events).unwrap();
            hal.state().fail.insert(failing, -1);
            let report = session.teardown();
            assert_eq!(report.context_leaked, leaked, "{failing}");
            assert_eq!(
                report.errors,
                [HalError {
                    call: failing,
                    status: -1
                }]
            );
            drop(session);
            // A leaked context still holds its handle; a freed one does not.
            assert_eq!(log.senders_gone(), !leaked, "{failing}");
        }
    }

    /// A late callback into a context kept alive this way does nothing.
    #[test]
    fn a_callback_after_a_failed_stop_is_ignored() {
        use crate::system::ioproc::tests::{BufferList, call_io_proc, timestamp};

        let hal = FakeHal::new();
        let (events, log) = logging_events();
        let session = TapSession::open(hal.clone(), &checked(), events).unwrap();
        let mut context = std::ptr::null();
        assert!(hal.with_running_io_proc(|running| context = running));
        hal.state().fail.insert("stop", -1);
        drop(session);

        let samples = [0u8; 64];
        let list = BufferList::over([&samples[..]], 2);
        call_io_proc(
            context,
            list.as_ptr(),
            &timestamp(std::time::Duration::ZERO),
        );
        assert!(log.data().is_empty());
    }

    #[test]
    fn a_session_that_never_registered_a_proc_frees_its_context() {
        let hal = FakeHal::new();
        hal.state().fail.insert("create_io_proc", -1);
        let (events, log) = logging_events();
        // The context exists by the time the proc fails to register.
        assert!(TapSession::open(hal.clone(), &checked(), events).is_err());
        assert_eq!(hal.live(), [0, 0, 0, 0]);
        assert!(log.senders_gone());
    }

    #[test]
    fn excluded_processes_are_translated_to_audio_objects() {
        let hal = FakeHal::new();
        hal.state()
            .process_objects
            .extend([(4_242, 77), (4_243, 78)]);
        let config = TapConfig {
            // 999 is not a Core Audio client: there is nothing to exclude.
            excluded_pids: vec![4_242, 999, 4_243],
            check_access: true,
        };
        let _session = TapSession::open(hal.clone(), &config, StreamEvents::detached()).unwrap();
        assert_eq!(hal.state().excluded, [77, 78]);
    }

    #[test]
    fn the_access_check_can_be_left_out() {
        let hal = FakeHal::new();
        hal.state().fail.insert("check_tap_access", -1);
        let config = TapConfig {
            excluded_pids: Vec::new(),
            check_access: false,
        };
        let _session = TapSession::open(hal.clone(), &config, StreamEvents::detached()).unwrap();
        assert_eq!(hal.log(), OPENING);
    }

    #[test]
    fn io_runs_at_the_aggregate_devices_rate() {
        // The usual case: both agree, nothing is set.
        let hal = FakeHal::new();
        hal.set_output("usb-dac", 44_100.0);
        let session = open(&hal).unwrap();
        assert_eq!(session.sample_rate(), 44_100);
        assert!(!hal.log().iter().any(|call| call.starts_with("set_device")));

        // The aggregate came up at another rate: it is moved to the tap's.
        let hal = FakeHal::new();
        hal.state().aggregate_rate = Some(44_100.0);
        let session = open(&hal).unwrap();
        assert_eq!(session.sample_rate(), 48_000);
        assert!(
            hal.log()
                .contains(&"set_device_sample_rate 11 48000".to_owned())
        );

        // It refuses to move: its own rate is what the IO proc will deliver.
        let hal = FakeHal::new();
        {
            let mut state = hal.state();
            state.aggregate_rate = Some(96_000.0);
            state.aggregate_accepts_rate = false;
        }
        assert_eq!(open(&hal).unwrap().sample_rate(), 96_000);

        // It reports nothing usable: the tap's rate.
        let hal = FakeHal::new();
        hal.state().aggregate_rate = Some(0.0);
        hal.state().aggregate_accepts_rate = false;
        assert_eq!(open(&hal).unwrap().sample_rate(), 48_000);
        let hal = FakeHal::new();
        hal.state().fail.insert("device_sample_rate", -1);
        assert_eq!(open(&hal).unwrap().sample_rate(), 48_000);
    }

    #[test]
    fn the_device_identity_tells_devices_and_rates_apart() {
        let device = |uid: &str, sample_rate| OutputDevice {
            uid: uid.to_owned(),
            sample_rate,
        };
        let speakers = device("BuiltInSpeakerDevice", 48_000.0);
        assert_eq!(speakers.identity(), "BuiltInSpeakerDevice@48000");
        assert_eq!(speakers.identity(), speakers.clone().identity());
        assert_ne!(speakers.identity(), device("AirPods", 48_000.0).identity());
        // AirPods switching to the hands-free profile.
        assert_ne!(
            device("AirPods", 48_000.0).identity(),
            device("AirPods", 24_000.0).identity()
        );
    }

    #[test]
    fn hal_errors_read_well() {
        let error = HalError {
            call: "creating the process tap",
            status: 0x216f_626a,
        };
        assert_eq!(
            error.to_string(),
            "creating the process tap failed with status '!obj'"
        );
        assert_eq!(
            HalError {
                call: "starting IO",
                status: -50
            }
            .to_string(),
            "starting IO failed with status -50"
        );
        assert_eq!(
            AudioError::from(error),
            AudioError::Device("creating the process tap failed with status '!obj'".to_owned())
        );
    }
}
