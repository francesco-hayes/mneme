import { describe, expect, test } from "vitest";
import type { DownloadProgress, ModelInfo } from "./api";
import {
  applyDownloadEvent,
  type Downloads,
  type DownloadStatus,
  isDownloadActive,
  modelNote,
  pickDefaultModel,
} from "./models";

const event = (
  state: DownloadProgress["state"],
  receivedBytes = 0,
  extra: Partial<DownloadProgress> = {},
): DownloadProgress => ({
  modelId: "small",
  receivedBytes,
  totalBytes: 100,
  state,
  ...extra,
});

const run = (events: DownloadProgress[], start: Downloads = new Map()) =>
  events.reduce(applyDownloadEvent, start);

const status = (
  phase: DownloadStatus["phase"],
  receivedBytes = 0,
  error: string | null = null,
): DownloadStatus => ({ phase, receivedBytes, totalBytes: 100, error });

describe("applyDownloadEvent", () => {
  test("downloading and verifying events carry the bytes and the phase", () => {
    expect(run([event("downloading", 0)]).get("small")).toEqual(
      status("downloading", 0),
    );
    expect(
      run([event("downloading", 0), event("downloading", 40)]).get("small"),
    ).toEqual(status("downloading", 40));
    expect(
      run([event("downloading", 40), event("verifying", 100)]).get("small"),
    ).toEqual(status("verifying", 100));
  });

  test("done and cancelled end the download: no status is left", () => {
    expect(run([event("verifying", 100), event("done", 100)]).size).toBe(0);
    expect(run([event("downloading", 40), event("cancelled", 40)]).size).toBe(
      0,
    );
  });

  test("an error keeps the row, with the backend's sentence", () => {
    expect(
      run([
        event("downloading", 40),
        event("error", 50, { error: "Connection lost." }),
      ]).get("small"),
    ).toEqual(status("error", 50, "Connection lost."));
    // An error event without a sentence still says something.
    expect(run([event("error", 50)]).get("small")?.error).toBe(
      "The download failed.",
    );
  });

  test("the first event after a failure means it was started again", () => {
    const failed: Downloads = new Map([["small", status("error", 50, "No.")]]);
    expect(run([event("downloading", 0)], failed).get("small")).toEqual(
      status("downloading", 0),
    );
  });

  test("once cancelling, progress keeps the row cancelling until the backend ends it", () => {
    const cancelling: Downloads = new Map([
      ["small", status("cancelling", 40)],
    ]);

    expect(run([event("downloading", 60)], cancelling).get("small")).toEqual(
      status("cancelling", 60),
    );
    expect(run([event("verifying", 100)], cancelling).get("small")).toEqual(
      status("cancelling", 100),
    );
    expect(run([event("cancelled", 60)], cancelling).size).toBe(0);
    // A cancel that came too late: the model was installed after all.
    expect(run([event("done", 100)], cancelling).size).toBe(0);
    // A failure while stopping is the stop, not something to retry.
    expect(
      run([event("error", 60, { error: "Interrupted." })], cancelling).size,
    ).toBe(0);
  });

  test("each model has its own status, and the map passed in is not changed", () => {
    const before: Downloads = new Map([["medium", status("downloading", 10)]]);
    const after = run([event("downloading", 40)], before);

    expect(before.size).toBe(1);
    expect(after.get("medium")).toEqual(status("downloading", 10));
    expect(after.get("small")).toEqual(status("downloading", 40));
  });

  test("an event for a download nobody here started still shows", () => {
    // After the window is reloaded mid-download, say.
    expect(run([event("verifying", 100)]).get("small")?.phase).toBe(
      "verifying",
    );
  });
});

test("isDownloadActive: everything but a failure, and not nothing", () => {
  expect(isDownloadActive(undefined)).toBe(false);
  expect(isDownloadActive(status("error", 0, "No."))).toBe(false);
  for (const phase of [
    "starting",
    "downloading",
    "verifying",
    "cancelling",
  ] as const) {
    expect(isDownloadActive(status(phase))).toBe(true);
  }
});

const model = (id: string, installed: boolean): ModelInfo => ({
  id,
  name: id,
  sizeBytes: 1,
  note: "",
  englishOnly: id.endsWith(".en"),
  installed,
});

describe("pickDefaultModel", () => {
  const catalog = (...installed: string[]) =>
    ["tiny", "base", "small", "medium"].map((id) =>
      model(id, installed.includes(id)),
    );

  test("nothing installed: no default", () => {
    expect(pickDefaultModel(catalog(), null)).toBeNull();
    expect(pickDefaultModel(catalog(), "base")).toBeNull();
    expect(pickDefaultModel(catalog(), null, "small")).toBeNull();
  });

  test("the first installed model becomes the default", () => {
    expect(pickDefaultModel(catalog("small"), null, "small")).toBe("small");
    // Also when nobody says which one was just installed.
    expect(pickDefaultModel(catalog("small"), null)).toBe("small");
  });

  test("installing another model does not move the default", () => {
    expect(pickDefaultModel(catalog("base", "small"), "base", "small")).toBe(
      "base",
    );
    expect(pickDefaultModel(catalog("base", "tiny"), "base", "tiny")).toBe(
      "base",
    );
  });

  test("with no default, the model just installed wins over an older one", () => {
    expect(pickDefaultModel(catalog("tiny", "medium"), null, "medium")).toBe(
      "medium",
    );
  });

  test("a default that is no longer installed moves to the first installed model", () => {
    // `small` was the default and has been deleted.
    expect(pickDefaultModel(catalog("base", "medium"), null)).toBe("base");
    expect(pickDefaultModel(catalog("base", "medium"), "small")).toBe("base");
    expect(pickDefaultModel(catalog("medium"), "small")).toBe("medium");
  });

  test("a preferred model that is not installed is ignored", () => {
    expect(pickDefaultModel(catalog("base"), null, "small")).toBe("base");
  });
});

test("modelNote drops the closing 'English only.' that the tag already says", () => {
  expect(
    modelNote({
      ...model("tiny.en", false),
      note: "Fastest and smallest. Least accurate. English only.",
    }),
  ).toBe("Fastest and smallest. Least accurate.");
  // Only for English-only models, and only at the end.
  expect(modelNote({ ...model("base", false), note: "English only." })).toBe(
    "English only.",
  );
  expect(
    modelNote({ ...model("base.en", false), note: "English only. Fast." }),
  ).toBe("English only. Fast.");
});
