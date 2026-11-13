// App-wide model and settings state.
//
// `ModelLibraryStore` holds the app's one subscription to
// `api.onDownloadProgress`, the model list, the settings, and the state of
// every download. `ModelLibraryProvider` creates it once, above the screens,
// so a download keeps its progress while the user is on another screen, and
// Settings and First run read the same state with `useModelLibrary`.
//
// It is also where the default-model rules are applied, because the backend
// leaves them to the UI: installing a model does not change `defaultModelId`,
// and deleting the default model sets it to null. After every read of the
// models and settings the store compares the default with `pickDefaultModel`
// (src/lib/models.ts) and saves the difference with `api.updateSettings`. So
// the first model to finish downloading becomes the default, and deleting the
// default moves it to another installed model, or to none.

import { createContext, useContext, useSyncExternalStore } from "react";
import type { Api, DownloadProgress, ModelInfo, Settings } from "../lib/api";
import { errorMessage } from "../lib/errors";
import {
  applyDownloadEvent,
  type Downloads,
  type DownloadStatus,
  isDownloadActive,
  pickDefaultModel,
} from "../lib/models";

export interface ModelLibraryState {
  /**
   * `idle` until something asks for the models (`load`), then `loading`, and
   * `ready` or `error`. A later `load` keeps `ready` data on screen while it
   * reads again.
   */
  status: "idle" | "loading" | "ready" | "error";
  /** Why the last read failed, when `status` is `error`. */
  loadError: string | null;
  /** The catalog in the backend's order (smallest first). Empty until ready. */
  models: readonly ModelInfo[];
  /** Null until ready. */
  settings: Settings | null;
  /** By model id. A model with no entry is not being downloaded. */
  downloads: Downloads;
  /** Why the default model could not be saved when the rules moved it, or null. */
  defaultError: string | null;
}

const INITIAL: ModelLibraryState = {
  status: "idle",
  loadError: null,
  models: [],
  settings: null,
  downloads: new Map(),
  defaultError: null,
};

type LibraryApi = Pick<
  Api,
  | "listModels"
  | "getSettings"
  | "updateSettings"
  | "downloadModel"
  | "cancelDownload"
  | "deleteModel"
  | "onDownloadProgress"
>;

export class ModelLibraryStore {
  private state: ModelLibraryState = INITIAL;
  private readonly listeners = new Set<() => void>();
  private loadCount = 0;
  /** The model that last finished downloading: the first choice for a missing default. */
  private preferredDefault: string | null = null;
  private readonly api: LibraryApi;

  constructor(api: LibraryApi) {
    this.api = api;
  }

  // For `useSyncExternalStore`.
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  readonly getState = () => this.state;

  private set(patch: Partial<ModelLibraryState>) {
    this.state = { ...this.state, ...patch };
    for (const listener of [...this.listeners]) listener();
  }

  private setDownload(id: string, status: DownloadStatus | null) {
    const downloads = new Map(this.state.downloads);
    if (status === null) downloads.delete(id);
    else downloads.set(id, status);
    this.set({ downloads });
  }

  /**
   * Starts listening for download progress, and returns the function that
   * stops. The provider calls it once; nothing else subscribes to
   * `api.onDownloadProgress`.
   */
  connect(): () => void {
    return this.api.onDownloadProgress((event) => this.onProgress(event));
  }

  private onProgress(event: DownloadProgress) {
    this.set({ downloads: applyDownloadEvent(this.state.downloads, event) });
    if (event.state !== "done") return;

    // Show it as installed at once, then read the truth and settle the default.
    this.preferredDefault = event.modelId;
    this.set({
      models: this.state.models.map((model) =>
        model.id === event.modelId ? { ...model, installed: true } : model,
      ),
    });
    void this.load();
  }

  /**
   * Reads the models and the settings, then applies the default-model rules.
   * Screens call it when they are shown. It never rejects: a failure is in
   * `status` and `loadError`, and calling it again is the retry.
   */
  async load(): Promise<void> {
    const count = (this.loadCount += 1);
    if (this.state.status !== "ready") {
      this.set({ status: "loading", loadError: null });
    }
    try {
      const [models, settings] = await Promise.all([
        this.api.listModels(),
        this.api.getSettings(),
      ]);
      // A newer read is on its way, and its answer is the one to show.
      if (count !== this.loadCount) return;
      this.set({ status: "ready", loadError: null, models, settings });
      await this.settleDefault();
    } catch (error) {
      if (count !== this.loadCount) return;
      this.set({ status: "error", loadError: errorMessage(error) });
    }
  }

  /** Saves the default model the rules ask for, when it is not the one saved. */
  private async settleDefault(): Promise<void> {
    const { models, settings } = this.state;
    if (settings === null) return;
    const wanted = pickDefaultModel(
      models,
      settings.defaultModelId,
      this.preferredDefault,
    );
    if (wanted === settings.defaultModelId) {
      this.preferredDefault = null;
      if (this.state.defaultError !== null) this.set({ defaultError: null });
      return;
    }
    try {
      const saved = await this.api.updateSettings({ defaultModelId: wanted });
      this.preferredDefault = null;
      this.set({ settings: saved, defaultError: null });
    } catch (error) {
      this.set({ defaultError: errorMessage(error) });
    }
  }

