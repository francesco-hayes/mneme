import { beforeEach, expect, test, vi } from "vitest";
import { api, chooseApi } from "./index";
import { mockApi } from "./mock";
import { tauriApi } from "./tauri";
import type { Api } from "./types";

const { invoke, listen } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));
vi.mock(import("@tauri-apps/api/core"), async (importOriginal) => ({
  ...(await importOriginal()),
  invoke,
}));
vi.mock(import("@tauri-apps/api/event"), async (importOriginal) => ({
  ...(await importOriginal()),
  listen,
}));

beforeEach(() => {
  invoke.mockReset();
  listen.mockReset();
});

test("uses the mock outside Tauri and the Tauri implementation inside it", () => {
  expect(chooseApi(undefined, false)).toBe(mockApi);
  expect(chooseApi(undefined, true)).toBe(tauriApi);
});

test("VITE_MNEME_API overrides the choice", () => {
  expect(chooseApi("mock", true)).toBe(mockApi);
  expect(chooseApi("tauri", false)).toBe(tauriApi);
  expect(chooseApi("something else", false)).toBe(mockApi);
});

test("tests run against the mock", () => {
  expect(api).toBe(mockApi);
});

// The setup file has already loaded ./tauri with the real Tauri modules, so
// these tests load a fresh copy that picks up the mocks above.
async function loadTauriApi(): Promise<Api> {
  vi.resetModules();
  return (await import("./tauri")).tauriApi;
}

test("every method of the Tauri implementation calls its backend command", async () => {
  const tauriApi = await loadTauriApi();
  const methods = Object.keys(tauriApi) as (keyof Api)[];
  expect(methods).toHaveLength(21);

  invoke.mockResolvedValue("result");
  const calls: [Promise<unknown>, string, Record<string, unknown>?][] = [
    [tauriApi.listMeetings(), "list_meetings"],
    [tauriApi.getMeeting("m1"), "get_meeting", { id: "m1" }],
    [tauriApi.getTranscript("m1"), "get_transcript", { id: "m1" }],
    [tauriApi.startMeeting(), "start_meeting"],
    [tauriApi.stopMeeting(), "stop_meeting"],
    [tauriApi.saveNotes("m1", "n"), "save_notes", { id: "m1", notes: "n" }],
    [
      tauriApi.renameMeeting("m1", "T"),
      "rename_meeting",
      { id: "m1", title: "T" },
    ],
    [tauriApi.showInFinder("m1"), "show_in_finder", { id: "m1" }],
    [tauriApi.getRecordingState(), "get_recording_state"],
    [tauriApi.listModels(), "list_models"],
    [tauriApi.downloadModel("base"), "download_model", { id: "base" }],
    [tauriApi.cancelDownload("base"), "cancel_download", { id: "base" }],
    [tauriApi.deleteModel("base"), "delete_model", { id: "base" }],
    [tauriApi.getSettings(), "get_settings"],
    [
      tauriApi.updateSettings({ defaultModelId: null }),
      "update_settings",
      { patch: { defaultModelId: null } },
    ],
    [tauriApi.getPermissions(), "get_permissions"],
    [
      tauriApi.requestPermission("systemAudio"),
      "request_permission",
      { kind: "systemAudio" },
    ],
  ];
  // The four subscriptions are the only methods that are not commands.
  expect(calls).toHaveLength(methods.length - 4);
  for (const [result, command, args] of calls) {
    await expect(result, command).resolves.toBe("result");
    expect(invoke, command).toHaveBeenCalledWith(command, args);
  }
});

test("a backend refusal reaches the UI as an Error with the backend's sentence", async () => {
  const tauriApi = await loadTauriApi();
  invoke.mockRejectedValue("A meeting is already being recorded.");
  const attempt = tauriApi.startMeeting();
  await expect(attempt).rejects.toBeInstanceOf(Error);
  await expect(attempt).rejects.toThrow("A meeting is already being recorded.");
});

test("subscriptions deliver event payloads until they are cancelled", async () => {
  const tauriApi = await loadTauriApi();
  const unlisten = vi.fn();
  listen.mockResolvedValue(unlisten);
  const seen: unknown[] = [];

  const stop = tauriApi.onRecordingState((state) => seen.push(state));
  expect(listen).toHaveBeenCalledWith("recording-state", expect.any(Function));
  const handler = listen.mock.calls[0][1] as (e: { payload: unknown }) => void;
  const state = { status: "recording", meetingId: "m1", elapsedSecs: 3 };
  handler({ payload: state });
  expect(seen).toEqual([state]);

  await Promise.resolve();
  stop();
  expect(unlisten).toHaveBeenCalledTimes(1);
  handler({ payload: state });
  expect(seen).toHaveLength(1);
});

test("recording errors from the menu bar and the shortcut are subscribed to", async () => {
  const tauriApi = await loadTauriApi();
  const unlisten = vi.fn();
  listen.mockResolvedValue(unlisten);
  const seen: unknown[] = [];

  const stop = tauriApi.onRecordingError((error) => seen.push(error));
  expect(listen).toHaveBeenCalledTimes(1);
  expect(listen).toHaveBeenCalledWith("recording-error", expect.any(Function));
  const handler = listen.mock.calls[0][1] as (e: { payload: unknown }) => void;
  const error = { action: "start", message: "The microphone is not allowed." };
  handler({ payload: error });
  expect(seen).toEqual([error]);

  await Promise.resolve();
  stop();
  expect(unlisten).toHaveBeenCalledTimes(1);
  handler({ payload: error });
  expect(seen).toHaveLength(1);
});
