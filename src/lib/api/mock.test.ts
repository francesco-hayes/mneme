import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  createMockApi,
  MOCK_MIC_ONLY_WARNING,
  mockOptionsFromSearch,
  mockScenarios,
  type MockOptions,
} from "./mock";
import {
  generateTranscript,
  MOCK_MODEL_CATALOG,
  MOCK_RETIRED_MODEL_IDS,
} from "./mockSeed";
import type { DownloadProgress, RecordingError, RecordingState } from "./types";

const NOW = new Date(2026, 10, 12, 15, 0, 0); // Nov 12 2026, 3:00 PM local time

function create(options: MockOptions = {}) {
  return createMockApi({ now: () => NOW, ...options });
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("meetings", () => {
  test("the seed has meetings newest first, one of them interrupted", async () => {
    const api = create();
    const meetings = await api.listMeetings();

    expect(meetings.map((m) => m.id)).toEqual([
      "mtg-standup",
      "mtg-design-review",
      "mtg-untitled",
      "mtg-planning",
    ]);
    const starts = meetings.map((m) => Date.parse(m.startedAt));
    expect(starts).toEqual([...starts].sort((a, b) => b - a));
    expect(meetings.map((m) => m.status)).toEqual([
      "complete",
      "interrupted",
      "complete",
      "complete",
    ]);
  });

  test("a meeting carries notes, its transcript line count and a folder path", async () => {
    const api = create();

    const planning = await api.getMeeting("mtg-planning");
    expect(planning.transcriptLineCount).toBe(214);
    expect(planning.notes).toContain("# Quarterly planning");
    expect(planning.folderPath).toMatch(
      /\/Documents\/Mneme\/2026-10-27 1000 Quarterly planning$/,
    );

    const untitled = await api.getMeeting("mtg-untitled");
    expect(untitled.title).toBe("Meeting, Nov 9 1:00 PM");
    expect(untitled.transcriptLineCount).toBe(0);
    expect(untitled.folderPath).toMatch(/2026-11-09 1300 Meeting$/);
  });

  test("transcripts have lines for both speakers with hh:mm:ss timestamps", async () => {
    const api = create();
    const lines = await api.getTranscript("mtg-design-review");

    expect(lines).toHaveLength(8);
    expect(new Set(lines.map((line) => line.speaker))).toEqual(
      new Set(["Me", "Them"]),
    );
    for (const line of lines) {
      expect(line.t).toMatch(/^\d{2}:\d{2}:\d{2}$/);
      expect(line.text).not.toBe("");
    }
    expect(await api.getTranscript("mtg-untitled")).toEqual([]);
  });

  test("an unknown id is an error", async () => {
    const api = create();
    await expect(api.getMeeting("nope")).rejects.toThrow("Meeting not found");
    await expect(api.getTranscript("nope")).rejects.toThrow(
      "Meeting not found",
    );
    await expect(api.saveNotes("nope", "x")).rejects.toThrow(
      "Meeting not found",
    );
  });

  test("notes are saved exactly as given", async () => {
    const api = create();
    const notes = "  # Title\n\n- item\n\n";
    await api.saveNotes("mtg-standup", notes);
    expect((await api.getMeeting("mtg-standup")).notes).toBe(notes);
  });

  test("renaming changes the title and the folder, and refuses a clash or an empty title", async () => {
    const api = create();

    const renamed = await api.renameMeeting("mtg-untitled", " Budget ");
    expect(renamed.title).toBe("Budget");
    expect((await api.getMeeting("mtg-untitled")).folderPath).toMatch(
      /2026-11-09 1300 Budget$/,
    );

    await expect(api.renameMeeting("mtg-standup", "   ")).rejects.toThrow(
      "cannot be empty",
    );

    const standup = await api.getMeeting("mtg-standup");
    api.mock.putMeeting({
      id: "mtg-twin",
      title: "Retro",
      startedAt: standup.startedAt,
      durationSecs: 60,
      status: "complete",
      notes: "",
      transcript: [],
    });
    await expect(api.renameMeeting("mtg-standup", "Retro")).rejects.toThrow(
      "already exists",
    );
    expect((await api.getMeeting("mtg-standup")).title).toBe("Standup");
  });

  test("showInFinder records which meeting was revealed", async () => {
    const api = create();
    await api.showInFinder("mtg-standup");
    expect(api.mock.revealedInFinder).toEqual(["mtg-standup"]);
    await expect(api.showInFinder("nope")).rejects.toThrow("Meeting not found");
  });

  test("putMeeting adds a meeting with a long generated transcript", async () => {
    const api = create({ meetings: [] });
    api.mock.putMeeting({
      id: "mtg-long",
      title: "Long",
      startedAt: NOW.toISOString(),
      durationSecs: 10_000,
      status: "complete",
      notes: "",
      transcript: generateTranscript(2000),
    });

    const lines = await api.getTranscript("mtg-long");
    expect(lines).toHaveLength(2000);
    expect(lines.at(-1)?.t).toBe("02:46:40");
  });
});

describe("recording", () => {
  test("starting creates a recording meeting with a date-time title", async () => {
    const api = create();
    const meeting = await api.startMeeting();

    expect(meeting).toMatchObject({
      title: "Meeting, Nov 12 3:00 PM",
      startedAt: NOW.toISOString(),
      durationSecs: null,
      status: "recording",
      notes: "",
      transcriptLineCount: 0,
    });
    expect(meeting.folderPath).toMatch(/2026-11-12 1500 Meeting$/);
    expect(await api.getRecordingState()).toEqual({
      status: "recording",
      meetingId: meeting.id,
      elapsedSecs: 0,
      modelId: "base.en",
      warning: null,
      starting: false,
    });
    expect((await api.listMeetings())[0].id).toBe(meeting.id);
  });

  test("the timer ticks once a second and listeners hear every change", async () => {
    const api = create();
    const states: RecordingState[] = [];
    const unsubscribe = api.onRecordingState((s) => states.push(s));

    const meeting = await api.startMeeting();
    await vi.advanceTimersByTimeAsync(3000);

    // First the start, as the backend sends it while the model loads.
    const [loading, ...recording] = states;
    expect(loading).toEqual({
      status: "idle",
      meetingId: null,
      elapsedSecs: 0,
      modelId: "base.en",
      warning: null,
      starting: true,
    });
    expect(recording.map((s) => s.elapsedSecs)).toEqual([0, 1, 2, 3]);
    expect(recording.every((s) => s.status === "recording")).toBe(true);
    expect(recording.every((s) => s.meetingId === meeting.id)).toBe(true);
    expect(recording.every((s) => !s.starting)).toBe(true);
    expect((await api.getRecordingState()).elapsedSecs).toBe(3);

    unsubscribe();
    await vi.advanceTimersByTimeAsync(2000);
    expect(states).toHaveLength(5);
    expect((await api.getRecordingState()).elapsedSecs).toBe(5);
  });

  test("stopping goes through `stopping`, then completes the meeting with its duration and transcript", async () => {
    const api = create();
    const states: RecordingState[] = [];
    api.onRecordingState((s) => states.push(s));

    const { id } = await api.startMeeting();
    await vi.advanceTimersByTimeAsync(12_000);

    let stopped = false;
    const stopping = api.stopMeeting().then((meeting) => {
      stopped = true;
      return meeting;
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(await api.getRecordingState()).toEqual({
      status: "stopping",
      meetingId: id,
      elapsedSecs: 12,
      modelId: "base.en",
      warning: null,
      starting: false,
    });
    expect(stopped).toBe(false);

    await vi.advanceTimersByTimeAsync(600);
    const meeting = await stopping;
    expect(meeting).toMatchObject({
      id,
      status: "complete",
      durationSecs: 12,
      transcriptLineCount: 2,
    });
    expect(await api.getRecordingState()).toEqual({
      status: "idle",
      meetingId: null,
      elapsedSecs: 0,
      modelId: null,
      warning: null,
      starting: false,
    });
    expect(states.at(-2)?.status).toBe("stopping");
    expect(states.at(-1)?.status).toBe("idle");

    // The timer no longer runs.
    await vi.advanceTimersByTimeAsync(5000);
    expect((await api.getRecordingState()).elapsedSecs).toBe(0);
    expect(await api.getTranscript(id)).toEqual([
      { t: "00:00:05", speaker: "Them", text: "Can you hear me all right?" },
      { t: "00:00:10", speaker: "Me", text: "Yes, loud and clear." },
    ]);
  });

  test("a start from outside the window is `starting` while its model loads", async () => {
    const api = create();
    const states: RecordingState[] = [];
    const errors: RecordingError[] = [];
    api.onRecordingState((s) => states.push(s));
    api.onRecordingError((e) => errors.push(e));

    const started = api.mock.startFromOutside(1000);
    expect(await api.getRecordingState()).toMatchObject({
      status: "idle",
      meetingId: null,
      modelId: "base.en",
      starting: true,
    });
    // The window's own start is refused meanwhile, as the backend does.
    await expect(api.startMeeting()).rejects.toThrow(
      "A meeting is already being recorded.",
    );

    await vi.advanceTimersByTimeAsync(1000);
    await started;
    const state = await api.getRecordingState();
    expect(state).toMatchObject({ status: "recording", starting: false });
    expect(states.map((s) => s.starting)).toEqual([true, false]);
    expect(errors).toEqual([]);
    const stopped = api.stopMeeting();
    await vi.advanceTimersByTimeAsync(600);
    await stopped;

    // A start that fails during the load goes back to idle and says why.
    api.mock.failNext("startMeeting", "The model could not be loaded.");
    const failed = api.mock.startFromOutside(1000);
    await vi.advanceTimersByTimeAsync(1000);
    await failed;
    expect(await api.getRecordingState()).toEqual({
      status: "idle",
      meetingId: null,
      elapsedSecs: 0,
      modelId: null,
      warning: null,
      starting: false,
    });
    expect(errors).toEqual([
      { action: "start", message: "The model could not be loaded." },
    ]);

    // One refused at once sends no state, only the error.
    api.mock.setInstalledModels([]);
    const count = states.length;
    await api.mock.startFromOutside(1000);
    expect(states).toHaveLength(count);
    expect(errors.at(-1)?.message).toMatch(/^No transcription model/);
  });

  test("with stopDelayMs 0 a stop completes without waiting on a timer", async () => {
    const api = create({ stopDelayMs: 0 });
    await api.startMeeting();
    await vi.advanceTimersByTimeAsync(2000);

    const meeting = await api.stopMeeting();
    expect(meeting.status).toBe("complete");
    expect(meeting.durationSecs).toBe(2);
  });

  test("only one recording at a time, and stop needs a recording", async () => {
    const api = create();
    await expect(api.stopMeeting()).rejects.toThrow(
      "No meeting is being recorded",
    );
    await api.startMeeting();
    await expect(api.startMeeting()).rejects.toThrow("already being recorded");
  });

  test("starting is refused without an installed model or with the microphone denied", async () => {
    const noModel = create({ installedModelIds: [] });
    await expect(noModel.startMeeting()).rejects.toThrow(
      "No transcription model is installed",
    );

    const noMic = create({ permissions: { microphone: "denied" } });
    await expect(noMic.startMeeting()).rejects.toThrow(
      "Microphone access is denied",
    );
    expect((await noMic.getRecordingState()).status).toBe("idle");

    const noSystemAudio = create({ permissions: { systemAudio: "denied" } });
    expect((await noSystemAudio.startMeeting()).status).toBe("recording");
  });

  test("a rename while recording changes the title now and the folder at stop", async () => {
    const api = create({ stopDelayMs: 0 });
    const { id } = await api.startMeeting();

    await api.renameMeeting(id, "Kickoff");
    const during = await api.getMeeting(id);
    expect(during.title).toBe("Kickoff");
    expect(during.folderPath).toMatch(/2026-11-12 1500 Meeting$/);

    const after = await api.stopMeeting();
    expect(after.folderPath).toMatch(/2026-11-12 1500 Kickoff$/);
  });
});

describe("models and downloads", () => {
  test("the models offered are the real ones: six, smallest first, four English-only", async () => {
    const models = await create().listModels();

    expect(models.map((m) => m.id)).toEqual([
      "tiny.en",
      "base.en",
      "small.en",
      "medium.en",
      "large-v3-turbo",
      "large-v3",
    ]);
    const sizes = models.map((m) => m.sizeBytes);
    expect(sizes).toEqual([...sizes].sort((a, b) => a - b));
    expect(sizes[0]).toBe(77_704_715);
    expect(sizes.at(-1)).toBe(3_095_033_483);
    expect(models.filter((m) => m.englishOnly).map((m) => m.id)).toEqual([
      "tiny.en",
      "base.en",
      "small.en",
      "medium.en",
    ]);
    expect(models.every((m) => m.englishOnly === m.id.endsWith(".en"))).toBe(
      true,
    );
  });

  test("a retired model is listed only while it is installed", async () => {
    // The multilingual Tiny, Base, Small and Medium are no longer offered.
    const api = create({ installedModelIds: ["small", "base.en"] });
    const ids = async () => (await api.listModels()).map((m) => m.id);
    expect(MOCK_MODEL_CATALOG).toHaveLength(10);
    expect([...MOCK_RETIRED_MODEL_IDS]).toEqual([
      "tiny",
      "base",
      "small",
      "medium",
    ]);

    // One downloaded while it was offered is still there, in catalog order.
    expect(await ids()).toEqual([
      "tiny.en",
      "base.en",
      "small",
      "small.en",
      "medium.en",
      "large-v3-turbo",
      "large-v3",
    ]);
    await api.updateSettings({ defaultModelId: "small" });
    expect((await api.getSettings()).defaultModelId).toBe("small");

    // Deleting it takes it off the list.
    await api.deleteModel("small");
    expect(await ids()).not.toContain("small");
    expect(await ids()).toHaveLength(6);
  });

  test("the list has one model installed by default, and none on request", async () => {
    const one = await create().listModels();
    expect(one).toHaveLength(6);
    expect(one.filter((m) => m.installed).map((m) => m.id)).toEqual([
      "base.en",
    ]);
    expect(one.some((m) => m.englishOnly)).toBe(true);
    expect(one.some((m) => !m.englishOnly)).toBe(true);

    const none = create({ installedModelIds: [] });
    expect((await none.listModels()).some((m) => m.installed)).toBe(false);
    expect((await none.getSettings()).defaultModelId).toBeNull();
  });

  test("setInstalledModels switches between none and one installed", async () => {
    const api = create();

    api.mock.setInstalledModels([]);
    expect((await api.listModels()).some((m) => m.installed)).toBe(false);
    expect((await api.getSettings()).defaultModelId).toBeNull();

    api.mock.setInstalledModels(["small.en"]);
    const installed = (await api.listModels()).filter((m) => m.installed);
    expect(installed.map((m) => m.id)).toEqual(["small.en"]);
  });

  test("a download reports downloading, then verifying, then done, and installs the model", async () => {
    const api = create();
    const events: DownloadProgress[] = [];
    api.onDownloadProgress((p) => events.push(p));
    const total = MOCK_MODEL_CATALOG.find(
      (m) => m.id === "small.en",
    )!.sizeBytes;

    await api.downloadModel("small.en");
    expect(events).toEqual([
      {
        modelId: "small.en",
        receivedBytes: 0,
        totalBytes: total,
        state: "downloading",
      },
    ]);

    await vi.advanceTimersByTimeAsync(20 * 150);
    expect(events.at(-1)).toEqual({
      modelId: "small.en",
      receivedBytes: total,
      totalBytes: total,
      state: "verifying",
    });
    expect(events.slice(0, -1).every((e) => e.state === "downloading")).toBe(
      true,
    );
    const received = events.map((e) => e.receivedBytes);
    expect(received).toEqual([...received].sort((a, b) => a - b));
    expect(
      (await api.listModels()).find((m) => m.id === "small.en")?.installed,
    ).toBe(false);

    await vi.advanceTimersByTimeAsync(600);
    expect(events.at(-1)?.state).toBe("done");
    expect(events.filter((e) => e.state === "done")).toHaveLength(1);
    expect(
      (await api.listModels()).find((m) => m.id === "small.en")?.installed,
    ).toBe(true);
    // Installing does not change the default model.
    expect((await api.getSettings()).defaultModelId).toBe("base.en");
  });

  test("cancelling stops the download and installs nothing", async () => {
    const api = create();
    const events: DownloadProgress[] = [];
    api.onDownloadProgress((p) => events.push(p));

    await api.downloadModel("medium.en");
    await vi.advanceTimersByTimeAsync(5 * 150);
    await api.cancelDownload("medium.en");

    const last = events.at(-1)!;
    expect(last.state).toBe("cancelled");
    expect(last.receivedBytes).toBeGreaterThan(0);
    expect(last.receivedBytes).toBeLessThan(last.totalBytes);

    const count = events.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(events).toHaveLength(count);
    expect(
      (await api.listModels()).find((m) => m.id === "medium.en")?.installed,
    ).toBe(false);

    // Cancelling when nothing is downloading does nothing, and a retry works.
    await api.cancelDownload("medium.en");
    await api.downloadModel("medium.en");
    expect(events.at(-1)?.state).toBe("downloading");
  });

  test("with cancelMs, cancelled arrives that long after cancelDownload returns", async () => {
    const api = create({ download: { cancelMs: 3000 } });
    const events: DownloadProgress[] = [];
    api.onDownloadProgress((p) => events.push(p));

    await api.downloadModel("medium.en");
    await vi.advanceTimersByTimeAsync(5 * 150);
    const before = events.length;
    await api.cancelDownload("medium.en");

    // The call has returned; the download has stopped but has not said so.
    expect(events).toHaveLength(before);
    await vi.advanceTimersByTimeAsync(2999);
    expect(events).toHaveLength(before);
    await expect(api.downloadModel("medium.en")).rejects.toThrow(
      "already downloading",
    );
    // A second cancel changes nothing.
    await api.cancelDownload("medium.en");

    await vi.advanceTimersByTimeAsync(1);
    expect(events).toHaveLength(before + 1);
    expect(events.at(-1)).toMatchObject({
      modelId: "medium.en",
      state: "cancelled",
    });
    expect(
      (await api.listModels()).find((m) => m.id === "medium.en")?.installed,
    ).toBe(false);
    await api.downloadModel("medium.en");
    expect(events.at(-1)?.state).toBe("downloading");
  });

  test("failNextDownload ends the next download with an error event, and a retry succeeds", async () => {
    const api = create();
    const events: DownloadProgress[] = [];
    api.onDownloadProgress((p) => events.push(p));

    api.mock.failNextDownload("Connection lost.");
    await api.downloadModel("tiny.en");
    await vi.advanceTimersByTimeAsync(10_000);

    expect(events.at(-1)).toMatchObject({
      modelId: "tiny.en",
      state: "error",
      error: "Connection lost.",
    });
    expect(
      (await api.listModels()).find((m) => m.id === "tiny.en")?.installed,
    ).toBe(false);

    await api.downloadModel("tiny.en");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(events.at(-1)?.state).toBe("done");
    expect(
      (await api.listModels()).find((m) => m.id === "tiny.en")?.installed,
    ).toBe(true);
  });

  test("a download is refused for an unknown, installed or already downloading model", async () => {
    const api = create();
    await expect(api.downloadModel("nope")).rejects.toThrow("Unknown model");
    await expect(api.downloadModel("base.en")).rejects.toThrow(
      "already installed",
    );
    await api.downloadModel("small.en");
    await expect(api.downloadModel("small.en")).rejects.toThrow(
      "already downloading",
    );
  });

  test("two downloads run side by side", async () => {
    const api = create();
    const events: DownloadProgress[] = [];
    api.onDownloadProgress((p) => events.push(p));

    await api.downloadModel("small.en");
    await api.downloadModel("medium.en");
    await vi.advanceTimersByTimeAsync(10_000);

    const done = events.filter((e) => e.state === "done").map((e) => e.modelId);
    expect(done.sort()).toEqual(["medium.en", "small.en"]);
  });

  test("deleting removes the model, clears it as default, and is refused during its recording", async () => {
    const api = create({ installedModelIds: ["base.en", "small.en"] });
    await api.updateSettings({ defaultModelId: "small.en" });

    await api.startMeeting();
    await expect(api.deleteModel("small.en")).rejects.toThrow("in use");
    await api.deleteModel("base.en");

    await vi.advanceTimersByTimeAsync(1000);
    const stopping = api.stopMeeting();
    await vi.advanceTimersByTimeAsync(600);
    await stopping;

    await api.deleteModel("small.en");
    expect((await api.listModels()).some((m) => m.installed)).toBe(false);
    expect((await api.getSettings()).defaultModelId).toBeNull();
    await expect(api.deleteModel("small.en")).rejects.toThrow("not installed");
  });
});

describe("settings and permissions", () => {
  test("settings start with the installed model, automatic language and no shortcut", async () => {
    expect(await create().getSettings()).toEqual({
      defaultModelId: "base.en",
      language: "auto",
      globalShortcut: null,
      spellcheck: false,
    });
  });

  test("updateSettings merges a patch and refuses a default model that is not installed", async () => {
    const api = create();
    const updated = await api.updateSettings({
      language: "pt",
      globalShortcut: "CommandOrControl+Shift+R",
    });
    expect(updated).toEqual({
      defaultModelId: "base.en",
      language: "pt",
      globalShortcut: "CommandOrControl+Shift+R",
      spellcheck: false,
    });
    expect(await api.getSettings()).toEqual(updated);

    await expect(
      api.updateSettings({ defaultModelId: "medium.en" }),
    ).rejects.toThrow("not installed");
    expect((await api.getSettings()).defaultModelId).toBe("base.en");
  });

  test("an undetermined permission takes the prompt's answer; a denied one stays denied", async () => {
    const api = create({
      permissions: { microphone: "undetermined", systemAudio: "undetermined" },
      permissionResponses: { systemAudio: "denied" },
    });
    expect(await api.getPermissions()).toEqual({
      microphone: "undetermined",
      systemAudio: "undetermined",
    });

    expect(await api.requestPermission("microphone")).toBe("granted");
    expect(await api.requestPermission("systemAudio")).toBe("denied");
    expect(await api.requestPermission("systemAudio")).toBe("denied");
    expect(await api.getPermissions()).toEqual({
      microphone: "granted",
      systemAudio: "denied",
    });

    api.mock.setPermission("systemAudio", "granted");
    expect((await api.getPermissions()).systemAudio).toBe("granted");
  });
});

describe("controls", () => {
  test("failNext fails one call to that method, then it works again", async () => {
    const api = create();
    api.mock.failNext("saveNotes", "Disk full.");

    await expect(api.saveNotes("mtg-standup", "kept?")).rejects.toThrow(
      "Disk full.",
    );
    expect((await api.getMeeting("mtg-standup")).notes).not.toBe("kept?");
    await api.saveNotes("mtg-standup", "kept");
    expect((await api.getMeeting("mtg-standup")).notes).toBe("kept");
  });

  test("reset stops a recording and a download and restores the seed", async () => {
    const api = create();
    const states: RecordingState[] = [];
    const events: DownloadProgress[] = [];
    api.onRecordingState((s) => states.push(s));
    api.onDownloadProgress((p) => events.push(p));

    await api.startMeeting();
    await api.downloadModel("small.en");
    await api.saveNotes("mtg-standup", "changed");
    api.mock.reset({ now: () => NOW, installedModelIds: [] });

    expect(states.at(-1)).toEqual({
      status: "idle",
      meetingId: null,
      elapsedSecs: 0,
      modelId: null,
      warning: null,
      starting: false,
    });
    const eventCount = events.length;
    const stateCount = states.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(events).toHaveLength(eventCount);
    expect(states).toHaveLength(stateCount);

    expect(await api.listMeetings()).toHaveLength(4);
    expect((await api.getMeeting("mtg-standup")).notes).toContain("# Standup");
    expect((await api.listModels()).some((m) => m.installed)).toBe(false);
  });

  test("latencyMs delays every call", async () => {
    const api = create({ latencyMs: 200 });
    let settled = false;
    const pending = api.listMeetings().then(() => {
      settled = true;
    });

    await vi.advanceTimersByTimeAsync(199);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await pending;
    expect(settled).toBe(true);
  });

  test("returned data is a copy, so callers cannot change the mock's state", async () => {
    const api = create();
    const settings = await api.getSettings();
    settings.language = "xx";
    const lines = await api.getTranscript("mtg-design-review");
    lines[0].text = "changed";

    expect((await api.getSettings()).language).toBe("auto");
    expect((await api.getTranscript("mtg-design-review"))[0].text).not.toBe(
      "changed",
    );
  });
});

describe("scenarios", () => {
  test("first-run has no model, no meetings and permissions not asked yet", async () => {
    const api = create(mockScenarios["first-run"]);
    expect((await api.listModels()).some((m) => m.installed)).toBe(false);
    expect(await api.listMeetings()).toEqual([]);
    expect(await api.getPermissions()).toEqual({
      microphone: "undetermined",
      systemAudio: "undetermined",
    });
  });

  test("long-transcript adds a meeting with 2,000 lines", async () => {
    const api = create(mockScenarios["long-transcript"]);
    expect(await api.listMeetings()).toHaveLength(5);
    expect((await api.getMeeting("mtg-long")).transcriptLineCount).toBe(2000);
  });

  test("the query string picks a scenario and a latency", () => {
    expect(mockOptionsFromSearch("")).toEqual({});
    expect(mockOptionsFromSearch("?mock=first-run")).toEqual(
      mockScenarios["first-run"],
    );
    expect(mockOptionsFromSearch("?mock=empty&mockLatency=300")).toEqual({
      meetings: [],
      latencyMs: 300,
    });
    expect(mockOptionsFromSearch("?mock=toString&mockLatency=abc")).toEqual({});
    expect(mockOptionsFromSearch("?mockCancelDelay=3000")).toEqual({
      download: { cancelMs: 3000 },
    });
  });
});

describe("recording warnings and open requests", () => {
  test("a meeting with system audio denied records with a warning that lasts until stop", async () => {
    const api = create({ permissions: { systemAudio: "denied" } });
    const { id } = await api.startMeeting();
    const state = await api.getRecordingState();
    expect(state.warning).toBe(MOCK_MIC_ONLY_WARNING);
    expect(state.modelId).toBe("base.en");

    const stopping = api.stopMeeting();
    await vi.advanceTimersByTimeAsync(0);
    expect((await api.getRecordingState()).warning).toBe(MOCK_MIC_ONLY_WARNING);
    await vi.advanceTimersByTimeAsync(600);
    expect((await stopping).id).toBe(id);
    expect((await api.getRecordingState()).warning).toBeNull();
  });

  test("emitOpenMeeting sends the live meeting, and nothing when idle", async () => {
    const api = create();
    const opened: string[] = [];
    const unsubscribe = api.onOpenMeeting((meetingId) =>
      opened.push(meetingId),
    );

    api.mock.emitOpenMeeting();
    expect(opened).toEqual([]);

    const { id } = await api.startMeeting();
    api.mock.emitOpenMeeting();
    api.mock.emitOpenMeeting("mtg-standup");
    expect(opened).toEqual([id, "mtg-standup"]);

    unsubscribe();
    api.mock.emitOpenMeeting();
    expect(opened).toHaveLength(2);
  });
});
