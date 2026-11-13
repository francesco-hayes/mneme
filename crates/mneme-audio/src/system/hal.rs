//! The real Core Audio calls behind [`Hal`], through the `objc2-core-audio`
//! bindings.
//!
//! Each function here is one call (or one property read) with its pointer
//! handling, and nothing else: the order in which they are made, and what
//! happens when one fails, is decided in [`session`](super::session) and
//! tested there against a fake.
//!
//! Tested here, against the real system: the property reads that need no
//! permission (default output device, process objects, object lists) and the
//! two descriptions handed to Core Audio (the `CATapDescription` and the
//! aggregate device dictionary), which can be built and inspected without
//! creating anything. **Never run in this crate's test suite:** creating a
//! tap or an aggregate device, registering or starting an IO proc, the
//! access check, and the four teardown calls. Those run only in the ignored
//! live tests.

use std::ffi::{CStr, c_void};
use std::ptr::{self, NonNull};

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProc, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertySelector, AudioObjectSetPropertyData,
    CATapDescription, CATapMuteBehavior, kAudioAggregateDeviceIsPrivateKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioHardwareBadPropertySizeError,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyTranslatePIDToProcessObject,
    kAudioHardwareUnspecifiedError, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyDescription,
    kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_foundation::{NSArray, NSNumber, NSString, NSUUID};

use super::ioproc::{IoContext, RawIoProc, io_proc};
use super::session::{CreatedTap, Hal, HalError, ObjectId, OutputDevice};

/// Name of the tap and of the aggregate device. Both are private to the
/// process, so the name is seen only by this process and in logs.
pub(crate) const OBJECT_NAME: &str = "Mneme system audio";

const SYSTEM_OBJECT: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

/// Core Audio itself.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CoreAudioHal;

fn check(status: i32, call: &'static str) -> Result<(), HalError> {
    if status == 0 {
        Ok(())
    } else {
        Err(HalError { call, status })
    }
}

fn global(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads a property whose value is one fixed-size plain value.
///
/// `T` must be a type every bit pattern is valid for (integers, floats,
/// structs of them, raw pointers): Core Audio fills it in byte by byte.
/// `qualifier` is passed as the property's qualifier data.
fn read_value<T: Copy, Q: Copy>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    qualifier: Option<&Q>,
    mut value: T,
    call: &'static str,
) -> Result<T, HalError> {
    let address = global(selector);
    let mut size = size_of::<T>() as u32;
    let (qualifier_size, qualifier_data) = match qualifier {
        Some(q) => (size_of::<Q>() as u32, ptr::from_ref(q).cast::<c_void>()),
        None => (0, ptr::null()),
    };
    // SAFETY: every pointer refers to a live local of the size stated; Core
    // Audio writes at most `size` bytes into `value`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            qualifier_size,
            qualifier_data,
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    check(status, call)?;
    if size as usize != size_of::<T>() {
        return Err(HalError {
            call,
            status: kAudioHardwareBadPropertySizeError,
        });
    }
    Ok(value)
}

/// Reads a property whose value is a `CFString` the caller must release.
fn read_string(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    call: &'static str,
) -> Result<String, HalError> {
    let string: *const CFString = read_value(object, selector, None::<&()>, ptr::null(), call)?;
    let Some(string) = NonNull::new(string.cast_mut()) else {
        return Err(HalError {
            call,
            status: kAudioHardwareUnspecifiedError,
        });
    };
    // SAFETY: the property hands over one retain count of a CFString ("the
    // caller is responsible for releasing the returned CFObject").
    let string = unsafe { CFRetained::from_raw(string) };
    Ok(string.to_string())
}

/// Reads a property whose value is an array of object IDs.
#[cfg(test)]
fn read_object_list(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
    call: &'static str,
) -> Result<Vec<AudioObjectID>, HalError> {
    let address = global(selector);
    let mut size = 0u32;
    // SAFETY: pointers to live locals.
    let status = unsafe {
        objc2_core_audio::AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
        )
    };
    check(status, call)?;
    let mut ids = vec![kAudioObjectUnknown; size as usize / size_of::<AudioObjectID>()];
    if ids.is_empty() {
        return Ok(ids);
    }
    let mut size = (ids.len() * size_of::<AudioObjectID>()) as u32;
    // SAFETY: `ids` has room for `size` bytes; Core Audio reports how many
    // it wrote back through `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(ids.as_mut_ptr())
                .expect("non-empty Vec")
                .cast(),
        )
    };
    check(status, call)?;
    // The list can shrink between the two calls.
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    Ok(ids)
}

