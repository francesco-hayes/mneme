//! The permission decisions, as pure functions (issue 024).
//!
//! Nothing here asks macOS anything: the functions take a permission state
//! and say what the app does with it. [`AppCore`](crate::app::AppCore) calls
//! them with what the [`Host`](crate::app::Host) reports, tests with made-up
//! states.
//!
//! The microphone can be read and asked for with public API. System audio
//! cannot: macOS has no public call to read that permission, and a tap
//! without it delivers silence. The one reliable signal is the access check
//! in `SystemAudioSource::start`, so what the app knows about system audio is
//! what its recordings learned ([`SystemAudioSeen`]), for this run only.

use mneme_audio::{AudioError, PermissionStatus};
use mneme_core::Speaker;
use mneme_pipeline::SourcePhase;

use crate::dto;

/// The two capture permissions, as `requestPermission` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    Microphone,
    SystemAudio,
}

impl PermissionKind {
    pub fn parse(kind: &str) -> Result<Self, String> {
        match kind {
            "microphone" => Ok(Self::Microphone),
            "systemAudio" => Ok(Self::SystemAudio),
            other => Err(format!("Unknown permission: {other}")),
        }
    }

    /// Where the permission is changed, as the user reads it in System
    /// Settings.
    pub fn pane(self) -> &'static str {
        match self {
            Self::Microphone => "System Settings > Privacy & Security > Microphone",
            Self::SystemAudio => {
                "System Settings > Privacy & Security > Screen & System Audio Recording, \
                 under System Audio Recording Only"
            }
        }
    }

    /// The URL that opens that pane. The anchors are the names the Privacy &
    /// Security pane uses for the two services; whether they still scroll to
    /// the right list is a manual check. An anchor macOS does not know opens
    /// Privacy & Security itself.
    pub fn settings_url(self) -> &'static str {
        match self {
            Self::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            Self::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture"
            }
        }
    }
}

/// A status as the contract spells it. `Restricted` (blocked by a policy)
/// is `denied`: the contract has three values.
pub fn status_name(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Granted => dto::GRANTED,
        PermissionStatus::Denied | PermissionStatus::Restricted => dto::DENIED,
        PermissionStatus::NotDetermined => dto::UNDETERMINED,
    }
}

/// What this run's recordings have learned about system audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemAudioSeen {
    /// No recording has settled it yet.
    #[default]
    Unknown,
    /// The system source was refused access.
    Refused,
    /// The system source ran and delivered audio.
    Delivering,
}

/// What a source's status says about the system audio permission, or `None`
/// if it says nothing (still starting, no frames yet, or it failed for a
/// reason that is not the permission, such as no output device).
pub fn system_audio_evidence(
    phase: SourcePhase,
    errors: &[AudioError],
    frames: u64,
) -> Option<SystemAudioSeen> {
    let refused = |error: &AudioError| matches!(error, AudioError::PermissionDenied(_));
    match phase {
        // The first error is the one `start` returned.
        SourcePhase::FailedToStart => errors
            .first()
            .is_some_and(refused)
            .then_some(SystemAudioSeen::Refused),
        // Access taken away while it ran.
        SourcePhase::Ended if errors.iter().any(refused) => Some(SystemAudioSeen::Refused),
        SourcePhase::Running | SourcePhase::Ended | SourcePhase::Stopped if frames > 0 => {
            Some(SystemAudioSeen::Delivering)
        }
        _ => None,
    }
}

/// What `getPermissions` reports for system audio. An answer from macOS
/// itself (`host`, only with private API) comes first; otherwise it is what
/// the recordings of this run learned, and `undetermined` until one did.
pub fn system_audio_status(host: Option<PermissionStatus>, seen: SystemAudioSeen) -> &'static str {
    match (host, seen) {
        (Some(status), _) => status_name(status),
        (None, SystemAudioSeen::Delivering) => dto::GRANTED,
        (None, SystemAudioSeen::Refused) => dto::DENIED,
        (None, SystemAudioSeen::Unknown) => dto::UNDETERMINED,
    }
}

/// What `requestPermission` does for a permission in this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestAction {
    /// Already granted: nothing to do.
    Nothing,
    /// Not asked yet: show the system prompt.
    Prompt,
    /// Denied: macOS does not ask twice, so open the pane in System
    /// Settings. The request still resolves with `denied`.
    OpenSettings,
}

/// `status` is a contract value (`granted`, `denied`, `undetermined`).
pub fn request_action(status: &str) -> RequestAction {
    match status {
        dto::GRANTED => RequestAction::Nothing,
        dto::DENIED => RequestAction::OpenSettings,
        _ => RequestAction::Prompt,
    }
}

