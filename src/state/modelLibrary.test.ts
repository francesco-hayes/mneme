import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { createMockApi, type MockApi, type MockOptions } from "../lib/api/mock";
import { ModelLibraryStore } from "./modelLibrary";

// The mock's downloads run on timers: 20 steps of 150 ms, then 600 ms of
// verifying. `wait(ms)` moves that clock.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const wait = (ms = 0) => vi.advanceTimersByTimeAsync(ms);
const DOWNLOAD_MS = 20 * 150;
const VERIFY_MS = 600;

/** A store on its own mock, connected and loaded. */
async function setup(options: MockOptions = {}) {
  const api = createMockApi(options);
  const store = new ModelLibraryStore(api);
  const disconnect = store.connect();
  const loading = store.load();
  // With `latencyMs`, the mock answers on a timer.
  await wait(options.latencyMs ?? 0);
  await loading;
  return { api, store, disconnect };
}

const installedIds = (store: ModelLibraryStore) =>
  store
    .getState()
    .models.filter((model) => model.installed)
    .map((model) => model.id);
const defaultId = (store: ModelLibraryStore) =>
  store.getState().settings?.defaultModelId;
const download = (store: ModelLibraryStore, id: string) =>
  store.getState().downloads.get(id);
const savedDefault = async (api: MockApi) =>
  (await api.getSettings()).defaultModelId;

