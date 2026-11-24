// In-memory implementation of the API contract, for the browser dev server
// and for tests. See README.md ("The mock API") for how to drive it.
//
// Choices the contract leaves open, which the real backend (issue 021) should
// either match or correct here:
// - `downloadModel` resolves once the download has started. Progress and the
//   outcome (done, cancelled, error) arrive through `onDownloadProgress`.
// - `stopMeeting` resolves once the meeting is complete. In between, the
//   recording state is `stopping`.
// - Installing a model does not change `defaultModelId`; deleting the default
//   model sets it to null. `startMeeting` uses the default model, or any
//   installed one when no default is set.
// - The recording state is emitted on every change and once a second while
//   recording. A start first sends an idle state with `starting` set, as the
//   backend does while the model loads; `mock.startFromOutside` keeps it
//   there for a while.

import {
  DEFAULT_INSTALLED_MODEL_ID,
  defaultMeetingTitle,
  meetingFolderName,
  MOCK_MEETINGS_ROOT,
  MOCK_MODEL_CATALOG,
  MOCK_RETIRED_MODEL_IDS,
  type MockMeetingSeed,
  scriptedLine,
  seedMeetings,
  seedMeetingsWithLongTranscript,
} from "./mockSeed";
import type {
  Api,
  DownloadProgress,
  Meeting,
  MeetingSummary,
  Microphone,
  ModelInfo,
  Permission,
  Permissions,
  RecordingError,
  RecordingState,
  Settings,
  TranscriptLine,
} from "./types";

export type PermissionKind = keyof Permissions;

/** The input devices the mock starts with: the built-in one is the default. */
export const MOCK_MICROPHONES: readonly Microphone[] = [
  {
    id: "coreaudio:BuiltInMicrophoneDevice",
    name: "MacBook Pro Microphone",
    isDefault: true,
  },
  {
    id: "coreaudio:AppleUSBAudioEngine:Shure:MV7:1",
    name: "Shure MV7",
    isDefault: false,
  },
  {
    id: "coreaudio:AppleUSBAudioEngine:Logitech:StreamCam:2",
    name: "Logitech StreamCam",
    isDefault: false,
  },
];

export interface MockOptions {
  /** The clock: seed dates are relative to it and new meetings start at it. Default: the real time. */
  now?: () => Date;
  /** Meetings to start with, or a function of the clock's current time. Default: `seedMeetings`. */
  meetings?: MockMeetingSeed[] | ((now: Date) => MockMeetingSeed[]);
  /** Ids of installed models. Default: one model. Pass `[]` for "none installed". */
  installedModelIds?: string[];
  /** Merged over `{ defaultModelId: <first installed or null>, language: "auto", globalShortcut: null, spellcheck: false, microphoneId: null }`. */
  settings?: Partial<Settings>;
  /** The connected input devices. Default: `MOCK_MICROPHONES`. */
  microphones?: Microphone[];
  /** Merged over both permissions granted. */
  permissions?: Partial<Permissions>;
  /** What the simulated system prompt answers for an undetermined permission. Default: granted. */
  permissionResponses?: Partial<Record<PermissionKind, Permission>>;
  /** Delay before every call settles. Default 0. */
  latencyMs?: number;
  /** Real milliseconds per second of recording time. Default 1000. */
  tickMs?: number;
  /** A transcript line is added every this many seconds of recording. Default 5. */
  secondsPerTranscriptLine?: number;
  /** How long `stopping` lasts. 0 completes the stop without a timer. Default 600. */
  stopDelayMs?: number;
  /**
   * A download takes `steps` progress events `stepMs` apart, then `verifyMs`
   * of verifying. Default 20, 150, 600. `cancelMs` is how long after
   * `cancelDownload` returns the `cancelled` event arrives: 0 (the default)
   * sends it before the call returns; the real backend takes a moment, and up
   * to about 35 s while a connection is still being set up.
   */
  download?: {
    steps?: number;
    stepMs?: number;
    verifyMs?: number;
    cancelMs?: number;
  };
}