/// Whether a meeting may start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartDecision {
    /// Both channels are expected.
    Record,
    /// The microphone alone, with the sentence that says why. The system
    /// source is still tried: that attempt is the only way to notice that
    /// access was given in System Settings meanwhile.
    MicOnly(String),
    /// No meeting, with the sentence the UI shows.
    Refuse(String),
}

/// Decides a start. `microphone` is the status after the prompt, if one was
/// due; `system_audio` is what `getPermissions` reports.
pub fn start_decision(microphone: PermissionStatus, system_audio: &str) -> StartDecision {
    if !microphone.is_granted() {
        return StartDecision::Refuse(microphone_refusal(microphone));
    }
    if system_audio == dto::DENIED {
        return StartDecision::MicOnly(system_audio_refusal());
    }
    StartDecision::Record
}

/// Why a meeting cannot start without the microphone, and the fix.
pub fn microphone_refusal(status: PermissionStatus) -> String {
    match status {
        PermissionStatus::Restricted => "Microphone access is blocked on this Mac by a policy \
             (Screen Time or device management), so Mneme cannot record."
            .to_owned(),
        _ => format!(
            "Microphone access is denied. Allow Mneme in {}. If it still cannot record \
             after that, quit and reopen Mneme.",
            PermissionKind::Microphone.pane()
        ),
    }
}

/// Why a meeting records the microphone only, and the fix.
pub fn system_audio_refusal() -> String {
    format!(
        "System audio access is denied, so this meeting records the microphone only. \
         Allow Mneme in {}, then quit and reopen Mneme.",
        PermissionKind::SystemAudio.pane()
    )
}