/// Describes the tap: every process's output mixed down to stereo, except
/// the given process objects; private to this process; unmuted, so the user
/// keeps hearing what is captured.
///
/// Building the description creates nothing in the audio system.
fn tap_description(excluded: &[ObjectId]) -> Retained<CATapDescription> {
    let excluded: Vec<Retained<NSNumber>> =
        excluded.iter().map(|id| NSNumber::new_u32(*id)).collect();
    let excluded = NSArray::from_retained_slice(&excluded);
    // SAFETY: a plain initializer of a Foundation-style value object, given
    // the array of NSNumbers it documents.
    let description = unsafe {
        CATapDescription::initStereoGlobalTapButExcludeProcesses(
            CATapDescription::alloc(),
            &excluded,
        )
    };
    // SAFETY: property setters on an object nothing else refers to yet.
    unsafe {
        description.setName(&NSString::from_str(OBJECT_NAME));
        description.setPrivate(true);
        description.setMuteBehavior(CATapMuteBehavior::Unmuted);
    }
    description
}

fn cf_key(key: &CStr) -> CFRetained<CFString> {
    // The keys are ASCII string literals in Apple's headers.
    CFString::from_str(key.to_str().expect("Core Audio dictionary keys are ASCII"))
}

/// Describes the aggregate device: private to this process, no sub-devices,
/// one tap with drift compensation, and IO that starts at once.
///
/// "Tap auto start" is off on purpose. With it on, `AudioDeviceStart` waits
/// until some tapped process plays audio, so a quiet system would deliver no
/// callbacks, and the capture thread would take that for a dead stream. With
/// it off the IO proc runs from the start and silence arrives as silent
/// buffers, which keeps the meeting clock continuous.
///
/// This is the composition Chromium uses. Building the dictionary creates
/// nothing in the audio system.
fn aggregate_description(uid: &str, tap_uid: &str) -> CFRetained<CFDictionary<CFString, CFType>> {
    let yes: &CFType = CFBoolean::new(true).as_ref();
    let no: &CFType = CFBoolean::new(false).as_ref();

    let tap_uid = CFString::from_str(tap_uid);
    let tap = CFDictionary::<CFString, CFType>::from_slices(
        &[
            &*cf_key(kAudioSubTapUIDKey),
            &*cf_key(kAudioSubTapDriftCompensationKey),
        ],
        &[tap_uid.as_ref(), yes],
    );
    let taps = CFArray::from_objects(&[&*tap]);

    let name = CFString::from_str(OBJECT_NAME);
    let uid = CFString::from_str(uid);
    CFDictionary::<CFString, CFType>::from_slices(
        &[
            &*cf_key(kAudioAggregateDeviceNameKey),
            &*cf_key(kAudioAggregateDeviceUIDKey),
            &*cf_key(kAudioAggregateDeviceTapListKey),
            &*cf_key(kAudioAggregateDeviceTapAutoStartKey),
            &*cf_key(kAudioAggregateDeviceIsPrivateKey),
        ],
        &[name.as_ref(), uid.as_ref(), taps.as_ref(), no, yes],
    )
}

impl Hal for CoreAudioHal {
    type IoProc = AudioDeviceIOProcID;

    fn default_output(&self) -> Option<OutputDevice> {
        let device: AudioObjectID = read_value(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyDefaultOutputDevice,
            None::<&()>,
            kAudioObjectUnknown,
            "reading the default output device",
        )
        .ok()?;
        if device == kAudioObjectUnknown {
            return None;
        }
        let uid = read_string(
            device,
            kAudioDevicePropertyDeviceUID,
            "reading the output device's UID",
        )
        .ok()?;
        let sample_rate = self.device_sample_rate(device).unwrap_or(0.0);
        Some(OutputDevice { uid, sample_rate })
    }