/** Test and dev controls. None of this exists on the real API. */
export interface MockControls {
  /** Stops every timer and replaces all state with a fresh seed built from `options`. Listeners stay subscribed and receive the idle recording state. */
  reset(options?: MockOptions): void;
  /** The "none installed" versus "one installed" switch: sets exactly which models are installed. */
  setInstalledModels(ids: string[]): void;
  setPermission(kind: PermissionKind, value: Permission): void;
  /** Replaces the connected input devices, as plugging one in or out does. */
  setMicrophones(microphones: Microphone[]): void;
  /** Adds a meeting, or replaces the one with the same id. */
  putMeeting(meeting: MockMeetingSeed): void;
  /** Makes the next call to `method` fail with `message`. Subscriptions (`on…`) cannot fail. */
  failNext(method: keyof Api, message?: string): void;
  /** Makes the next download fail halfway with an `error` progress event. */
  failNextDownload(message?: string): void;
  /**
   * Emits a recording error, as the backend does when a start or stop asked
   * for from the menu bar or the global shortcut fails. Nothing else changes.
   */
  emitRecordingError(action: RecordingError["action"], message?: string): void;
  /**
   * Asks the window to show a meeting, as the backend does when Open Mneme is
   * chosen from the menu bar during a recording. Default: the live meeting;
   * with none, nothing is sent.
   */
  emitOpenMeeting(meetingId?: string): void;
  /**
   * Starts a meeting as the menu bar or the global shortcut does: a
   * `starting` state, the model load (`loadMs`, default 1500), then the
   * recording. The window's own `startMeeting` is refused meanwhile. A start
   * refused at once (no model, microphone denied) only emits a recording
   * error; a `failNext("startMeeting")` fails it after the load, back to idle
   * and with a recording error. Resolves when the start has settled.
   */
  startFromOutside(loadMs?: number): Promise<void>;
  /** Ids passed to `showInFinder` since the last reset, oldest first. */
  readonly revealedInFinder: readonly string[];
}

export interface MockApi extends Api {
  readonly mock: MockControls;
}

/** Ready-made options for common starting points. */
export const mockScenarios = {
  /** A returning user: one model installed, permissions granted, four meetings. */
  default: {},
  /** A fresh install: no model, no meetings, permissions not asked yet. */
  "first-run": {
    installedModelIds: [],
    meetings: [],
    permissions: { microphone: "undetermined", systemAudio: "undetermined" },
  },
  /** Set up, but no meetings yet. */
  empty: { meetings: [] },
  /** The default seed plus a meeting with a 2,000-line transcript. */
  "long-transcript": { meetings: (now) => seedMeetingsWithLongTranscript(now) },
} satisfies Record<string, MockOptions>;

export type MockScenario = keyof typeof mockScenarios;

/**
 * Reads mock options from a URL query string, so the browser dev server can
 * start in any scenario: `?mock=first-run`, `?mock=empty`,
 * `?mock=long-transcript`, `&mockLatency=300` to slow every call down, and
 * `&mockCancelDelay=3000` to make a cancelled download take that long to stop.
 */
export function mockOptionsFromSearch(search: string): MockOptions {
  const params = new URLSearchParams(search);
  const scenario = params.get("mock");
  const options: MockOptions =
    scenario !== null && Object.hasOwn(mockScenarios, scenario)
      ? { ...mockScenarios[scenario as MockScenario] }
      : {};
  const latency = Number(params.get("mockLatency"));
  if (Number.isFinite(latency) && latency > 0) {
    options.latencyMs = latency;
  }
  const cancelDelay = Number(params.get("mockCancelDelay"));
  if (Number.isFinite(cancelDelay) && cancelDelay > 0) {
    options.download = { ...options.download, cancelMs: cancelDelay };
  }
  return options;
}

