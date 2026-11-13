//! Capture permissions: check them and ask for them.
//!
//! The microphone has a public API for both (AVFoundation). System audio
//! does not: see [`system_audio_permission_status`].

/// What the OS says about this app's access to a capture device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionStatus {
    /// The user has not been asked yet. Requesting shows the system prompt.
    NotDetermined,
    /// Access is blocked by policy (parental controls, device management);
    /// the user cannot grant it.
    Restricted,
    /// The user said no. Only System Settings can change that; requesting
    /// again shows nothing.
    Denied,
    /// Access is allowed.
    Granted,
}

impl PermissionStatus {
    /// `true` only for [`PermissionStatus::Granted`].
    pub fn is_granted(self) -> bool {
        self == PermissionStatus::Granted
    }
}

/// Returns the current microphone permission without prompting and without
/// opening the device.
///
/// macOS decides per app. A command-line binary has no `Info.plist` of its
/// own, so macOS attributes it to the app that launched it (Terminal, an
/// IDE), and it is that app's entry in System Settings > Privacy & Security >
/// Microphone that counts.
///
/// On other platforms this always returns [`PermissionStatus::Granted`].
pub fn microphone_permission_status() -> PermissionStatus {
    imp::status()
}

/// Asks for microphone permission and returns the outcome.
///
/// Shows the system prompt only when the status is
/// [`NotDetermined`](PermissionStatus::NotDetermined) and then **blocks until
/// the user answers**, so call it off the UI thread. In every other state it
/// returns the current status at once.
///
/// The app's `Info.plist` must contain `NSMicrophoneUsageDescription`; its
/// text is shown in the prompt. macOS terminates a process that requests
/// access without one (its own, or that of the app responsible for it, as
/// described on [`microphone_permission_status`]).
///
/// On other platforms this always returns [`PermissionStatus::Granted`].
pub fn request_microphone_permission() -> PermissionStatus {
    imp::request()
}

/// Returns what is known about the System Audio Recording permission (the
/// one Core Audio taps need), without prompting and without touching the
/// audio system. `None` means **unknown**.
///
/// # What macOS exposes
///
/// Nothing public. There is no counterpart of
/// `AVCaptureDevice.authorizationStatus` for audio capture: Apple's
/// documentation only says that the system prompts "the first time you start
/// recording from an aggregate device that contains a tap", and a process
/// without the permission gets no error, only silence.
///
/// So this returns `None` unless the crate is built with the `tcc-spi`
/// feature. With it, the answer comes from `TCCAccessPreflight` for
/// `kTCCServiceAudioCapture`, a private function of Apple's TCC framework
/// that is looked up at run time (AudioCap, the sample most users of taps
/// start from, reads the status the same way). It tells
/// [`Granted`](PermissionStatus::Granted),
/// [`Denied`](PermissionStatus::Denied) and
/// [`NotDetermined`](PermissionStatus::NotDetermined) apart; it never
/// reports [`Restricted`](PermissionStatus::Restricted). If the function is
/// missing on some macOS version the result is `None` again. Being private,
/// it can change or disappear without notice, and is not allowed in the Mac
/// App Store.
///
/// Whatever this returns,
/// [`SystemAudioSource::start`](crate::AudioSource::start) checks access
/// itself, with public API, once the tap exists, and fails with
/// [`AudioError::PermissionDenied`](crate::AudioError::PermissionDenied) if
/// it is missing. This function only lets a UI show the state beforehand.
///
/// As with the microphone, macOS decides per app, and attributes a
/// command-line binary to the app that launched it.
pub fn system_audio_permission_status() -> Option<PermissionStatus> {
    system_audio::status()
}

/// Asks for the System Audio Recording permission and returns the outcome,
/// or `None` if the outcome could not be determined.
///
/// **Blocks while the system prompt is on screen**, so call it off the UI
/// thread. The app's `Info.plist` must contain
/// `NSAudioCaptureUsageDescription`.
///
/// With the `tcc-spi` feature this calls the private `TCCAccessRequest`
/// when the status is [`NotDetermined`](PermissionStatus::NotDetermined) and
/// returns the user's answer; in the other states it returns the status
/// without prompting.
///
/// Without the feature (or if the private function is missing) it does the
/// one thing public API offers: it opens a tap for a moment, exactly as
/// [`SystemAudioSource`](crate::SystemAudioSource) does, which makes macOS
/// prompt if it has not asked before, checks whether the tap may be read,
/// and closes it again. `Some(Granted)` and `Some(Denied)` are what that
/// check found. If macOS turns out to show the prompt without waiting for
/// the answer, the first call returns `Some(Denied)` while the prompt is
/// still open; call again after the user has answered. `None` means the tap
/// could not be opened for another reason (no output device, for one).
///
/// On other platforms this returns `None`.
pub fn request_system_audio_permission() -> Option<PermissionStatus> {
    system_audio::request()
}