    fn process_object(&self, pid: i32) -> Option<ObjectId> {
        // "An error is not returned if the PID doesn't refer to any Process.
        // Rather, this property will return kAudioObjectUnknown."
        read_value(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyTranslatePIDToProcessObject,
            Some(&pid),
            kAudioObjectUnknown,
            "translating a PID to a process object",
        )
        .ok()
        .filter(|object| *object != kAudioObjectUnknown)
    }

    fn create_tap(&self, excluded: &[ObjectId]) -> Result<CreatedTap, HalError> {
        const CALL: &str = "creating the process tap";
        let description = tap_description(excluded);
        // SAFETY: reading a property of a live object.
        let uid = unsafe { description.UUID() }.UUIDString().to_string();
        let mut id = kAudioObjectUnknown;
        // SAFETY: a valid description and a pointer to a live local.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut id) };
        check(status, CALL)?;
        if id == kAudioObjectUnknown {
            return Err(HalError {
                call: CALL,
                status: kAudioHardwareUnspecifiedError,
            });
        }
        Ok(CreatedTap { id, uid })
    }

    fn tap_format(&self, tap: ObjectId) -> Result<AudioStreamBasicDescription, HalError> {
        let empty = AudioStreamBasicDescription {
            mSampleRate: 0.0,
            mFormatID: 0,
            mFormatFlags: 0,
            mBytesPerPacket: 0,
            mFramesPerPacket: 0,
            mBytesPerFrame: 0,
            mChannelsPerFrame: 0,
            mBitsPerChannel: 0,
            mReserved: 0,
        };
        read_value(
            tap,
            kAudioTapPropertyFormat,
            None::<&()>,
            empty,
            "reading the tap's format",
        )
    }