interface MeetingRecord {
  id: string;
  title: string;
  titleEdited: boolean;
  startedAt: string;
  durationSecs: number | null;
  status: Meeting["status"];
  notes: string;
  transcript: TranscriptLine[];
  /** Kept apart from the title because a rename while recording is applied at stop. */
  folderName: string;
}

interface ActiveDownload {
  receivedBytes: number;
  totalBytes: number;
  stepBytes: number;
  failWith: string | null;
  /** True from `cancelDownload` until the `cancelled` event has been sent. */
  cancelling: boolean;
  timer: ReturnType<typeof setTimeout> | null;
}

const IDLE: RecordingState = {
  status: "idle",
  meetingId: null,
  elapsedSecs: 0,
  modelId: null,
  warning: null,
  starting: false,
};

/** What the backend says when system audio is known to be refused at the start. */
export const MOCK_MIC_ONLY_WARNING =
  "System audio access is denied, so this meeting records the microphone only. Allow Mneme in System Settings > Privacy & Security > Screen & System Audio Recording, under System Audio Recording Only, then quit and reopen Mneme.";

const sleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

export function createMockApi(initialOptions: MockOptions = {}): MockApi {
  // Configuration, set by `init`.
  let now: () => Date;
  let latencyMs: number;
  let tickMs: number;
  let secondsPerTranscriptLine: number;
  let stopDelayMs: number;
  let downloadSteps: number;
  let downloadStepMs: number;
  let downloadVerifyMs: number;
  let downloadCancelMs: number;
  let permissionResponses: Record<PermissionKind, Permission>;

  // State, replaced by `init`.
  let meetings: Map<string, MeetingRecord>;
  let installed: Set<string>;
  let settings: Settings;
  let permissions: Permissions;
  let recording: RecordingState;
  let recordingModelId: string | null;
  let downloads = new Map<string, ActiveDownload>();
  let failures: Map<keyof Api, string>;
  let nextDownloadFailure: string | null;
  let revealedInFinder: string[];
  let microphones: Microphone[];
  let meetingCounter: number;
  /** Counts resets, so a start still loading knows it was cancelled. */
  let resets = 0;
  let tickTimer: ReturnType<typeof setInterval> | null = null;
  let stopTimer: ReturnType<typeof setTimeout> | null = null;

  // Listeners survive `reset`.
  const recordingListeners = new Set<(s: RecordingState) => void>();
  const recordingErrorListeners = new Set<(e: RecordingError) => void>();
  const openMeetingListeners = new Set<(meetingId: string) => void>();
  const downloadListeners = new Set<(p: DownloadProgress) => void>();

  function toRecord(seed: MockMeetingSeed): MeetingRecord {
    const titleEdited = seed.titleEdited ?? true;
    return {
      id: seed.id,
      title: seed.title,
      titleEdited,
      startedAt: seed.startedAt,
      durationSecs: seed.durationSecs,
      status: seed.status,
      notes: seed.notes,
      transcript: seed.transcript.map((line) => ({ ...line })),
      folderName: meetingFolderName(seed.startedAt, seed.title, titleEdited),
    };
  }

  function stopTimers() {
    if (tickTimer !== null) clearInterval(tickTimer);
    if (stopTimer !== null) clearTimeout(stopTimer);
    tickTimer = null;
    stopTimer = null;
    for (const download of downloads.values()) {
      if (download.timer !== null) clearTimeout(download.timer);
    }
  }

  function init(options: MockOptions) {
    stopTimers();
    resets += 1;

    now = options.now ?? (() => new Date());
    latencyMs = options.latencyMs ?? 0;
    tickMs = options.tickMs ?? 1000;
    secondsPerTranscriptLine = options.secondsPerTranscriptLine ?? 5;
    stopDelayMs = options.stopDelayMs ?? 600;
    downloadSteps = Math.max(1, options.download?.steps ?? 20);
    downloadStepMs = options.download?.stepMs ?? 150;
    downloadVerifyMs = options.download?.verifyMs ?? 600;
    downloadCancelMs = Math.max(0, options.download?.cancelMs ?? 0);
    permissionResponses = {
      microphone: "granted",
      systemAudio: "granted",
      ...options.permissionResponses,
    };

    const seeds =
      typeof options.meetings === "function"
        ? options.meetings(now())
        : (options.meetings ?? seedMeetings(now()));
    meetings = new Map(seeds.map((seed) => [seed.id, toRecord(seed)]));
    installed = new Set(
      options.installedModelIds ?? [DEFAULT_INSTALLED_MODEL_ID],
    );
    settings = {
      defaultModelId: firstInstalledModelId(),
      language: "auto",
      globalShortcut: null,
      spellcheck: false,
      microphoneId: null,
      ...options.settings,
    };
    microphones = (options.microphones ?? MOCK_MICROPHONES).map((m) => ({
      ...m,
    }));
    permissions = {
      microphone: "granted",
      systemAudio: "granted",
      ...options.permissions,
    };
    recording = { ...IDLE };
    recordingModelId = null;
    downloads = new Map();
    failures = new Map();
    nextDownloadFailure = null;
    revealedInFinder = [];
    meetingCounter = 0;
  }

  function firstInstalledModelId(): string | null {
    return (
      MOCK_MODEL_CATALOG.find((model) => installed.has(model.id))?.id ?? null
    );
  }

  /** Runs one API call: the optional delay, then a failure queued by `failNext`, then the call. */
  async function call<T>(
    method: keyof Api,
    run: () => T | Promise<T>,
  ): Promise<T> {
    if (latencyMs > 0) await sleep(latencyMs);
    const failure = failures.get(method);
    if (failure !== undefined) {
      failures.delete(method);
      throw new Error(failure);
    }
    return run();
  }

  function record(id: string): MeetingRecord {
    const meeting = meetings.get(id);
    if (!meeting) throw new Error(`Meeting not found: ${id}`);
    return meeting;
  }

  function toSummary(meeting: MeetingRecord): MeetingSummary {
    return {
      id: meeting.id,
      title: meeting.title,
      startedAt: meeting.startedAt,
      durationSecs: meeting.durationSecs,
      status: meeting.status,
    };
  }

  function toMeeting(meeting: MeetingRecord): Meeting {
    return {
      ...toSummary(meeting),
      notes: meeting.notes,
      transcriptLineCount: meeting.transcript.length,
      folderPath: `${MOCK_MEETINGS_ROOT}/${meeting.folderName}`,
    };
  }

  function emitRecording() {
    for (const listener of [...recordingListeners]) listener({ ...recording });
  }

  function emitDownload(progress: DownloadProgress) {
    for (const listener of [...downloadListeners]) listener(progress);
  }

  function resolvePermission(kind: PermissionKind): Permission {
    if (permissions[kind] === "undetermined") {
      permissions[kind] = permissionResponses[kind];
    }
    return permissions[kind];
  }

  function tick() {
    const meeting = meetings.get(recording.meetingId ?? "");
    if (!meeting) return;
    recording.elapsedSecs += 1;
    if (recording.elapsedSecs % secondsPerTranscriptLine === 0) {
      meeting.transcript.push(
        scriptedLine(meeting.transcript.length, recording.elapsedSecs),
      );
    }
    emitRecording();
  }

  /** The checks of a start, then the `starting` state (not emitted yet). Returns the model to record with. */
  function beginStart(): string {
    if (recording.status !== "idle" || recording.starting) {
      throw new Error("A meeting is already being recorded.");
    }
    const modelId =
      settings.defaultModelId !== null && installed.has(settings.defaultModelId)
        ? settings.defaultModelId
        : firstInstalledModelId();
    if (modelId === null) {
      throw new Error(
        "No transcription model is installed. Download one in Settings.",
      );
    }
    if (resolvePermission("microphone") === "denied") {
      throw new Error(
        "Microphone access is denied. Allow Mneme in System Settings > Privacy & Security > Microphone.",
      );
    }
    recording = { ...IDLE, modelId, starting: true };
    recordingModelId = modelId;
    return modelId;
  }

  /** The model has loaded: creates the meeting and starts recording it. */
  function finishStart(modelId: string): Meeting {
    // A denied system audio permission still records, with the microphone only.
    const micOnly = resolvePermission("systemAudio") === "denied";

    const start = now();
    meetingCounter += 1;
    const meeting = toRecord({
      id: `mtg-new-${meetingCounter}`,
      title: defaultMeetingTitle(start),
      titleEdited: false,
      startedAt: start.toISOString(),
      durationSecs: null,
      status: "recording",
      notes: "",
      transcript: [],
    });
    meetings.set(meeting.id, meeting);
    recording = {
      status: "recording",
      meetingId: meeting.id,
      elapsedSecs: 0,
      modelId,
      warning: micOnly ? MOCK_MIC_ONLY_WARNING : null,
      starting: false,
    };
    recordingModelId = modelId;
    tickTimer = setInterval(tick, tickMs);
    emitRecording();
    return toMeeting(meeting);
  }

  function finishStop(meeting: MeetingRecord): Meeting {
    stopTimer = null;
    meeting.status = "complete";
    meeting.durationSecs = recording.elapsedSecs;
    // A rename made while recording reaches the folder name now.
    meeting.folderName = meetingFolderName(
      meeting.startedAt,
      meeting.title,
      meeting.titleEdited,
    );
    recording = { ...IDLE };
    recordingModelId = null;
    emitRecording();
    return toMeeting(meeting);
  }

  function stepDownload(modelId: string) {
    const download = downloads.get(modelId);
    if (!download) return;
    download.timer = null;
    download.receivedBytes = Math.min(
      download.totalBytes,
      download.receivedBytes + download.stepBytes,
    );
    const { receivedBytes, totalBytes } = download;

    if (download.failWith !== null && receivedBytes * 2 >= totalBytes) {
      downloads.delete(modelId);
      emitDownload({
        modelId,
        receivedBytes,
        totalBytes,
        state: "error",
        error: download.failWith,
      });
      return;
    }

    if (receivedBytes < totalBytes) {
      download.timer = setTimeout(() => stepDownload(modelId), downloadStepMs);
      emitDownload({
        modelId,
        receivedBytes,
        totalBytes,
        state: "downloading",
      });
      return;
    }

    download.timer = setTimeout(() => {
      downloads.delete(modelId);
      installed.add(modelId);
      emitDownload({ modelId, receivedBytes, totalBytes, state: "done" });
    }, downloadVerifyMs);
    emitDownload({ modelId, receivedBytes, totalBytes, state: "verifying" });
  }

  const api: Api = {
    listMeetings: () =>
      call("listMeetings", () =>
        [...meetings.values()]
          .sort((a, b) => Date.parse(b.startedAt) - Date.parse(a.startedAt))
          .map(toSummary),
      ),

    getMeeting: (id) => call("getMeeting", () => toMeeting(record(id))),

    getTranscript: (id) =>
      call("getTranscript", () =>
        record(id).transcript.map((line) => ({ ...line })),
      ),

    startMeeting: () =>
      call("startMeeting", () => {
        const modelId = beginStart();
        emitRecording();
        return finishStart(modelId);
      }),

    stopMeeting: () =>
      call("stopMeeting", () => {
        if (recording.status !== "recording" || recording.meetingId === null) {
          throw new Error("No meeting is being recorded.");
        }
        const meeting = record(recording.meetingId);
        if (tickTimer !== null) clearInterval(tickTimer);
        tickTimer = null;
        recording = { ...recording, status: "stopping" };
        emitRecording();

        if (stopDelayMs <= 0) return finishStop(meeting);
        return new Promise<Meeting>((resolve) => {
          stopTimer = setTimeout(
            () => resolve(finishStop(meeting)),
            stopDelayMs,
          );
        });
      }),

    saveNotes: (id, notes) =>
      call("saveNotes", () => {
        record(id).notes = notes;
      }),

    renameMeeting: (id, title) =>
      call("renameMeeting", () => {
        const meeting = record(id);
        const trimmed = title.trim();
        if (trimmed === "") throw new Error("The title cannot be empty.");

        const folderName = meetingFolderName(meeting.startedAt, trimmed, true);
        const clash = [...meetings.values()].some(
          (other) => other.id !== id && other.folderName === folderName,
        );
        if (clash) {
          throw new Error(
            `A meeting folder named "${folderName}" already exists.`,
          );
        }
        meeting.title = trimmed;
        meeting.titleEdited = true;
        // The folder is never renamed while recording; `stopMeeting` applies it.
        if (meeting.status !== "recording") meeting.folderName = folderName;
        return toSummary(meeting);
      }),

    showInFinder: (id) =>
      call("showInFinder", () => {
        revealedInFinder.push(record(id).id);
      }),

    deleteMeeting: (id) =>
      call("deleteMeeting", () => {
        record(id);
        if (recording.status !== "idle" && recording.meetingId === id) {
          throw new Error(
            "This meeting is being recorded. Stop the recording before deleting it.",
          );
        }
        meetings.delete(id);
      }),

    getRecordingState: () =>
      call("getRecordingState", () => ({ ...recording })),

    onRecordingState: (cb) => {
      recordingListeners.add(cb);
      return () => {
        recordingListeners.delete(cb);
      };
    },

    // Only `mock.emitRecordingError` sends these: a start or stop made
    // through this API reports its failure by rejecting.
    onRecordingError: (cb) => {
      recordingErrorListeners.add(cb);
      return () => {
        recordingErrorListeners.delete(cb);
      };
    },

    // Only `mock.emitOpenMeeting` sends these.
    onOpenMeeting: (cb) => {
      openMeetingListeners.add(cb);
      return () => {
        openMeetingListeners.delete(cb);
      };
    },

    listModels: () =>
      call("listModels", () =>
        // A retired model is listed only while it is installed.
        MOCK_MODEL_CATALOG.filter(
          (model) =>
            installed.has(model.id) || !MOCK_RETIRED_MODEL_IDS.has(model.id),
        ).map((model): ModelInfo => ({
          ...model,
          installed: installed.has(model.id),
        })),
      ),

    downloadModel: (id) =>
      call("downloadModel", () => {
        const model = MOCK_MODEL_CATALOG.find((m) => m.id === id);
        if (!model) throw new Error(`Unknown model: ${id}`);
        if (installed.has(id)) {
          throw new Error(`${model.name} is already installed.`);
        }
        if (downloads.has(id)) {
          throw new Error(`${model.name} is already downloading.`);
        }
        const download: ActiveDownload = {
          receivedBytes: 0,
          totalBytes: model.sizeBytes,
          stepBytes: Math.ceil(model.sizeBytes / downloadSteps),
          failWith: nextDownloadFailure,
          cancelling: false,
          timer: setTimeout(() => stepDownload(id), downloadStepMs),
        };
        nextDownloadFailure = null;
        downloads.set(id, download);
        emitDownload({
          modelId: id,
          receivedBytes: 0,
          totalBytes: model.sizeBytes,
          state: "downloading",
        });
      }),

    cancelDownload: (id) =>
      call("cancelDownload", () => {
        const download = downloads.get(id);
        if (!download || download.cancelling) return;
        if (download.timer !== null) clearTimeout(download.timer);
        const finish = () => {
          downloads.delete(id);
          emitDownload({
            modelId: id,
            receivedBytes: download.receivedBytes,
            totalBytes: download.totalBytes,
            state: "cancelled",
          });
        };
        if (downloadCancelMs <= 0) {
          finish();
          return;
        }
        // Like the real backend: the call returns at once, the download stops
        // a little later, and until then the same model cannot be started.
        download.cancelling = true;
        download.timer = setTimeout(finish, downloadCancelMs);
      }),

    deleteModel: (id) =>
      call("deleteModel", () => {
        const model = MOCK_MODEL_CATALOG.find((m) => m.id === id);
        if (!model) throw new Error(`Unknown model: ${id}`);
        if (!installed.has(id)) {
          throw new Error(`${model.name} is not installed.`);
        }
        if (recordingModelId === id) {
          throw new Error(
            `${model.name} is in use by the current recording and cannot be deleted.`,
          );
        }
        installed.delete(id);
        if (settings.defaultModelId === id) {
          settings = { ...settings, defaultModelId: null };
        }
      }),

    onDownloadProgress: (cb) => {
      downloadListeners.add(cb);
      return () => {
        downloadListeners.delete(cb);
      };
    },

    getSettings: () => call("getSettings", () => ({ ...settings })),

    updateSettings: (patch) =>
      call("updateSettings", () => {
        const next = { ...settings, ...patch };
        if (
          next.defaultModelId !== null &&
          !installed.has(next.defaultModelId)
        ) {
          throw new Error(
            `Model ${next.defaultModelId} is not installed, so it cannot be the default.`,
          );
        }
        settings = next;
        return { ...settings };
      }),

    listMicrophones: () =>
      call("listMicrophones", () => microphones.map((m) => ({ ...m }))),

    getPermissions: () => call("getPermissions", () => ({ ...permissions })),

    // An undetermined permission gets the simulated prompt's answer. A denied
    // one stays denied: the real app can only send the user to System Settings.
    requestPermission: (kind) =>
      call("requestPermission", () => resolvePermission(kind)),
  };

  const mock: MockControls = {
    reset(options = {}) {
      init(options);
      emitRecording();
    },
    setInstalledModels(ids) {
      installed = new Set(ids);
      if (
        settings.defaultModelId !== null &&
        !installed.has(settings.defaultModelId)
      ) {
        settings = { ...settings, defaultModelId: null };
      }
    },
    setPermission(kind, value) {
      permissions = { ...permissions, [kind]: value };
    },
    setMicrophones(list) {
      microphones = list.map((m) => ({ ...m }));
    },
    putMeeting(meeting) {
      meetings.set(meeting.id, toRecord(meeting));
    },
    failNext(method, message = `Mock failure in ${method}`) {
      failures.set(method, message);
    },
    failNextDownload(message = "The download failed. Check your connection.") {
      nextDownloadFailure = message;
    },
    emitRecordingError(
      action,
      message = action === "start"
        ? "Mneme is not allowed to use the microphone."
        : "The recording could not be closed.",
    ) {
      for (const listener of [...recordingErrorListeners]) {
        listener({ action, message });
      }
    },
    emitOpenMeeting(meetingId = recording.meetingId ?? undefined) {
      if (meetingId === undefined) return;
      for (const listener of [...openMeetingListeners]) listener(meetingId);
    },
    async startFromOutside(loadMs = 1500) {
      const refuse = (message: string) => {
        for (const listener of [...recordingErrorListeners]) {
          listener({ action: "start", message });
        }
      };
      let modelId: string;
      try {
        modelId = beginStart();
      } catch (error) {
        refuse(error instanceof Error ? error.message : String(error));
        return;
      }
      emitRecording();
      const startedIn = resets;
      await sleep(loadMs);
      // A reset while the model loaded ends this start.
      if (startedIn !== resets) return;
      const failure = failures.get("startMeeting");
      if (failure !== undefined) {
        failures.delete("startMeeting");
        recording = { ...IDLE };
        recordingModelId = null;
        emitRecording();
        refuse(failure);
        return;
      }
      finishStart(modelId);
    },
    get revealedInFinder() {
      return [...revealedInFinder];
    },
  };

  init(initialOptions);
  return { ...api, mock };
}

/**
 * The instance the app uses outside Tauri. Tests can drive it through
 * `mockApi.mock`; the test setup resets it after every test.
 */
export const mockApi: MockApi = createMockApi(
  typeof window === "undefined"
    ? {}
    : mockOptionsFromSearch(window.location.search),
);