mod system_audio {
    use super::PermissionStatus;
    use crate::AudioError;

    pub(super) fn status() -> Option<PermissionStatus> {
        #[cfg(all(target_os = "macos", feature = "tcc-spi"))]
        {
            tcc::preflight()
        }
        #[cfg(not(all(target_os = "macos", feature = "tcc-spi")))]
        {
            None
        }
    }

    pub(super) fn request() -> Option<PermissionStatus> {
        #[cfg(all(target_os = "macos", feature = "tcc-spi"))]
        match tcc::preflight() {
            Some(PermissionStatus::NotDetermined) => {
                if let Some(answer) = tcc::request() {
                    return Some(answer);
                }
            }
            Some(known) => return Some(known),
            None => {}
        }
        from_probe(crate::system::probe_access())
    }

    /// Reads the result of opening a tap for a moment.
    pub(super) fn from_probe(result: Result<(), AudioError>) -> Option<PermissionStatus> {
        match result {
            Ok(()) => Some(PermissionStatus::Granted),
            Err(AudioError::PermissionDenied(_)) => Some(PermissionStatus::Denied),
            Err(_) => None,
        }
    }

    /// `TCCAccessPreflight` returns 0 for granted and 1 for denied; anything
    /// else means the user has not been asked.
    #[cfg(any(test, all(target_os = "macos", feature = "tcc-spi")))]
    pub(super) fn from_preflight(result: i32) -> PermissionStatus {
        match result {
            0 => PermissionStatus::Granted,
            1 => PermissionStatus::Denied,
            _ => PermissionStatus::NotDetermined,
        }
    }

    /// The two private TCC functions, resolved at run time so that a macOS
    /// without them costs nothing but an unknown status.
    #[cfg(all(target_os = "macos", feature = "tcc-spi"))]
    mod tcc {
        use std::ffi::{CStr, c_char, c_int, c_void};
        use std::sync::mpsc;

        use block2::{Block, RcBlock};
        use objc2_core_foundation::CFString;

        use super::{PermissionStatus, from_preflight};

        unsafe extern "C" {
            fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
            fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        }

        /// `RTLD_LAZY` in `<dlfcn.h>`.
        const RTLD_LAZY: c_int = 0x1;
        const TCC: &CStr = c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC";
        const SERVICE: &str = "kTCCServiceAudioCapture";

        /// `TCCAccessPreflight(CFStringRef service, CFDictionaryRef options)`.
        type Preflight = unsafe extern "C" fn(*const CFString, *const c_void) -> c_int;
        /// `TCCAccessRequest(CFStringRef service, CFDictionaryRef options,
        /// void (^completion)(Boolean granted))`.
        type Request = unsafe extern "C" fn(*const CFString, *const c_void, &Block<dyn Fn(u8)>);

        fn symbol(name: &CStr) -> Option<*mut c_void> {
            // SAFETY: both calls take NUL-terminated strings and report
            // failure as null. The framework is never unloaded.
            let symbol = unsafe {
                let handle = dlopen(TCC.as_ptr(), RTLD_LAZY);
                if handle.is_null() {
                    return None;
                }
                dlsym(handle, name.as_ptr())
            };
            (!symbol.is_null()).then_some(symbol)
        }

        pub(super) fn preflight() -> Option<PermissionStatus> {
            let symbol = symbol(c"TCCAccessPreflight")?;
            // SAFETY: the symbol is the function of that name, called with
            // the signature other callers of this SPI use.
            let preflight = unsafe { std::mem::transmute::<*mut c_void, Preflight>(symbol) };
            let service = CFString::from_static_str(SERVICE);
            // SAFETY: a live CFString and no options.
            let result = unsafe { preflight(&*service, std::ptr::null()) };
            Some(from_preflight(result))
        }

