// The API contract between the UI and the backend, copied from prds/05-ui.md.
// Apart from `export` on each declaration, the lines below are the PRD's code
// block character for character, so this file is excluded from Prettier.
// A change to the contract is made here first and noted in the issue that needed it.

export type MeetingStatus = 'recording' | 'complete' | 'interrupted';
export type Permission = 'granted' | 'denied' | 'undetermined';

export interface MeetingSummary { id: string; title: string; startedAt: string; durationSecs: number | null; status: MeetingStatus }
export interface Meeting extends MeetingSummary { notes: string; transcriptLineCount: number; folderPath: string }
export interface TranscriptLine { t: string; speaker: 'Me' | 'Them'; text: string }
export interface ModelInfo { id: string; name: string; sizeBytes: number; note: string; englishOnly: boolean; installed: boolean }
export interface DownloadProgress { modelId: string; receivedBytes: number; totalBytes: number; state: 'downloading' | 'verifying' | 'done' | 'cancelled' | 'error'; error?: string }
export interface Settings { defaultModelId: string | null; language: string; globalShortcut: string | null; spellcheck: boolean }
export interface RecordingState { status: 'idle' | 'recording' | 'stopping'; meetingId: string | null; elapsedSecs: number; modelId: string | null; warning: string | null }
export interface Permissions { microphone: Permission; systemAudio: Permission }
export interface RecordingError { action: 'start' | 'stop'; message: string }

export interface Api {
  listMeetings(): Promise<MeetingSummary[]>;
  getMeeting(id: string): Promise<Meeting>;
  getTranscript(id: string): Promise<TranscriptLine[]>;
  startMeeting(): Promise<Meeting>;
  stopMeeting(): Promise<Meeting>;
  saveNotes(id: string, notes: string): Promise<void>;
  renameMeeting(id: string, title: string): Promise<MeetingSummary>;
  showInFinder(id: string): Promise<void>;
  getRecordingState(): Promise<RecordingState>;
  onRecordingState(cb: (s: RecordingState) => void): () => void;
  onRecordingError(cb: (e: RecordingError) => void): () => void;
  onOpenMeeting(cb: (meetingId: string) => void): () => void;
  listModels(): Promise<ModelInfo[]>;
  downloadModel(id: string): Promise<void>;
  cancelDownload(id: string): Promise<void>;
  deleteModel(id: string): Promise<void>;
  onDownloadProgress(cb: (p: DownloadProgress) => void): () => void;
  getSettings(): Promise<Settings>;
  updateSettings(patch: Partial<Settings>): Promise<Settings>;
  getPermissions(): Promise<Permissions>;
  requestPermission(kind: 'microphone' | 'systemAudio'): Promise<Permission>;
}
