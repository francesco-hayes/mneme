//! Microphone permission: check it and ask for it.

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
}