        pub(super) fn request() -> Option<PermissionStatus> {
            let symbol = symbol(c"TCCAccessRequest")?;
            // SAFETY: as in `preflight`.
            let request = unsafe { std::mem::transmute::<*mut c_void, Request>(symbol) };
            let service = CFString::from_static_str(SERVICE);
            let (tx, rx) = mpsc::channel();
            let completion = RcBlock::new(move |granted: u8| {
                let _ = tx.send(granted != 0);
            });
            // SAFETY: a live CFString, no options, and a block the callee
            // copies and calls once on a queue of its choosing, which a
            // channel sender allows.
            unsafe { request(&*service, std::ptr::null(), &completion) };
            match rx.recv() {
                Ok(true) => Some(PermissionStatus::Granted),
                Ok(false) => Some(PermissionStatus::Denied),
                Err(_) => preflight(),
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    use super::PermissionStatus;

    pub(super) fn status() -> PermissionStatus {
        // SAFETY: reading an immutable framework constant.
        let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
            return PermissionStatus::NotDetermined;
        };
        // SAFETY: a class method taking a valid media type constant;
        // AVMediaTypeAudio is one of the two values it accepts.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
        from_av(status)
    }

    pub(super) fn request() -> PermissionStatus {
        let current = status();
        if current != PermissionStatus::NotDetermined {
            return current;
        }
        // SAFETY: as in `status`.
        let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
            return current;
        };
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |granted: Bool| {
            let _ = tx.send(granted.as_bool());
        });
        // SAFETY: valid media type; the block is copied by the framework and
        // called once, on an arbitrary queue, which a channel sender allows.
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &handler);
        }
        match rx.recv() {
            Ok(true) => PermissionStatus::Granted,
            Ok(false) => PermissionStatus::Denied,
            Err(_) => status(),
        }
    }

    pub(super) fn from_av(status: AVAuthorizationStatus) -> PermissionStatus {
        if status == AVAuthorizationStatus::Authorized {
            PermissionStatus::Granted
        } else if status == AVAuthorizationStatus::Denied {
            PermissionStatus::Denied
        } else if status == AVAuthorizationStatus::Restricted {
            PermissionStatus::Restricted
        } else {
            PermissionStatus::NotDetermined
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::PermissionStatus;

    pub(super) fn status() -> PermissionStatus {
        PermissionStatus::Granted
    }

    pub(super) fn request() -> PermissionStatus {
        PermissionStatus::Granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_granted_counts_as_granted() {
        assert!(PermissionStatus::Granted.is_granted());
        assert!(!PermissionStatus::NotDetermined.is_granted());
        assert!(!PermissionStatus::Denied.is_granted());
        assert!(!PermissionStatus::Restricted.is_granted());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn os_status_values_map_to_ours() {
        use objc2_av_foundation::AVAuthorizationStatus as Av;
        assert_eq!(
            imp::from_av(Av::NotDetermined),
            PermissionStatus::NotDetermined
        );
        assert_eq!(imp::from_av(Av::Restricted), PermissionStatus::Restricted);
        assert_eq!(imp::from_av(Av::Denied), PermissionStatus::Denied);
        assert_eq!(imp::from_av(Av::Authorized), PermissionStatus::Granted);
    }

    /// Reading the status never prompts and never opens the device.
    #[test]
    fn reading_the_status_does_not_panic() {
        let status = microphone_permission_status();
        println!("microphone permission for the process running this test: {status:?}");
    }

    #[test]
    fn tcc_preflight_results_map_to_ours() {
        assert_eq!(system_audio::from_preflight(0), PermissionStatus::Granted);
        assert_eq!(system_audio::from_preflight(1), PermissionStatus::Denied);
        assert_eq!(
            system_audio::from_preflight(2),
            PermissionStatus::NotDetermined
        );
        assert_eq!(
            system_audio::from_preflight(-1),
            PermissionStatus::NotDetermined
        );
    }

    #[test]
    fn opening_a_tap_tells_granted_from_denied_and_from_unknown() {
        use crate::AudioError;
        assert_eq!(
            system_audio::from_probe(Ok(())),
            Some(PermissionStatus::Granted)
        );
        assert_eq!(
            system_audio::from_probe(Err(AudioError::PermissionDenied("no".to_owned()))),
            Some(PermissionStatus::Denied)
        );
        // Anything else says nothing about the permission.
        assert_eq!(
            system_audio::from_probe(Err(AudioError::NoOutputDevice)),
            None
        );
        assert_eq!(
            system_audio::from_probe(Err(AudioError::Device("busy".to_owned()))),
            None
        );
    }

    /// Without the `tcc-spi` feature macOS offers no way to read the status.
    #[cfg(not(feature = "tcc-spi"))]
    #[test]
    fn the_system_audio_status_is_unknown_without_the_private_api() {
        assert_eq!(system_audio_permission_status(), None);
    }

    /// With it, the preflight is read. It is a status query: it shows no
    /// prompt and creates nothing in the audio system.
    #[cfg(all(target_os = "macos", feature = "tcc-spi"))]
    #[test]
    fn the_system_audio_status_can_be_read_with_the_private_api() {
        let status = system_audio_permission_status();
        println!("system audio permission for the process running this test: {status:?}");
        assert!(status.is_some(), "TCCAccessPreflight was not found");
        assert_ne!(status, Some(PermissionStatus::Restricted));
    }
}
