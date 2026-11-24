//! The frontend contract (`src/lib/api/types.ts`) as serializable types.
//!
//! The area crates' own types do not derive `serde`; they are mapped here.
//! Field names serialize in camelCase, exactly as the contract spells them.

use mneme_store as store;
use mneme_transcribe as transcribe;
use serde::Serialize;

pub const STATUS_RECORDING: &str = "recording";
pub const STATUS_INTERRUPTED: &str = "interrupted";

pub const GRANTED: &str = "granted";
pub const DENIED: &str = "denied";
pub const UNDETERMINED: &str = "undetermined";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    /// RFC 3339 with the local UTC offset of the moment recording started.
    pub started_at: String,
    pub duration_secs: Option<u64>,
    pub status: &'static str,
}

impl MeetingSummary {
    /// `live` says whether this run of the app is recording the meeting. A
    /// meeting that is `recording` on disk with nothing recording it (its
    /// recovery failed) is shown as interrupted, never as live.
    pub fn from_store(summary: &store::MeetingSummary, live: bool) -> Self {
        let status = match summary.status {
            store::MeetingStatus::Recording if !live => STATUS_INTERRUPTED,
            status => status.as_str(),
        };
        Self {
            id: summary.id.as_str().to_owned(),
            title: summary.title.clone(),
            started_at: summary.started_at.to_rfc3339(),
            duration_secs: summary.duration_secs,
            status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    #[serde(flatten)]
    pub summary: MeetingSummary,
    pub notes: String,
    pub transcript_line_count: usize,
    pub folder_path: String,
}

impl Meeting {
    pub fn from_store(meeting: &store::Meeting, live: bool) -> Self {
        Self {
            summary: MeetingSummary::from_store(&meeting.summary, live),
            notes: meeting.notes.clone(),
            transcript_line_count: meeting.transcript_line_count,
            folder_path: meeting.summary.folder.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptLine {
    /// `hh:mm:ss`, as in the transcript file.
    pub t: String,
    pub speaker: &'static str,
    pub text: String,
}

impl From<&store::TranscriptLine> for TranscriptLine {
    fn from(line: &store::TranscriptLine) -> Self {
        Self {
            t: line.timestamp(),
            speaker: line.speaker.as_str(),
            text: line.text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub note: String,
    pub english_only: bool,
    pub installed: bool,
}

impl From<transcribe::ModelInfo> for ModelInfo {
    fn from(model: transcribe::ModelInfo) -> Self {
        Self {
            id: model.id,
            name: model.name,
            size_bytes: model.size_bytes,
            note: model.note,
            english_only: model.english_only,
            installed: model.installed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub model_id: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<transcribe::DownloadProgress> for DownloadProgress {
    fn from(progress: transcribe::DownloadProgress) -> Self {
        Self {
            model_id: progress.model_id,
            received_bytes: progress.received_bytes,
            total_bytes: progress.total_bytes,
            state: progress.state.as_str(),
            error: progress.error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
    /// `idle`, `recording` or `stopping`.
    pub status: &'static str,
    pub meeting_id: Option<String>,
    pub elapsed_secs: u64,
    /// The model the meeting is transcribed with, while recording or
    /// stopping. That model cannot be deleted.
    pub model_id: Option<String>,
    /// Why a channel is not being recorded (system audio refused, a device
    /// that failed), or `None`. Kept for the rest of the meeting.
    pub warning: Option<String>,
    /// A start is loading its model. Only with status `idle`: the meeting
    /// does not exist yet, so a second start would be refused.
    pub starting: bool,
}

impl RecordingState {
    pub const fn idle() -> Self {
        Self {
            status: "idle",
            meeting_id: None,
            elapsed_secs: 0,
            model_id: None,
            warning: None,
            starting: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub microphone: &'static str,
    pub system_audio: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn types_serialize_with_the_contract_field_names() {
        let meeting = Meeting {
            summary: MeetingSummary {
                id: "m1".into(),
                title: "Weekly".into(),
                started_at: "2026-11-12T10:00:00+01:00".into(),
                duration_secs: None,
                status: STATUS_RECORDING,
            },
            notes: "n".into(),
            transcript_line_count: 2,
            folder_path: "/tmp/x".into(),
        };
        assert_eq!(
            serde_json::to_value(&meeting).unwrap(),
            json!({
                "id": "m1", "title": "Weekly", "startedAt": "2026-11-12T10:00:00+01:00",
                "durationSecs": null, "status": "recording", "notes": "n",
                "transcriptLineCount": 2, "folderPath": "/tmp/x"
            })
        );
        assert_eq!(
            serde_json::to_value(RecordingState::idle()).unwrap(),
            json!({
                "status": "idle", "meetingId": null, "elapsedSecs": 0,
                "modelId": null, "warning": null, "starting": false
            })
        );
        let progress = DownloadProgress {
            model_id: "base".into(),
            received_bytes: 1,
            total_bytes: 2,
            state: "downloading",
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            json!({ "modelId": "base", "receivedBytes": 1, "totalBytes": 2, "state": "downloading" })
        );
        assert_eq!(
            serde_json::to_value(Permissions {
                microphone: GRANTED,
                system_audio: UNDETERMINED
            })
            .unwrap(),
            json!({ "microphone": "granted", "systemAudio": "undetermined" })
        );
    }
}
