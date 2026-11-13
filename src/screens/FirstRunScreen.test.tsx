import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import App from "../App";
import { mockApi, type MockOptions, mockScenarios } from "../lib/api/mock";
import { advance, renderShell } from "../test/render";

const DOWNLOAD_MS = 20 * 150;
const VERIFY_MS = 600;

const heading = (name: string) => screen.queryByRole("heading", { name });
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const queryButton = (name: string | RegExp) =>
  screen.queryByRole("button", { name });
const row = (name: string) => screen.getByRole("listitem", { name });
const modelRadio = (name: string) =>
  screen.getByRole<HTMLInputElement>("radio", { name });
const rowNames = () =>
  within(screen.getByRole("list", { name: "Models" }))
    .getAllByRole("listitem")
    .map((item) => item.querySelector("[id]")?.textContent);
const permission = (kind: "microphone" | "systemAudio") => {
  const status = document.querySelector(`[data-permission="${kind}"]`);
  if (!(status instanceof HTMLElement)) throw new Error(`No ${kind} status`);
  return status;
};
const defaultModelId = async () => (await mockApi.getSettings()).defaultModelId;

// Downloads and the mock's delays run on timers, so every test uses fake
// ones: `advance(1500)` is half of a download.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const click = async (name: string | RegExp) => {
  fireEvent.click(button(name));
  await advance();
};

/** The whole app, gate included, on a Mac with no model. */
async function openApp(options: MockOptions = {}) {
  mockApi.mock.reset({ ...mockScenarios["first-run"], ...options });
  const view = render(<App />);
  await advance();
  return view;
}

/**
 * First run with a model already downloaded, which is the permissions step:
 * the screen picks its step from the model library and keeps none itself.
 */
async function openPermissions(permissions: MockOptions["permissions"]) {
  mockApi.mock.reset({
    ...mockScenarios["first-run"],
    installedModelIds: ["small"],
    permissions,
  });
  const view = renderShell({ route: { name: "first-run" } });
  await advance();
  return view;
}

