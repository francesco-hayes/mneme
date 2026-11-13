// The Tauri implementation of the API contract. A stub for now: issue 021
// replaces every method with a call to a backend command or event.

import type { Api } from "./types";

function notImplemented(method: keyof Api): never {
  throw new Error(`api.${method} is not implemented yet (issue 021)`);
}

export const tauriApi: Api = {
  listMeetings: async () => notImplemented("listMeetings"),
  getMeeting: async () => notImplemented("getMeeting"),
  getTranscript: async () => notImplemented("getTranscript"),
  startMeeting: async () => notImplemented("startMeeting"),
  stopMeeting: async () => notImplemented("stopMeeting"),
  saveNotes: async () => notImplemented("saveNotes"),
  renameMeeting: async () => notImplemented("renameMeeting"),
  showInFinder: async () => notImplemented("showInFinder"),
  getRecordingState: async () => notImplemented("getRecordingState"),
  onRecordingState: () => notImplemented("onRecordingState"),
  listModels: async () => notImplemented("listModels"),
  downloadModel: async () => notImplemented("downloadModel"),
  cancelDownload: async () => notImplemented("cancelDownload"),
  deleteModel: async () => notImplemented("deleteModel"),
  onDownloadProgress: () => notImplemented("onDownloadProgress"),
  getSettings: async () => notImplemented("getSettings"),
  updateSettings: async () => notImplemented("updateSettings"),
  getPermissions: async () => notImplemented("getPermissions"),
  requestPermission: async () => notImplemented("requestPermission"),
};