  /**
   * Starts downloading a model. Progress, the outcome and any failure arrive
   * in `downloads`; the promise only says the request has been answered, and
   * never rejects. Retry is the same call.
   */
  async download(id: string): Promise<void> {
    if (isDownloadActive(this.state.downloads.get(id))) return;
    const totalBytes =
      this.state.models.find((model) => model.id === id)?.sizeBytes ?? 0;
    this.setDownload(id, {
      phase: "starting",
      receivedBytes: 0,
      totalBytes,
      error: null,
    });
    try {
      await this.api.downloadModel(id);
      // The first progress event may come just after the call returns.
      const now = this.state.downloads.get(id);
      if (now?.phase === "starting") {
        this.setDownload(id, { ...now, phase: "downloading" });
      }
    } catch (error) {
      this.setDownload(id, {
        phase: "error",
        receivedBytes: 0,
        totalBytes,
        error: errorMessage(error),
      });
    }
  }

  /**
   * Asks the backend to stop a download. The row is `cancelling` until the
   * backend's `cancelled` event arrives, which is not at once. Never rejects:
   * a cancel that could not be sent leaves the download running, with the
   * reason in its `error`.
   */
  async cancelDownload(id: string): Promise<void> {
    const before = this.state.downloads.get(id);
    if (
      before === undefined ||
      (before.phase !== "downloading" && before.phase !== "verifying")
    ) {
      return;
    }
    this.setDownload(id, { ...before, phase: "cancelling", error: null });
    try {
      await this.api.cancelDownload(id);
    } catch (error) {
      const now = this.state.downloads.get(id);
      if (now?.phase === "cancelling") {
        this.setDownload(id, {
          ...now,
          phase: before.phase,
          error: `Could not cancel: ${errorMessage(error)}`,
        });
      }
    }
  }

  /** Forgets a failed download, so its row offers Download again. */
  dismissDownloadError(id: string): void {
    if (this.state.downloads.get(id)?.phase === "error") {
      this.setDownload(id, null);
    }
  }

  /**
   * Deletes an installed model, then moves the default if it was the default.
   * Rejects with the backend's reason when the model could not be deleted.
   */
  async deleteModel(id: string): Promise<void> {
    await this.api.deleteModel(id);
    const { settings } = this.state;
    this.set({
      models: this.state.models.map((model) =>
        model.id === id ? { ...model, installed: false } : model,
      ),
      // What the backend has just done on its side.
      settings:
        settings !== null && settings.defaultModelId === id
          ? { ...settings, defaultModelId: null }
          : settings,
    });
    await this.load();
  }

  /**
   * Saves a change to the settings and returns them as the backend has them.
   * Rejects with the backend's reason, and then nothing here has changed.
   */
  async updateSettings(patch: Partial<Settings>): Promise<Settings> {
    const saved = await this.api.updateSettings(patch);
    this.set({
      settings: saved,
      defaultError: "defaultModelId" in patch ? null : this.state.defaultError,
    });
    return saved;
  }
}

export const ModelLibraryContext = createContext<ModelLibraryStore | null>(
  null,
);

/** What `useModelLibrary` gives: the state, and the store's actions. */
export interface ModelLibrary extends ModelLibraryState {
  /** The default model, when one is set and installed. */
  defaultModel: ModelInfo | undefined;
  load: () => Promise<void>;
  download: (id: string) => Promise<void>;
  cancelDownload: (id: string) => Promise<void>;
  dismissDownloadError: (id: string) => void;
  deleteModel: (id: string) => Promise<void>;
  updateSettings: (patch: Partial<Settings>) => Promise<Settings>;
}

/**
 * The shared model library. It does not read anything by itself: a screen
 * that shows models calls `load()` when it is shown.
 *
 *     const library = useModelLibrary();
 *     const { load } = library;
 *     useEffect(() => void load(), [load]);
 */
export function useModelLibrary(): ModelLibrary {
  const store = useContext(ModelLibraryContext);
  if (store === null) {
    throw new Error("useModelLibrary needs a ModelLibraryProvider above it.");
  }
  const state = useSyncExternalStore(store.subscribe, store.getState);
  const actions = storeActions(store);
  const defaultId = state.settings?.defaultModelId ?? null;
  return {
    ...state,
    defaultModel: state.models.find(
      (model) => model.id === defaultId && model.installed,
    ),
    ...actions,
  };
}

type Actions = Pick<
  ModelLibrary,
  | "load"
  | "download"
  | "cancelDownload"
  | "dismissDownloadError"
  | "deleteModel"
  | "updateSettings"
>;

// One set of bound functions per store, so they are stable across renders and
// safe in dependency lists.
const actionsByStore = new WeakMap<ModelLibraryStore, Actions>();

function storeActions(store: ModelLibraryStore): Actions {
  let actions = actionsByStore.get(store);
  if (actions === undefined) {
    actions = {
      load: () => store.load(),
      download: (id) => store.download(id),
      cancelDownload: (id) => store.cancelDownload(id),
      dismissDownloadError: (id) => store.dismissDownloadError(id),
      deleteModel: (id) => store.deleteModel(id),
      updateSettings: (patch) => store.updateSettings(patch),
    };
    actionsByStore.set(store, actions);
  }
  return actions;
}