describe("step 1: the model", () => {
  test("with no model installed the app opens on First run, with the recommended model selected", async () => {
    await openApp();

    expect(heading("Welcome to Mneme")).toBeInTheDocument();
    expect(heading("Choose a transcription model")).toBeInTheDocument();
    expect(heading("Meetings")).toBeNull();
    expect(screen.getByText(/on this Mac, and needs a model/)).toBeVisible();

    expect(rowNames()).toEqual(["Small"]);
    expect(modelRadio("Small")).toBeChecked();
    expect(within(row("Small")).getByText("Recommended")).toBeVisible();
    expect(within(row("Small")).getByText("488 MB")).toBeVisible();
    expect(button("Download Small")).toBeEnabled();

    expect(
      screen.getByText(
        "Nothing you record leaves this Mac. Downloading the model is the only time Mneme uses the network.",
      ),
    ).toBeVisible();
  });

  test("the full list is there to choose from, with sizes and notes", async () => {
    await openApp();
    await click("Show all models");

    expect(rowNames()).toEqual([
      "Tiny",
      "Tiny (English)",
      "Base",
      "Base (English)",
      "Small",
      "Small (English)",
      "Medium",
      "Medium (English)",
      "Large v3 Turbo",
      "Large v3",
    ]);
    const base = within(row("Base"));
    expect(base.getByText("148 MB")).toBeVisible();
    expect(base.getByText("Fast. More accurate than Tiny.")).toBeVisible();
    expect(base.queryByText("Recommended")).toBeNull();
    // Still the recommended one until another is picked.
    expect(modelRadio("Small")).toBeChecked();

    fireEvent.click(modelRadio("Base"));
    expect(modelRadio("Base")).toBeChecked();
    expect(button("Download Base")).toBeEnabled();
    expect(queryButton("Download Small")).toBeNull();
  });

  test("downloading shows progress, then verifying, then the permissions step, with the model as the default", async () => {
    const { container } = await openApp();
    expect(await defaultModelId()).toBeNull();

    await click("Download Small");
    await advance(DOWNLOAD_MS / 2);

    const small = within(row("Small"));
    expect(
      small.getByRole("progressbar", { name: "Downloading Small" }),
    ).toBeVisible();
    expect(small.getByText("244 MB of 488 MB")).toBeVisible();
    expect(queryButton("Download Small")).toBeNull();
    expect(modelRadio("Small")).toBeDisabled();
    expect(heading("Allow recording")).toBeNull();

    await advance(DOWNLOAD_MS / 2);
    expect(small.getByText("Verifying…")).toBeVisible();
    expect(heading("Allow recording")).toBeNull();

    await advance(VERIFY_MS);
    expect(heading("Allow recording")).toBeInTheDocument();
    expect(heading("Choose a transcription model")).toBeNull();
    expect(screen.getByText(/^Small is installed\./)).toBeVisible();
    expect(await defaultModelId()).toBe("small");

    // The app's policy is `style-src 'self'`.
    expect(container.querySelector("[style]")).toBeNull();
    expect(document.querySelector("style")).toBeNull();
  });

  test("a model other than the recommended one can be downloaded, and becomes the default", async () => {
    await openApp();
    await click("Show all models");
    fireEvent.click(modelRadio("Tiny"));
    await click("Download Tiny");
    await advance(DOWNLOAD_MS + VERIFY_MS);

    expect(heading("Allow recording")).toBeInTheDocument();
    expect(await defaultModelId()).toBe("tiny");
  });

  test("Cancel stops the download and leaves the choice open", async () => {
    await openApp();
    await click("Download Small");
    await advance(DOWNLOAD_MS / 2);

    await click("Cancel downloading Small");

    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(button("Download Small")).toBeEnabled();
    expect(modelRadio("Small")).toBeEnabled();
    await advance(DOWNLOAD_MS + VERIFY_MS);
    expect(heading("Allow recording")).toBeNull();
    expect(await defaultModelId()).toBeNull();
  });

  test("a failed download says why, and Retry downloads it", async () => {
    await openApp();
    mockApi.mock.failNextDownload("The connection was lost.");
    await click("Download Small");
    await advance(DOWNLOAD_MS);

    expect(within(row("Small")).getByRole("alert")).toHaveTextContent(
      "Could not download Small: The connection was lost.",
    );
    expect(heading("Allow recording")).toBeNull();
    expect(await defaultModelId()).toBeNull();

    await click("Retry downloading Small");
    expect(screen.queryByRole("alert")).toBeNull();
    await advance(DOWNLOAD_MS + VERIFY_MS);

    expect(heading("Allow recording")).toBeInTheDocument();
    expect(await defaultModelId()).toBe("small");
  });

  test("the permissions step waits until the model has been saved as the default", async () => {
    await openApp();
    await click("Download Small");
    mockApi.mock.failNext("updateSettings", "The settings file is locked.");
    await advance(DOWNLOAD_MS + VERIFY_MS);

    expect(heading("Allow recording")).toBeNull();
    expect(within(row("Small")).getByText("Installed")).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not set the default model: The settings file is locked.",
    );
    expect(queryButton("Download Small")).toBeNull();

    await click("Retry");
    expect(heading("Allow recording")).toBeInTheDocument();
    expect(await defaultModelId()).toBe("small");
  });

  test("quitting during a download and opening the app again starts First run afresh", async () => {
    const first = await openApp();
    await click("Download Small");
    await advance(DOWNLOAD_MS / 2);
    first.unmount();

    // The app starts again: the unfinished file is not a model.
    await openApp();

    expect(heading("Choose a transcription model")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(button("Download Small")).toBeEnabled();
  });
});

