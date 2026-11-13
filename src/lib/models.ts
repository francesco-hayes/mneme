// The rules about transcription models that do not depend on React or on the
// backend: what a download's events mean, and which model is the default. `ModelLibraryStore`
// (src/state/modelLibrary.ts) applies them; Settings and First run show the
// result.

import type { DownloadProgress, ModelInfo } from "./api";

/**
 * Where a download stands, as a row shows it.
 *
 * - `starting`: asked for, and the backend has not answered yet.
 * - `downloading`: bytes are arriving.
 * - `verifying`: every byte is here and the checksum is being compared.
 * - `cancelling`: Cancel was pressed and the backend has not confirmed yet.
 *   With the real backend that takes a moment, and up to about 35 seconds
 *   while a connection is still being set up.
 * - `error`: it failed; `error` says why, and it can be tried again.
 *
 * A download that finished or was cancelled has no status at all: the model
 * is then installed, or it is not.
 */
export type DownloadPhase =
  "starting" | "downloading" | "verifying" | "cancelling" | "error";

export interface DownloadStatus {
  phase: DownloadPhase;
  receivedBytes: number;
  totalBytes: number;
  /**
   * In the `error` phase, why the download failed. In any other phase, why a
   * Cancel could not be carried out (the download is still running).
   */
  error: string | null;
}

export type Downloads = ReadonlyMap<string, DownloadStatus>;

const GENERIC_FAILURE = "The download failed.";

/**
 * The downloads after one progress event from the backend. Returns a new map
 * and never changes the one passed in.
 *
 * Once Cancel has been pressed the row stays `cancelling` whatever arrives
 * (bytes already on their way, a failure caused by the cancel itself) until
 * the backend ends the download: with `cancelled`, or with `done` when the
 * cancel came too late to stop it.
 */
export function applyDownloadEvent(
  downloads: Downloads,
  event: DownloadProgress,
): Downloads {
  const next = new Map(downloads);
  const previous = downloads.get(event.modelId);
  const cancelling = previous?.phase === "cancelling";

  switch (event.state) {
    case "downloading":
    case "verifying":
      next.set(event.modelId, {
        phase: cancelling ? "cancelling" : event.state,
        receivedBytes: event.receivedBytes,
        totalBytes: event.totalBytes,
        // A failed download that reports progress again has been restarted.
        error: previous && previous.phase !== "error" ? previous.error : null,
      });
      break;
    case "done":
    case "cancelled":
      next.delete(event.modelId);
      break;
    case "error":
      if (cancelling) {
        // The user asked for it to stop, and it has.
        next.delete(event.modelId);
      } else {
        next.set(event.modelId, {
          phase: "error",
          receivedBytes: event.receivedBytes,
          totalBytes: event.totalBytes,
          error: event.error?.trim() || GENERIC_FAILURE,
        });
      }
      break;
  }
  return next;
}

/** True while a download holds the row: anything but a failure waiting for Retry. */
export function isDownloadActive(status: DownloadStatus | undefined): boolean {
  return status !== undefined && status.phase !== "error";
}

/**
 * Which model should be the default, given what is installed.
 *
 * - The current default stays while it is installed. Installing another model
 *   never moves it.
 * - With no usable default, `preferred` (the model that has just finished
 *   downloading) becomes it: the first installed model is the default.
 * - Otherwise the first installed model in catalog order (the same choice the
 *   backend makes for a recording when no default is set), which is where the
 *   default moves when the default model is deleted.
 * - Null when nothing is installed.
 */
export function pickDefaultModel(
  models: readonly ModelInfo[],
  currentDefaultId: string | null,
  preferredId: string | null = null,
): string | null {
  const installed = (id: string | null) =>
    id !== null && models.some((model) => model.id === id && model.installed);

  if (installed(currentDefaultId)) return currentDefaultId;
  if (installed(preferredId)) return preferredId;
  return models.find((model) => model.installed)?.id ?? null;
}

const ENGLISH_ONLY_SUFFIX = /\s*English only\.?\s*$/i;

/**
 * The catalog's note for a model, without the closing "English only." that
 * the English-only models carry: rows say that with a tag.
 */
export function modelNote(model: ModelInfo): string {
  return model.englishOnly
    ? model.note.replace(ENGLISH_ONLY_SUFFIX, "")
    : model.note;
}
