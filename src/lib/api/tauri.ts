// The Tauri implementation of the API contract: every method is a command or
// an event of the Rust side (src-tauri/src/commands.rs, src-tauri/src/host.rs).

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  Api,
  DownloadProgress,
  Meeting,
  MeetingSummary,
  ModelInfo,
  Permission,
  Permissions,
  RecordingError,
  RecordingState,
  Settings,
  TranscriptLine,
} from "./types";

/** Event names, as `src-tauri/src/host.rs` and `src-tauri/src/tray.rs` send them. */
const RECORDING_STATE_EVENT = "recording-state";
const RECORDING_ERROR_EVENT = "recording-error";
const DOWNLOAD_PROGRESS_EVENT = "download-progress";
/** Sent by `src-tauri/src/tray.rs`: Open Mneme was chosen during a recording. */
const OPEN_MEETING_EVENT = "open-meeting";

/**
 * Calls a backend command. The backend rejects with a plain sentence; the UI
 * expects an `Error`, as the mock throws, so the sentence becomes its message.
 */
async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}

/**
 * Subscribes to a backend event and returns the unsubscribe function at once.
 * `listen` registers asynchronously, so an unsubscribe that comes before it
 * has finished is remembered and carried out as soon as it has.
 */
function subscribe<T>(event: string, cb: (payload: T) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | null = null;
  listen<T>(event, (e) => {
    if (!stopped) cb(e.payload);
  }).then(
    (fn) => {
      if (stopped) fn();
      else unlisten = fn;
    },
    (error: unknown) => {
      console.error(`Could not listen to ${event}`, error);
    },
  );
  return () => {
    stopped = true;
    unlisten?.();
    unlisten = null;
  };
}

export const tauriApi: Api = {
  listMeetings: () => call<MeetingSummary[]>("list_meetings"),
  getMeeting: (id) => call<Meeting>("get_meeting", { id }),
  getTranscript: (id) => call<TranscriptLine[]>("get_transcript", { id }),
  startMeeting: () => call<Meeting>("start_meeting"),
  stopMeeting: () => call<Meeting>("stop_meeting"),
  saveNotes: (id, notes) => call<void>("save_notes", { id, notes }),
  renameMeeting: (id, title) =>
    call<MeetingSummary>("rename_meeting", { id, title }),
  showInFinder: (id) => call<void>("show_in_finder", { id }),
  getRecordingState: () => call<RecordingState>("get_recording_state"),
  onRecordingState: (cb) =>
    subscribe<RecordingState>(RECORDING_STATE_EVENT, cb),
  onRecordingError: (cb) =>
    subscribe<RecordingError>(RECORDING_ERROR_EVENT, cb),
  onOpenMeeting: (cb) => subscribe<string>(OPEN_MEETING_EVENT, cb),
  listModels: () => call<ModelInfo[]>("list_models"),
  downloadModel: (id) => call<void>("download_model", { id }),
  cancelDownload: (id) => call<void>("cancel_download", { id }),
  deleteModel: (id) => call<void>("delete_model", { id }),
  onDownloadProgress: (cb) =>
    subscribe<DownloadProgress>(DOWNLOAD_PROGRESS_EVENT, cb),
  getSettings: () => call<Settings>("get_settings"),
  updateSettings: (patch) => call<Settings>("update_settings", { patch }),
  getPermissions: () => call<Permissions>("get_permissions"),
  requestPermission: (kind) => call<Permission>("request_permission", { kind }),
};