describe("loading", () => {
  test("starts idle and reads nothing until asked", async () => {
    const api = createMockApi();
    const listModels = vi.spyOn(api, "listModels");
    const store = new ModelLibraryStore(api);
    store.connect();
    await wait();

    expect(store.getState().status).toBe("idle");
    expect(listModels).not.toHaveBeenCalled();
  });

  test("load reads the models and the settings", async () => {
    const { store } = await setup();

    expect(store.getState().status).toBe("ready");
    expect(store.getState().models).toHaveLength(6);
    expect(installedIds(store)).toEqual(["base.en"]);
    expect(store.getState().settings).toEqual({
      defaultModelId: "base.en",
      language: "auto",
      globalShortcut: null,
    });
  });

  test("a failed read is an error with the reason, and loading again recovers", async () => {
    const api = createMockApi();
    const store = new ModelLibraryStore(api);
    api.mock.failNext("getSettings", "The settings file is locked.");
    await store.load();

    expect(store.getState().status).toBe("error");
    expect(store.getState().loadError).toBe("The settings file is locked.");

    await store.load();
    expect(store.getState().status).toBe("ready");
    expect(store.getState().loadError).toBeNull();
  });

  test("reading again keeps what is on screen until the new answer arrives", async () => {
    const { api, store } = await setup({ latencyMs: 100 });
    api.mock.setInstalledModels(["base.en", "small.en"]);

    const reading = store.load();
    expect(store.getState().status).toBe("ready");
    expect(installedIds(store)).toEqual(["base.en"]);
    await wait(100);
    await reading;
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("subscribers hear about every change", async () => {
    const { store } = await setup();
    const heard = vi.fn();
    const stop = store.subscribe(heard);

    await store.updateSettings({ globalShortcut: "Alt+F5" });
    expect(heard).toHaveBeenCalled();
    expect(store.getState().settings?.globalShortcut).toBe("Alt+F5");

    stop();
    heard.mockClear();
    await store.updateSettings({ globalShortcut: null });
    expect(heard).not.toHaveBeenCalled();
  });
});

describe("downloads", () => {
  test("a download goes from starting through downloading and verifying to installed", async () => {
    const { store } = await setup({ latencyMs: 50 });
    const total = store
      .getState()
      .models.find((m) => m.id === "small.en")!.sizeBytes;

    const asked = store.download("small.en");
    // Before the backend has answered.
    expect(download(store, "small.en")).toEqual({
      phase: "starting",
      receivedBytes: 0,
      totalBytes: total,
      error: null,
    });
    await wait(50);
    await asked;
    expect(download(store, "small.en")?.phase).toBe("downloading");

    await wait(10 * 150);
    const halfway = download(store, "small.en")!;
    expect(halfway.phase).toBe("downloading");
    expect(halfway.receivedBytes).toBeGreaterThan(0);
    expect(halfway.receivedBytes).toBeLessThan(total);
    expect(halfway.totalBytes).toBe(total);

    await wait(10 * 150);
    expect(download(store, "small.en")).toMatchObject({
      phase: "verifying",
      receivedBytes: total,
    });
    expect(installedIds(store)).toEqual(["base.en"]);

    await wait(VERIFY_MS);
    expect(download(store, "small.en")).toBeUndefined();
    // Shown as installed at once, before the list has been read again.
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
    await wait(50);
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("the first event may come after downloadModel has returned", async () => {
    // A backend that answers before it has sent anything.
    const api = createMockApi();
    const store = new ModelLibraryStore({
      ...api,
      downloadModel: async () => {},
    });
    store.connect();
    await store.load();

    await store.download("small.en");
    expect(download(store, "small.en")?.phase).toBe("downloading");
    expect(download(store, "small.en")?.receivedBytes).toBe(0);
  });

  test("a request the backend refuses is an error on the row", async () => {
    const { api, store } = await setup();
    api.mock.failNext("downloadModel", "The models folder cannot be written.");

    await store.download("small.en");
    expect(download(store, "small.en")).toMatchObject({
      phase: "error",
      error: "The models folder cannot be written.",
    });
  });

  test("a download that fails says why, and trying again installs the model", async () => {
    const { api, store } = await setup();
    api.mock.failNextDownload("The connection was lost.");

    await store.download("small.en");
    await wait(DOWNLOAD_MS);
    expect(download(store, "small.en")).toMatchObject({
      phase: "error",
      error: "The connection was lost.",
    });
    expect(installedIds(store)).toEqual(["base.en"]);

    await store.download("small.en");
    expect(download(store, "small.en")).toMatchObject({
      phase: "downloading",
      error: null,
    });
    await wait(DOWNLOAD_MS + VERIFY_MS);
    expect(download(store, "small.en")).toBeUndefined();
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("a failed download can be dismissed; a running one cannot", async () => {
    const { api, store } = await setup();
    api.mock.failNextDownload();
    await store.download("small.en");
    await wait(DOWNLOAD_MS);

    store.dismissDownloadError("small.en");
    expect(download(store, "small.en")).toBeUndefined();

    await store.download("medium.en");
    store.dismissDownloadError("medium.en");
    expect(download(store, "medium.en")?.phase).toBe("downloading");
  });

  test("asking twice starts one download", async () => {
    const { api, store } = await setup();
    const downloadModel = vi.spyOn(api, "downloadModel");

    await Promise.all([store.download("small.en"), store.download("small.en")]);
    await store.download("small.en");
    expect(downloadModel).toHaveBeenCalledTimes(1);
    expect(download(store, "small.en")?.phase).toBe("downloading");
  });

  test("two downloads run side by side, each with its own progress", async () => {
    const { store } = await setup();
    await store.download("small.en");
    await wait(5 * 150);
    await store.download("medium.en");
    await wait(5 * 150);

    expect(download(store, "small.en")!.receivedBytes).toBeGreaterThan(0);
    expect(download(store, "medium.en")!.receivedBytes).toBeGreaterThan(0);
    expect(download(store, "small.en")!.totalBytes).not.toBe(
      download(store, "medium.en")!.totalBytes,
    );

    await wait(DOWNLOAD_MS + VERIFY_MS);
    expect(installedIds(store)).toEqual(["base.en", "small.en", "medium.en"]);
  });

  test("progress is kept by the store, with no screen listening", async () => {
    const { store } = await setup();
    await store.download("small.en");
    // Nothing is subscribed to the store: the user is on another screen.
    await wait(10 * 150);

    expect(download(store, "small.en")?.phase).toBe("downloading");
    expect(download(store, "small.en")!.receivedBytes).toBeGreaterThan(0);
  });

  test("disconnecting stops listening to the backend", async () => {
    const { api, store, disconnect } = await setup();
    disconnect();
    await api.downloadModel("small.en");
    await wait(DOWNLOAD_MS + VERIFY_MS);

    expect(download(store, "small.en")).toBeUndefined();
    expect(installedIds(store)).toEqual(["base.en"]);
  });
});

describe("cancelling", () => {
  test("cancel ends the download and installs nothing", async () => {
    const { store } = await setup();
    await store.download("small.en");
    await wait(5 * 150);

    await store.cancelDownload("small.en");
    expect(download(store, "small.en")).toBeUndefined();
    await wait(DOWNLOAD_MS + VERIFY_MS);
    expect(installedIds(store)).toEqual(["base.en"]);
  });

  test("the row is cancelling from the press until the backend confirms", async () => {
    const { store } = await setup({ download: { cancelMs: 5000 } });
    await store.download("small.en");
    await wait(5 * 150);
    const received = download(store, "small.en")!.receivedBytes;

    const asked = store.cancelDownload("small.en");
    // At once, before the backend has even answered the call.
    expect(download(store, "small.en")).toMatchObject({
      phase: "cancelling",
      receivedBytes: received,
    });
    await asked;
    expect(download(store, "small.en")?.phase).toBe("cancelling");

    await wait(4999);
    expect(download(store, "small.en")?.phase).toBe("cancelling");
    // It cannot be started again, and a second cancel is not sent.
    await store.download("small.en");
    expect(download(store, "small.en")?.phase).toBe("cancelling");

    await wait(1);
    expect(download(store, "small.en")).toBeUndefined();
    expect(installedIds(store)).toEqual(["base.en"]);

    // Once it has stopped, it can be downloaded again.
    await store.download("small.en");
    await wait(DOWNLOAD_MS + VERIFY_MS);
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("cancel is sent once, however often it is pressed", async () => {
    const { api, store } = await setup({ download: { cancelMs: 5000 } });
    const cancel = vi.spyOn(api, "cancelDownload");
    await store.download("small.en");

    await store.cancelDownload("small.en");
    await store.cancelDownload("small.en");
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  test("a cancel that comes too late leaves the model installed", async () => {
    // The backend finishes the download although Cancel was pressed.
    const api = createMockApi();
    const store = new ModelLibraryStore({
      ...api,
      cancelDownload: async () => {},
    });
    store.connect();
    await store.load();
    await store.download("small.en");
    await wait(DOWNLOAD_MS);
    expect(download(store, "small.en")?.phase).toBe("verifying");

    await store.cancelDownload("small.en");
    expect(download(store, "small.en")?.phase).toBe("cancelling");
    await wait(VERIFY_MS);
    expect(download(store, "small.en")).toBeUndefined();
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("a cancel that cannot be sent leaves the download running and says why", async () => {
    const { api, store } = await setup();
    await store.download("small.en");
    await wait(5 * 150);
    api.mock.failNext("cancelDownload", "The downloader is not answering.");

    await store.cancelDownload("small.en");
    expect(download(store, "small.en")).toMatchObject({
      phase: "downloading",
      error: "Could not cancel: The downloader is not answering.",
    });

    await wait(DOWNLOAD_MS + VERIFY_MS);
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
  });

  test("there is nothing to cancel before the backend has the request, or after a failure", async () => {
    const { api, store } = await setup({ latencyMs: 50 });
    const cancel = vi.spyOn(api, "cancelDownload");

    const asked = store.download("small.en");
    await store.cancelDownload("small.en");
    expect(cancel).not.toHaveBeenCalled();
    expect(download(store, "small.en")?.phase).toBe("starting");
    await wait(50);
    await asked;

    api.mock.failNextDownload();
    const second = store.download("medium.en");
    await wait(50);
    await second;
    await wait(DOWNLOAD_MS);
    expect(download(store, "medium.en")?.phase).toBe("error");
    await store.cancelDownload("medium.en");
    expect(cancel).not.toHaveBeenCalled();
    expect(download(store, "medium.en")?.phase).toBe("error");
  });
});

describe("the default model", () => {
  test("the first model to be installed becomes the default", async () => {
    const { api, store } = await setup({ installedModelIds: [] });
    expect(defaultId(store)).toBeNull();

    await store.download("small.en");
    await wait(DOWNLOAD_MS + VERIFY_MS);

    expect(installedIds(store)).toEqual(["small.en"]);
    expect(defaultId(store)).toBe("small.en");
    expect(await savedDefault(api)).toBe("small.en");
  });

  test("installing a second model does not move the default", async () => {
    const { api, store } = await setup();
    const updateSettings = vi.spyOn(api, "updateSettings");

    await store.download("tiny");
    await wait(DOWNLOAD_MS + VERIFY_MS);

    expect(installedIds(store)).toEqual(["tiny", "base.en"]);
    expect(defaultId(store)).toBe("base.en");
    expect(await savedDefault(api)).toBe("base.en");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  test("with two downloads and no default, the one that finishes first is the default", async () => {
    const { api, store } = await setup({ installedModelIds: [] });
    await store.download("medium.en");
    await wait(10 * 150);
    // `tiny` comes before `medium` in the catalog, and finishes after it.
    await store.download("tiny");
    await wait(DOWNLOAD_MS + VERIFY_MS);

    expect(installedIds(store)).toEqual(["tiny", "medium.en"]);
    expect(defaultId(store)).toBe("medium.en");
    expect(await savedDefault(api)).toBe("medium.en");
  });

  test("a download that finishes before anything was read still sets the default", async () => {
    // The window was reloaded mid-download: the store is new and unloaded.
    const api = createMockApi({ installedModelIds: [] });
    await api.downloadModel("small.en");
    const store = new ModelLibraryStore(api);
    store.connect();

    await wait(DOWNLOAD_MS);
    expect(download(store, "small.en")?.phase).toBe("verifying");
    await wait(VERIFY_MS);

    expect(store.getState().status).toBe("ready");
    expect(defaultId(store)).toBe("small.en");
    expect(await savedDefault(api)).toBe("small.en");
  });

  test("deleting the default moves it to another installed model", async () => {
    const { api, store } = await setup({
      installedModelIds: ["base.en", "small.en", "medium.en"],
      settings: { defaultModelId: "small.en" },
    });

    await store.deleteModel("small.en");

    expect(installedIds(store)).toEqual(["base.en", "medium.en"]);
    expect(defaultId(store)).toBe("base.en");
    expect(await savedDefault(api)).toBe("base.en");
  });

  test("deleting the only model leaves no default", async () => {
    const { api, store } = await setup();

    await store.deleteModel("base.en");

    expect(installedIds(store)).toEqual([]);
    expect(defaultId(store)).toBeNull();
    expect(await savedDefault(api)).toBeNull();
  });

  test("deleting a model that is not the default leaves the default alone", async () => {
    const { api, store } = await setup({
      installedModelIds: ["base.en", "small.en"],
      settings: { defaultModelId: "small.en" },
    });

    await store.deleteModel("base.en");

    expect(installedIds(store)).toEqual(["small.en"]);
    expect(defaultId(store)).toBe("small.en");
    expect(await savedDefault(api)).toBe("small.en");
  });

  test("a delete the backend refuses rejects with its reason and changes nothing", async () => {
    const { api, store } = await setup({
      installedModelIds: ["base.en", "small.en"],
    });
    await api.startMeeting();

    await expect(store.deleteModel("base.en")).rejects.toThrow("in use");
    expect(installedIds(store)).toEqual(["base.en", "small.en"]);
    expect(defaultId(store)).toBe("base.en");
  });

  test("models installed with no default saved get one when they are read", async () => {
    const { api, store } = await setup({
      installedModelIds: ["small.en", "medium.en"],
      settings: { defaultModelId: null },
    });

    expect(defaultId(store)).toBe("small.en");
    expect(await savedDefault(api)).toBe("small.en");
  });

  test("a default that cannot be saved is reported, and the next read tries again", async () => {
    const api = createMockApi({ installedModelIds: [] });
    const store = new ModelLibraryStore(api);
    store.connect();
    await store.load();
    await store.download("small.en");
    api.mock.failNext("updateSettings", "The settings file is read-only.");
    await wait(DOWNLOAD_MS + VERIFY_MS);

    expect(installedIds(store)).toEqual(["small.en"]);
    expect(defaultId(store)).toBeNull();
    expect(store.getState().defaultError).toBe(
      "The settings file is read-only.",
    );

    await store.load();
    expect(defaultId(store)).toBe("small.en");
    expect(store.getState().defaultError).toBeNull();
  });
});

describe("updateSettings", () => {
  test("saves the change and keeps the backend's answer", async () => {
    const { api, store } = await setup({
      installedModelIds: ["base.en", "small.en"],
    });

    const saved = await store.updateSettings({ defaultModelId: "small.en" });
    expect(saved.defaultModelId).toBe("small.en");
    expect(defaultId(store)).toBe("small.en");
    expect(await savedDefault(api)).toBe("small.en");
  });

  test("a refused change rejects and leaves the settings as they were", async () => {
    const { store } = await setup();

    await expect(
      store.updateSettings({ defaultModelId: "medium.en" }),
    ).rejects.toThrow("not installed");
    expect(defaultId(store)).toBe("base.en");
  });
});