/// The sentence for a channel that is not being recorded: it failed to
/// start, or stopped by itself during the meeting. Names the pane when the
/// reason is the permission.
pub fn channel_lost(speaker: Speaker, phase: SourcePhase, errors: &[AudioError]) -> String {
    let refused = errors
        .iter()
        .any(|e| matches!(e, AudioError::PermissionDenied(_)));
    let (channel, kind) = match speaker {
        Speaker::Me => ("The microphone", PermissionKind::Microphone),
        Speaker::Them => ("System audio", PermissionKind::SystemAudio),
    };
    let what = if phase == SourcePhase::FailedToStart {
        "is not being recorded"
    } else {
        "stopped being recorded"
    };
    if refused {
        return format!(
            "{channel} {what}: access is denied. Allow Mneme in {}, then quit and reopen Mneme.",
            kind.pane()
        );
    }
    let reason = match phase {
        // What `start` returned.
        SourcePhase::FailedToStart => errors.first(),
        _ => errors.last(),
    };
    match reason {
        Some(error) => format!("{channel} {what}: {error}"),
        None => format!("{channel} {what}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIC_PANE: &str = "Privacy & Security > Microphone";
    const SYSTEM_PANE: &str = "Privacy & Security > Screen & System Audio Recording";

    fn denied() -> AudioError {
        AudioError::PermissionDenied("tap access refused".to_owned())
    }

    #[test]
    fn a_start_is_allowed_mic_only_or_refused() {
        use PermissionStatus::{Denied, Granted, NotDetermined, Restricted};

        // Unknown system audio does not hold a meeting back: the first
        // recording is what settles it.
        for system_audio in [dto::GRANTED, dto::UNDETERMINED] {
            assert_eq!(start_decision(Granted, system_audio), StartDecision::Record);
        }

        let StartDecision::MicOnly(why) = start_decision(Granted, dto::DENIED) else {
            panic!("system audio denied records the microphone only");
        };
        assert!(why.contains(SYSTEM_PANE), "{why}");
        assert!(why.contains("System Audio Recording Only"), "{why}");
        assert!(why.contains("quit and reopen"), "{why}");
        assert!(!why.contains(MIC_PANE), "{why}");

        // Without the microphone there is no meeting, whatever system audio
        // says. `NotDetermined` here means the prompt gave no answer.
        for microphone in [Denied, Restricted, NotDetermined] {
            for system_audio in [dto::GRANTED, dto::UNDETERMINED, dto::DENIED] {
                assert!(
                    matches!(
                        start_decision(microphone, system_audio),
                        StartDecision::Refuse(_)
                    ),
                    "{microphone:?} / {system_audio}"
                );
            }
        }
        let StartDecision::Refuse(why) = start_decision(Denied, dto::GRANTED) else {
            unreachable!();
        };
        assert!(why.starts_with("Microphone access is denied."), "{why}");
        assert!(why.contains(MIC_PANE), "{why}");
        assert!(!why.contains("Screen & System Audio"), "{why}");

        // A policy cannot be fixed in System Settings: no pane is named.
        let blocked = microphone_refusal(Restricted);
        assert!(blocked.contains("policy"), "{blocked}");
        assert!(!blocked.contains("Privacy & Security"), "{blocked}");
    }

    #[test]
    fn each_permission_names_and_opens_its_own_pane() {
        let microphone = PermissionKind::Microphone;
        let system_audio = PermissionKind::SystemAudio;
        assert!(microphone.pane().ends_with(MIC_PANE));
        assert!(system_audio.pane().contains(SYSTEM_PANE));
        assert!(
            system_audio
                .pane()
                .ends_with("under System Audio Recording Only")
        );
        assert_eq!(
            microphone.settings_url(),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        );
        assert_eq!(
            system_audio.settings_url(),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture"
        );

        assert_eq!(PermissionKind::parse("microphone"), Ok(microphone));
        assert_eq!(PermissionKind::parse("systemAudio"), Ok(system_audio));
        assert!(PermissionKind::parse("camera").is_err());
    }

    #[test]
    fn a_request_prompts_when_undetermined_and_opens_settings_when_denied() {
        assert_eq!(request_action(dto::UNDETERMINED), RequestAction::Prompt);
        assert_eq!(request_action(dto::DENIED), RequestAction::OpenSettings);
        assert_eq!(request_action(dto::GRANTED), RequestAction::Nothing);
    }

    #[test]
    fn a_recording_start_settles_system_audio() {
        use SourcePhase::{Abandoned, Ended, FailedToStart, Running, Starting, Stopped};
        use SystemAudioSeen::{Delivering, Refused};

        // Refused by the access check in `start`.
        assert_eq!(
            system_audio_evidence(FailedToStart, &[denied()], 0),
            Some(Refused)
        );
        // Failed for another reason: the permission is no better known.
        let no_device = [AudioError::NoOutputDevice];
        assert_eq!(system_audio_evidence(FailedToStart, &no_device, 0), None);
        let later = [AudioError::NoOutputDevice, denied()];
        assert_eq!(system_audio_evidence(FailedToStart, &later, 0), None);

        // Delivering: it ran and frames arrived.
        for phase in [Running, Stopped, Ended] {
            assert_eq!(system_audio_evidence(phase, &[], 12), Some(Delivering));
        }
        // An error that did not end it does not undo that.
        let hiccup = [AudioError::Stream("glitch".to_owned())];
        assert_eq!(
            system_audio_evidence(Running, &hiccup, 12),
            Some(Delivering)
        );
        // Access taken away mid-recording.
        assert_eq!(system_audio_evidence(Ended, &[denied()], 12), Some(Refused));

        // Nothing learned yet: a prompt may be open, or no frame has come.
        for phase in [Starting, Running, Stopped, Abandoned] {
            assert_eq!(system_audio_evidence(phase, &[], 0), None, "{phase:?}");
        }
    }

    #[test]
    fn get_permissions_reports_what_the_recordings_learned() {
        use SystemAudioSeen::{Delivering, Refused, Unknown};

        assert_eq!(system_audio_status(None, Unknown), "undetermined");
        assert_eq!(system_audio_status(None, Refused), "denied");
        assert_eq!(system_audio_status(None, Delivering), "granted");

        // An answer from macOS itself (private API only) beats the memory:
        // it also sees a change made in System Settings.
        let granted = Some(PermissionStatus::Granted);
        assert_eq!(system_audio_status(granted, Refused), "granted");
        let denied = Some(PermissionStatus::Denied);
        assert_eq!(system_audio_status(denied, Delivering), "denied");
        let not_asked = Some(PermissionStatus::NotDetermined);
        assert_eq!(system_audio_status(not_asked, Unknown), "undetermined");

        assert_eq!(status_name(PermissionStatus::Restricted), "denied");
    }

    #[test]
    fn a_lost_channel_is_named_with_its_reason_and_its_pane() {
        let refused = channel_lost(Speaker::Them, SourcePhase::FailedToStart, &[denied()]);
        assert!(
            refused.starts_with("System audio is not being recorded"),
            "{refused}"
        );
        assert!(refused.contains(SYSTEM_PANE), "{refused}");

        let revoked = channel_lost(Speaker::Me, SourcePhase::Ended, &[denied()]);
        assert!(
            revoked.starts_with("The microphone stopped being recorded"),
            "{revoked}"
        );
        assert!(revoked.contains(MIC_PANE), "{revoked}");
        assert!(!revoked.contains("Screen & System Audio"), "{revoked}");

        // Not a permission problem: the reason, and no pane.
        let no_device = [AudioError::NoOutputDevice];
        let other = channel_lost(Speaker::Them, SourcePhase::FailedToStart, &no_device);
        assert!(other.contains(&no_device[0].to_string()), "{other}");
        assert!(!other.contains("Privacy & Security"), "{other}");
        assert_eq!(
            channel_lost(Speaker::Me, SourcePhase::Ended, &[]),
            "The microphone stopped being recorded."
        );
    }
}