    fn create_aggregate(&self, tap_uid: &str) -> Result<ObjectId, HalError> {
        const CALL: &str = "creating the aggregate device";
        // The UID must be unique among devices; a fresh UUID is.
        let uid = NSUUID::UUID().UUIDString().to_string();
        let description = aggregate_description(&uid, tap_uid);
        let mut id = kAudioObjectUnknown;
        // SAFETY: a valid dictionary and a pointer to a live local.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(description.as_ref(), NonNull::from(&mut id))
        };
        check(status, CALL)?;
        if id == kAudioObjectUnknown {
            return Err(HalError {
                call: CALL,
                status: kAudioHardwareUnspecifiedError,
            });
        }
        Ok(id)
    }

    fn device_sample_rate(&self, device: ObjectId) -> Result<f64, HalError> {
        read_value(
            device,
            kAudioDevicePropertyNominalSampleRate,
            None::<&()>,
            0.0f64,
            "reading a device's sample rate",
        )
    }

    fn set_device_sample_rate(&self, device: ObjectId, rate: f64) -> Result<(), HalError> {
        let address = global(kAudioDevicePropertyNominalSampleRate);
        // SAFETY: pointers to live locals of the size stated.
        let status = unsafe {
            AudioObjectSetPropertyData(
                device,
                NonNull::from(&address),
                0,
                ptr::null(),
                size_of::<f64>() as u32,
                NonNull::from(&rate).cast(),
            )
        };
        check(status, "setting the aggregate device's sample rate")
    }

    fn create_io_proc(
        &self,
        device: ObjectId,
        context: NonNull<IoContext>,
    ) -> Result<AudioDeviceIOProcID, HalError> {
        const CALL: &str = "registering the IO proc";
        type BoundIoProc = unsafe extern "C-unwind" fn(
            AudioObjectID,
            NonNull<AudioTimeStamp>,
            NonNull<AudioBufferList>,
            NonNull<AudioTimeStamp>,
            NonNull<AudioBufferList>,
            NonNull<AudioTimeStamp>,
            *mut c_void,
        ) -> i32;
        // SAFETY: the two function types differ only in spelling every
        // pointer parameter as `NonNull<T>` or as a raw pointer, which have
        // the same ABI.
        let proc: AudioDeviceIOProc =
            Some(unsafe { std::mem::transmute::<RawIoProc, BoundIoProc>(io_proc) });
        let mut id: AudioDeviceIOProcID = None;
        // SAFETY: `io_proc` matches the IO proc signature; `context` stays
        // alive until the session has destroyed the proc (see
        // `TapSession::teardown`); `id` is a live local.
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                device,
                proc,
                context.as_ptr().cast(),
                NonNull::from(&mut id),
            )
        };
        check(status, CALL)?;
        if id.is_none() {
            return Err(HalError {
                call: CALL,
                status: kAudioHardwareUnspecifiedError,
            });
        }
        Ok(id)
    }

    fn start(&self, device: ObjectId, io_proc: &AudioDeviceIOProcID) -> Result<(), HalError> {
        // SAFETY: an IO proc ID Core Audio returned for this device.
        check(
            unsafe { AudioDeviceStart(device, *io_proc) },
            "starting IO on the aggregate device",
        )
    }

    /// Reads the tap's description and writes it back unchanged. macOS
    /// refuses the write when the process lacks the System Audio Recording
    /// permission (this is Chromium's `ProbeAudioTapPermissions`).
    fn check_tap_access(&self, tap: ObjectId) -> Result<(), HalError> {
        const CALL: &str = "updating the tap's description";
        let description: *mut CATapDescription = read_value(
            tap,
            kAudioTapPropertyDescription,
            None::<&()>,
            ptr::null_mut(),
            "reading the tap's description",
        )?;
        // SAFETY: the property hands over one retain count of the
        // description object, or null.
        let Some(description) = (unsafe { Retained::from_raw(description) }) else {
            return Err(HalError {
                call: CALL,
                status: kAudioHardwareUnspecifiedError,
            });
        };
        let address = global(kAudioTapPropertyDescription);
        let pointer: *const CATapDescription = Retained::as_ptr(&description);
        // SAFETY: the property's value is one object pointer; `pointer` is a
        // live local holding a live object.
        let status = unsafe {
            AudioObjectSetPropertyData(
                tap,
                NonNull::from(&address),
                0,
                ptr::null(),
                size_of::<*const CATapDescription>() as u32,
                NonNull::from(&pointer).cast(),
            )
        };
        check(status, CALL)
    }

    fn stop(&self, device: ObjectId, io_proc: &AudioDeviceIOProcID) -> Result<(), HalError> {
        // SAFETY: an IO proc ID Core Audio returned for this device.
        check(
            unsafe { AudioDeviceStop(device, *io_proc) },
            "stopping IO on the aggregate device",
        )
    }

    fn destroy_io_proc(
        &self,
        device: ObjectId,
        io_proc: AudioDeviceIOProcID,
    ) -> Result<(), HalError> {
        // SAFETY: an IO proc ID Core Audio returned for this device.
        check(
            unsafe { AudioDeviceDestroyIOProcID(device, io_proc) },
            "destroying the IO proc",
        )
    }

    fn destroy_aggregate(&self, device: ObjectId) -> Result<(), HalError> {
        // SAFETY: takes an object ID by value.
        check(
            unsafe { AudioHardwareDestroyAggregateDevice(device) },
            "destroying the aggregate device",
        )
    }

    fn destroy_tap(&self, tap: ObjectId) -> Result<(), HalError> {
        // SAFETY: takes an object ID by value.
        check(
            unsafe { AudioHardwareDestroyProcessTap(tap) },
            "destroying the process tap",
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use objc2_core_audio::{kAudioHardwarePropertyDevices, kAudioHardwarePropertyTapList};

    use super::*;
    use crate::capture::StreamEvents;
    use crate::system::format::TapFormat;
    use crate::system::session::{TapConfig, TapSession};

    /// The default output device is read the way `open` reads it. A property
    /// read: nothing is created, nothing captured, no prompt. Passes
    /// trivially on a machine with no output device.
    #[test]
    fn the_default_output_device_can_be_described() {
        let hal = CoreAudioHal;
        let Some(device) = hal.default_output() else {
            println!("no default output device");
            return;
        };
        println!(
            "default output {}: {} Hz, identity {}",
            device.uid,
            device.sample_rate,
            device.identity()
        );
        assert!(!device.uid.is_empty());
        assert!(device.sample_rate > 0.0);
        // Asking twice gives the same answer; change detection relies on it.
        assert_eq!(
            hal.default_output().map(|d| d.identity()),
            Some(device.identity())
        );
    }

    /// Translating a PID is a property read too.
    #[test]
    fn process_objects_are_looked_up_by_pid() {
        let hal = CoreAudioHal;
        // This process has talked to Core Audio by the second call at the
        // latest; whether that makes it a client with a process object is up
        // to the system, so only consistency is asserted.
        let _ = hal.default_output();
        let own = hal.process_object(std::process::id() as i32);
        println!("process object of this test process: {own:?}");
        assert_ne!(own, Some(kAudioObjectUnknown));
        // PID 0 is the kernel: never an audio client.
        assert_eq!(hal.process_object(0), None);
    }

    /// Object lists are read without error, and the tap list is empty or not
    /// depending on what else runs on this machine.
    #[test]
    fn device_and_tap_lists_can_be_read() {
        let devices = read_object_list(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyDevices,
            "reading the device list",
        )
        .unwrap();
        let taps = read_object_list(
            SYSTEM_OBJECT,
            kAudioHardwarePropertyTapList,
            "reading the tap list",
        )
        .unwrap();
        println!("{} device(s), {} tap(s) visible", devices.len(), taps.len());
        assert!(devices.iter().all(|id| *id != kAudioObjectUnknown));
    }

    #[test]
    fn reading_a_property_of_a_missing_object_is_an_error_not_a_crash() {
        // No object has this ID.
        let error = CoreAudioHal.device_sample_rate(0x7fff_fff0).unwrap_err();
        assert_eq!(error.call, "reading a device's sample rate");
        assert_ne!(error.status, 0);
        println!("{error}");
        assert!(read_string(0x7fff_fff0, kAudioDevicePropertyDeviceUID, "reading a UID").is_err());
    }

    /// The description handed to `AudioHardwareCreateProcessTap` says what
    /// the issue asks for. It is only built here, never passed on.
    #[test]
    fn the_tap_description_is_a_private_unmuted_stereo_global_tap() {
        let description = tap_description(&[77, 78]);
        // SAFETY: property getters on a live object.
        unsafe {
            assert!(description.isExclusive(), "taps everything but the list");
            assert!(description.isMixdown());
            assert!(!description.isMono(), "stereo mixdown");
            assert!(description.isPrivate());
            assert_eq!(description.isMuted(), CATapMuteBehavior::Unmuted);
            assert_eq!(description.name().to_string(), OBJECT_NAME);
            let excluded: Vec<u32> = description
                .processes()
                .to_vec()
                .iter()
                .map(|n| n.as_u32())
                .collect();
            assert_eq!(excluded, [77, 78]);
            // Not tied to one output device: a global tap.
            assert!(description.deviceUID().is_none());
            assert!(description.stream().is_none());
            // The UID the aggregate device will refer to.
            let uid = description.UUID().UUIDString().to_string();
            assert_eq!(uid.len(), 36, "{uid}");
        }
        // Excluding nothing is valid too.
        let description = tap_description(&[]);
        // SAFETY: as above.
        unsafe {
            assert!(description.isExclusive());
            assert_eq!(description.processes().len(), 0);
        }
    }

    /// The dictionary handed to `AudioHardwareCreateAggregateDevice`. It is
    /// only built here, never passed on.
    #[test]
    fn the_aggregate_description_holds_one_tap_and_is_private() {
        let description = aggregate_description("device-uid", "tap-uid");
        assert_eq!(description.len(), 5);
        let get = |key: &CStr| description.get(&cf_key(key)).unwrap();
        let string = |value: CFRetained<CFType>| value.downcast::<CFString>().unwrap().to_string();
        let boolean = |value: CFRetained<CFType>| value.downcast::<CFBoolean>().unwrap().as_bool();

        assert_eq!(string(get(kAudioAggregateDeviceNameKey)), OBJECT_NAME);
        assert_eq!(string(get(kAudioAggregateDeviceUIDKey)), "device-uid");
        assert!(boolean(get(kAudioAggregateDeviceIsPrivateKey)));
        assert!(!boolean(get(kAudioAggregateDeviceTapAutoStartKey)));

        let taps = get(kAudioAggregateDeviceTapListKey)
            .downcast::<CFArray>()
            .unwrap();
        assert_eq!(taps.len(), 1);
        // SAFETY: the array was built from dictionaries with CFString keys.
        let taps =
            unsafe { CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(taps) };
        let tap = taps.get(0).unwrap();
        assert_eq!(tap.len(), 2);
        assert_eq!(
            string(tap.get(&cf_key(kAudioSubTapUIDKey)).unwrap()),
            "tap-uid"
        );
        assert!(boolean(
            tap.get(&cf_key(kAudioSubTapDriftCompensationKey)).unwrap()
        ));
    }

    #[test]
    fn dictionary_keys_are_the_ones_in_apples_headers() {
        assert_eq!(kAudioAggregateDeviceTapListKey.to_str().unwrap(), "taps");
        assert_eq!(
            kAudioAggregateDeviceTapAutoStartKey.to_str().unwrap(),
            "tapautostart"
        );
        assert_eq!(
            kAudioAggregateDeviceIsPrivateKey.to_str().unwrap(),
            "private"
        );
        assert_eq!(kAudioSubTapUIDKey.to_str().unwrap(), "uid");
        assert_eq!(kAudioSubTapDriftCompensationKey.to_str().unwrap(), "drift");
    }

    /// **Live.** Opens a real tap session and closes it again, then checks
    /// that the tap and the aggregate device are gone. Creates a tap and
    /// starts IO, so macOS may show the System Audio Recording prompt.
    ///
    /// ```sh
    /// cargo test -p mneme-audio --lib -- --ignored --nocapture live_tap_session
    /// ```
    #[test]
    #[ignore = "creates a real Core Audio tap; needs the System Audio Recording permission"]
    fn live_tap_session_leaves_nothing_behind() {
        let hal = CoreAudioHal;
        let list = |selector, call| read_object_list(SYSTEM_OBJECT, selector, call).unwrap();
        let devices_before = list(kAudioHardwarePropertyDevices, "reading the device list");
        let taps_before = list(kAudioHardwarePropertyTapList, "reading the tap list");

        let config = TapConfig {
            excluded_pids: vec![std::process::id() as i32],
            check_access: std::env::var_os("MNEME_SKIP_TAP_ACCESS_CHECK").is_none(),
        };
        let session =
            TapSession::open(hal, &config, StreamEvents::detached()).expect("open a tap session");
        let (tap, aggregate) = (session.tap_id().unwrap(), session.aggregate_id().unwrap());
        let format = TapFormat::from_description(&hal.tap_format(tap).unwrap()).unwrap();
        println!("tap {tap} delivers {format:?}");
        println!(
            "aggregate device {aggregate} runs at {} Hz (default output: {:?})",
            session.sample_rate(),
            hal.default_output()
        );
        println!(
            "while open: tap listed = {}, aggregate listed = {}",
            list(kAudioHardwarePropertyTapList, "reading the tap list").contains(&tap),
            list(kAudioHardwarePropertyDevices, "reading the device list").contains(&aggregate),
        );
        std::thread::sleep(std::time::Duration::from_millis(500));

        let mut session = session;
        let report = session.teardown();
        assert_eq!(report.errors, [], "teardown calls failed");
        assert!(!report.context_leaked);
        drop(session);

        // The objects no longer answer...
        assert!(hal.tap_format(tap).is_err(), "the tap still exists");
        assert!(
            hal.device_sample_rate(aggregate).is_err(),
            "the aggregate device still exists"
        );
        // ...and the lists are what they were.
        let devices_after = list(kAudioHardwarePropertyDevices, "reading the device list");
        let taps_after = list(kAudioHardwarePropertyTapList, "reading the tap list");
        assert!(!devices_after.contains(&aggregate));
        assert!(!taps_after.contains(&tap));
        println!(
            "devices {} -> {}, taps {} -> {}",
            devices_before.len(),
            devices_after.len(),
            taps_before.len(),
            taps_after.len()
        );
    }
}