describe("step 2: permissions", () => {
  test("permissions not asked yet: each says why it is needed and has a button that asks", async () => {
    await openPermissions({
      microphone: "undetermined",
      systemAudio: "undetermined",
    });

    expect(heading("Allow recording")).toBeInTheDocument();
    expect(screen.getByText("Records your side of the meeting.")).toBeVisible();
    expect(
      screen.getByText(
        "Records the other side of the call: the sound your Mac plays.",
      ),
    ).toBeVisible();
    expect(
      screen.getByText(
        "Use headphones: without them the microphone also hears the other side of the call.",
      ),
    ).toBeVisible();

    await click("Allow the microphone");
    expect(permission("microphone")).toHaveTextContent("Allowed");
    expect(queryButton("Allow the microphone")).toBeNull();
    expect(button("Allow system audio")).toBeEnabled();

    await click("Allow system audio");
    expect(permission("systemAudio")).toHaveTextContent("Allowed");
    expect(await mockApi.getPermissions()).toEqual({
      microphone: "granted",
      systemAudio: "granted",
    });
    expect(button("Finish")).toBeEnabled();
  });

  test("granted permissions show as allowed, with nothing to press but Finish", async () => {
    await openPermissions({ microphone: "granted", systemAudio: "granted" });

    expect(permission("microphone")).toHaveTextContent("Allowed");
    expect(permission("systemAudio")).toHaveTextContent("Allowed");
    expect(queryButton(/^Allow/)).toBeNull();
    expect(queryButton(/System Settings/)).toBeNull();
    expect(button("Finish")).toBeEnabled();
  });

  test("a denied permission says how to fix it in System Settings", async () => {
    await openPermissions({ microphone: "denied", systemAudio: "denied" });

    expect(permission("microphone")).toHaveTextContent("Not allowed");
    expect(
      screen.getByText(/open Privacy & Security > Microphone and turn on/),
    ).toBeVisible();
    expect(permission("systemAudio")).toHaveTextContent("Not allowed");
    expect(
      screen.getByText(
        /open Privacy & Security > Screen & System Audio Recording and turn on/,
      ),
    ).toBeVisible();
    expect(queryButton(/^Allow/)).toBeNull();
    expect(button("Open System Settings for the microphone")).toBeEnabled();
  });

  test("answering no to the prompt shows the permission as denied", async () => {
    mockApi.mock.reset({
      ...mockScenarios["first-run"],
      installedModelIds: ["small"],
      permissionResponses: { microphone: "denied" },
    });
    renderShell({ route: { name: "first-run" } });
    await advance();

    await click("Allow the microphone");

    expect(permission("microphone")).toHaveTextContent("Not allowed");
    expect(button("Finish")).toBeDisabled();
  });

  test("the microphone cannot be skipped: Finish is disabled until it is allowed", async () => {
    await openPermissions({
      microphone: "undetermined",
      systemAudio: "granted",
    });

    expect(button("Finish")).toBeDisabled();
    expect(
      screen.getByText(/cannot record without the microphone/),
    ).toBeVisible();
    fireEvent.click(button("Finish"));
    await advance();
    expect(heading("Meetings")).toBeNull();

    await click("Allow the microphone");
    expect(button("Finish")).toBeEnabled();
  });

  test("a denied microphone blocks Finish until it is allowed in System Settings", async () => {
    await openPermissions({ microphone: "denied", systemAudio: "granted" });
    expect(button("Finish")).toBeDisabled();

    // The user turns it on in System Settings and comes back to the window.
    mockApi.mock.setPermission("microphone", "granted");
    fireEvent.focus(window);
    await advance();

    expect(permission("microphone")).toHaveTextContent("Allowed");
    expect(button("Finish")).toBeEnabled();
  });

  test.each(["undetermined", "denied"] as const)(
    "system audio can be skipped when it is %s: Finish goes to Home",
    async (systemAudio) => {
      await openPermissions({ microphone: "granted", systemAudio });

      expect(
        screen.getByText(
          "Without system audio, a recording has only your side.",
        ),
      ).toBeVisible();
      await click("Finish without system audio");

      expect(heading("Meetings")).toBeInTheDocument();
      expect(heading("Welcome to Mneme")).toBeNull();
      expect((await mockApi.getPermissions()).systemAudio).toBe(systemAudio);
    },
  );

  test("a failure to read the permissions is shown, and Retry reads them", async () => {
    mockApi.mock.reset({
      ...mockScenarios["first-run"],
      installedModelIds: ["small"],
    });
    mockApi.mock.failNext(
      "getPermissions",
      "The audio service is not running.",
    );
    renderShell({ route: { name: "first-run" } });
    await advance();

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not read the permissions: The audio service is not running.",
    );
    await click("Retry");
    expect(button("Allow the microphone")).toBeEnabled();
  });
});

test("the whole of first run: download, allow, Finish, and Home opens with the model as the default", async () => {
  await openApp();

  await click("Download Small");
  await advance(DOWNLOAD_MS + VERIFY_MS);
  await click("Allow the microphone");
  await click("Allow system audio");
  await click("Finish");

  expect(heading("Meetings")).toBeInTheDocument();
  expect(heading("Welcome to Mneme")).toBeNull();
  // First run is not a screen to go back to.
  expect(queryButton("Back")).toBeNull();
  expect(await defaultModelId()).toBe("small");
  expect(
    (await mockApi.listModels()).filter((model) => model.installed),
  ).toMatchObject([{ id: "small" }]);
});
