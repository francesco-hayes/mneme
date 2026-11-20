import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mockApi, type MockOptions } from "../lib/api/mock";
import { advance, renderShell } from "../test/render";

const DOWNLOAD_MS = 20 * 150;
const VERIFY_MS = 600;

const installedList = () =>
  screen.getByRole("list", { name: "Installed models" });
const downloadList = () =>
  screen.getByRole("list", { name: "Models to download" });
/** A model's row, wherever it is. */
const row = (name: string) => screen.getByRole("listitem", { name });
const names = (list: HTMLElement) =>
  within(list)
    .getAllByRole("listitem")
    .map((item) => item.querySelector("[id]")?.textContent);
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const queryButton = (name: string | RegExp) =>
  screen.queryByRole("button", { name });
const defaultRadio = (model: string) =>
  screen.getByRole<HTMLInputElement>("radio", {
    name: `Use ${model} for new meetings`,
  });
const savedSettings = () => mockApi.getSettings();

/** Settings as the screen the shell opens on. */
function renderSettings(options?: MockOptions) {
  if (options) mockApi.mock.reset(options);
  return renderShell({ route: { name: "settings" } });
}

describe("with the mock's timers", () => {
  // Downloads, recordings and the mock's delays run on timers, so these tests
  // use fake ones: `advance(1500)` is ten steps of a download.
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

  async function openSettings(options?: MockOptions) {
    const view = renderSettings(options);
    await advance();
    return view;
  }

  describe("the model list", () => {
    test("shows every model: the installed one as the default, the rest to download", async () => {
      await openSettings();

      expect(
        screen.getByRole("heading", { name: "Settings" }),
      ).toBeInTheDocument();
      expect(names(installedList())).toEqual(["Base (English)"]);
      expect(names(downloadList())).toEqual([
        "Tiny (English)",
        "Small (English)",
        "Medium (English)",
        "Large v3 Turbo",
        "Large v3",
      ]);

      const base = within(row("Base (English)"));
      expect(base.getByText("148 MB")).toBeVisible();
      expect(base.getByText("Fast. More accurate than Tiny.")).toBeVisible();
      expect(base.getByText("Default")).toBeVisible();
      expect(defaultRadio("Base (English)")).toBeChecked();
      expect(
        base.getByRole("button", { name: "Delete Base (English)" }),
      ).toBeEnabled();
      expect(base.queryByRole("button", { name: /Download/ })).toBeNull();

      const large = within(row("Large v3"));
      expect(large.getByText("3.1 GB")).toBeVisible();
      expect(
        large.getByText("Most accurate. Slowest and largest."),
      ).toBeVisible();
      expect(
        large.getByRole("button", { name: "Download Large v3" }),
      ).toBeEnabled();
      expect(large.queryByText("Default")).toBeNull();
      expect(large.queryByRole("radio")).toBeNull();
      expect(large.queryByRole("button", { name: /Delete/ })).toBeNull();
    });

    test("marks the English-only models with a tag, and says it once", async () => {
      await openSettings();

      const tagged = screen
        .getAllByText("English only")
        .map((tag) => tag.closest("li")?.getAttribute("aria-labelledby"))
        .map((id) => document.getElementById(id!)?.textContent);
      // The installed list comes first, then the models to download.
      expect(tagged).toEqual([
        "Base (English)",
        "Tiny (English)",
        "Small (English)",
        "Medium (English)",
      ]);
      // The catalog's note ends in "English only."; the tag replaces that.
      const tiny = within(row("Tiny (English)"));
      expect(
        tiny.getByText("Fastest and smallest. Least accurate."),
      ).toBeVisible();
      expect(tiny.getByText("77.7 MB")).toBeVisible();
      expect(screen.queryByText(/English only\./)).toBeNull();
    });

    test("with nothing installed it says so, and every model can be downloaded", async () => {
      await openSettings({ installedModelIds: [] });

      expect(screen.getByText(/No model is installed/)).toBeVisible();
      expect(
        screen.queryByRole("list", { name: "Installed models" }),
      ).toBeNull();
      expect(screen.queryByRole("radio")).toBeNull();
      expect(names(downloadList())).toHaveLength(6);
    });

    test("says it is loading only if the read drags on", async () => {
      renderSettings({ latencyMs: 2000 });
      await advance();

      // In the page from the start, and held back by `appears-late`.
      expect(screen.getByText("Loading settings…")).toHaveClass("appears-late");
      await advance(2000);
      expect(screen.queryByText("Loading settings…")).toBeNull();
      expect(installedList()).toBeInTheDocument();
    });

    test("a failed read is shown with Retry, which reads again", async () => {
      mockApi.mock.failNext("listModels", "The models folder is missing.");
      await openSettings();

      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not load the settings: The models folder is missing.",
      );
      await click("Retry");
      expect(screen.queryByRole("alert")).toBeNull();
      expect(names(installedList())).toEqual(["Base (English)"]);
    });
  });

  describe("downloading", () => {
    test("shows progress with received and total, then verifying, then the model is installed", async () => {
      await openSettings();
      await click("Download Small (English)");

      const small = () => within(row("Small (English)"));
      const bar = () =>
        small().getByRole<HTMLProgressElement>("progressbar", {
          name: "Downloading Small (English)",
        });
      expect(bar()).toHaveAttribute("max", "487614201");
      expect(bar()).toHaveAttribute("value", "0");
      expect(small().getByText("0 KB of 488 MB")).toBeVisible();
      expect(small().queryByRole("button", { name: /Download/ })).toBeNull();
      expect(
        small().getByRole("button", {
          name: "Cancel downloading Small (English)",
        }),
      ).toBeEnabled();

      await advance(10 * 150);
      expect(bar()).toHaveAttribute("value", "243807110");
      expect(small().getByText("244 MB of 488 MB")).toBeVisible();
      // The bar is filled by its attributes: nothing carries an inline style,
      // which the app's content security policy would refuse.
      expect(document.querySelector("[style]")).toBeNull();
      expect(document.querySelector("style")).toBeNull();

      await advance(10 * 150);
      expect(small().getByText("Verifying…")).toBeVisible();
      expect(bar()).toHaveAttribute("value", "487614201");
      expect(names(installedList())).toEqual(["Base (English)"]);

      await advance(VERIFY_MS);
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
      expect(names(downloadList())).not.toContain("Small (English)");
      expect(small().queryByRole("progressbar")).toBeNull();
      expect(small().getByText("488 MB")).toBeVisible();
      expect(
        small().getByRole("button", { name: "Delete Small (English)" }),
      ).toBeEnabled();
      // A second model does not take the default away from the first.
      expect(defaultRadio("Base (English)")).toBeChecked();
      expect(defaultRadio("Small (English)")).not.toBeChecked();
      expect((await savedSettings()).defaultModelId).toBe("base.en");
    });

    test("before the backend answers, the row says it is starting and cannot be cancelled yet", async () => {
      await openSettings({ latencyMs: 300 });
      await advance(300);
      fireEvent.click(button("Download Small (English)"));
      await advance();

      const small = within(row("Small (English)"));
      expect(small.getByText("Starting…")).toBeVisible();
      // No value: the bar is indeterminate.
      expect(small.getByRole("progressbar")).not.toHaveAttribute("value");
      expect(
        small.getByRole("button", {
          name: "Cancel downloading Small (English)",
        }),
      ).toBeDisabled();

      await advance(300);
      expect(small.getByText("0 KB of 488 MB")).toBeVisible();
      expect(
        small.getByRole("button", {
          name: "Cancel downloading Small (English)",
        }),
      ).toBeEnabled();
    });

    test("Cancel stops the download and the row offers Download again", async () => {
      await openSettings();
      await click("Download Medium (English)");
      await advance(5 * 150);
      expect(
        within(row("Medium (English)")).getByRole("progressbar"),
      ).toBeVisible();

      await click("Cancel downloading Medium (English)");
      const medium = within(row("Medium (English)"));
      expect(medium.queryByRole("progressbar")).toBeNull();
      expect(
        medium.getByRole("button", { name: "Download Medium (English)" }),
      ).toBeEnabled();
      expect(medium.getByText("1.53 GB")).toBeVisible();

      await advance(DOWNLOAD_MS + VERIFY_MS);
      expect(names(installedList())).toEqual(["Base (English)"]);
    });

    test("until the backend confirms a cancel, the row says it is cancelling", async () => {
      await openSettings({ download: { cancelMs: 8000 } });
      await click("Download Medium (English)");
      await advance(5 * 150);
      await click("Cancel downloading Medium (English)");

      const medium = () => within(row("Medium (English)"));
      expect(medium().getByText("Cancelling…")).toBeVisible();
      // The bar stays where it stopped.
      expect(medium().getByRole("progressbar")).toHaveAttribute(
        "value",
        "383443700",
      );
      expect(
        medium().getByRole("button", {
          name: "Cancel downloading Medium (English)",
        }),
      ).toBeDisabled();
      expect(medium().queryByRole("button", { name: /^Download/ })).toBeNull();

      await advance(7999);
      expect(medium().getByText("Cancelling…")).toBeVisible();
      await advance(1);
      expect(medium().queryByText("Cancelling…")).toBeNull();
      expect(medium().queryByRole("progressbar")).toBeNull();
      expect(
        medium().getByRole("button", { name: "Download Medium (English)" }),
      ).toBeEnabled();
    });

    test("a failed download says why on its row, and Retry downloads it", async () => {
      await openSettings();
      mockApi.mock.failNextDownload("The connection was lost.");
      await click("Download Small (English)");
      await advance(DOWNLOAD_MS);

      const small = () => within(row("Small (English)"));
      expect(small().getByRole("alert")).toHaveTextContent(
        "Could not download Small (English): The connection was lost.",
      );
      expect(small().queryByRole("progressbar")).toBeNull();
      // Only that row is affected.
      expect(screen.getAllByRole("alert")).toHaveLength(1);
      expect(names(downloadList())).toContain("Small (English)");

      await click("Retry downloading Small (English)");
      expect(small().queryByRole("alert")).toBeNull();
      expect(small().getByRole("progressbar")).toHaveAttribute("value", "0");

      await advance(DOWNLOAD_MS + VERIFY_MS);
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
    });

    test("a failure can be dismissed instead", async () => {
      await openSettings();
      mockApi.mock.failNextDownload();
      await click("Download Small (English)");
      await advance(DOWNLOAD_MS);

      await click("Dismiss the error for Small (English)");
      const small = within(row("Small (English)"));
      expect(small.queryByRole("alert")).toBeNull();
      expect(
        small.getByRole("button", { name: "Download Small (English)" }),
      ).toBeEnabled();
    });

    test("a download the backend refuses to start is an error on the row", async () => {
      await openSettings();
      mockApi.mock.failNext("downloadModel", "The models folder is read-only.");
      await click("Download Small (English)");

      expect(
        within(row("Small (English)")).getByRole("alert"),
      ).toHaveTextContent(
        "Could not download Small (English): The models folder is read-only.",
      );
    });

    test("progress carries on while Settings is closed, and is there on return", async () => {
      const downloadModel = vi.spyOn(mockApi, "downloadModel");
      renderShell();
      await advance();
      await click("Settings");
      await click("Download Small (English)");
      await advance(5 * 150);
      expect(
        within(row("Small (English)")).getByText("122 MB of 488 MB"),
      ).toBeVisible();

      await click("Back");
      expect(
        screen.getByRole("heading", { name: "Meetings" }),
      ).toBeInTheDocument();
      expect(screen.queryByRole("progressbar")).toBeNull();
      await advance(5 * 150);

      await click("Settings");
      // Further along than when it was left, not started over.
      expect(
        within(row("Small (English)")).getByText("244 MB of 488 MB"),
      ).toBeVisible();
      expect(
        within(row("Small (English)")).getByRole("progressbar"),
      ).toHaveAttribute("value", "243807110");
      expect(downloadModel).toHaveBeenCalledTimes(1);

      // And it can finish while the user is elsewhere.
      await click("Back");
      await advance(10 * 150 + VERIFY_MS);
      await click("Settings");
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
    });

    test("a cancel that is still pending is still shown on return", async () => {
      mockApi.mock.reset({ download: { cancelMs: 8000 } });
      renderShell();
      await advance();
      await click("Settings");
      await click("Download Small (English)");
      await advance(5 * 150);
      await click("Cancel downloading Small (English)");

      await click("Back");
      await advance(1000);
      await click("Settings");
      expect(
        within(row("Small (English)")).getByText("Cancelling…"),
      ).toBeVisible();
    });

    test("two models download at the same time", async () => {
      await openSettings();
      await click("Download Tiny (English)");
      await advance(10 * 150);
      await click("Download Large v3");
      await advance(5 * 150);

      expect(
        within(row("Tiny (English)")).getByText(/of 77\.7 MB$/),
      ).toBeVisible();
      expect(within(row("Large v3")).getByText(/of 3\.1 GB$/)).toBeVisible();
      expect(screen.getAllByRole("progressbar")).toHaveLength(2);

      await advance(DOWNLOAD_MS + VERIFY_MS);
      expect(names(installedList())).toEqual([
        "Tiny (English)",
        "Base (English)",
        "Large v3",
      ]);
    });
  });

  describe("the default model", () => {
    test("the first model installed becomes the default by itself", async () => {
      await openSettings({ installedModelIds: [] });
      expect((await savedSettings()).defaultModelId).toBeNull();

      await click("Download Small (English)");
      await advance(DOWNLOAD_MS + VERIFY_MS);

      expect(names(installedList())).toEqual(["Small (English)"]);
      expect(defaultRadio("Small (English)")).toBeChecked();
      expect(within(row("Small (English)")).getByText("Default")).toBeVisible();
      expect((await savedSettings()).defaultModelId).toBe("small.en");
    });

    test("choosing another installed model saves it as the default", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      expect(defaultRadio("Base (English)")).toBeChecked();

      fireEvent.click(defaultRadio("Small (English)"));
      // Shown at once, before the backend has answered.
      expect(defaultRadio("Small (English)")).toBeChecked();
      await advance();

      expect(defaultRadio("Small (English)")).toBeChecked();
      expect(defaultRadio("Base (English)")).not.toBeChecked();
      expect(within(row("Small (English)")).getByText("Default")).toBeVisible();
      expect(within(row("Base (English)")).queryByText("Default")).toBeNull();
      expect((await savedSettings()).defaultModelId).toBe("small.en");
    });

    test("a default that cannot be saved is said in place, and the old one stays", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      mockApi.mock.failNext("updateSettings", "The settings file is locked.");

      fireEvent.click(defaultRadio("Small (English)"));
      await advance();

      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not change the default model: The settings file is locked.",
      );
      expect(defaultRadio("Base (English)")).toBeChecked();
      expect(defaultRadio("Small (English)")).not.toBeChecked();
      expect((await savedSettings()).defaultModelId).toBe("base.en");

      // The next change clears the message.
      fireEvent.click(defaultRadio("Small (English)"));
      await advance();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(defaultRadio("Small (English)")).toBeChecked();
    });
  });

  describe("deleting", () => {
    test("Delete asks in the row first, and Keep leaves everything as it was", async () => {
      const confirm = vi.spyOn(window, "confirm");
      const deleteModel = vi.spyOn(mockApi, "deleteModel");
      await openSettings({ installedModelIds: ["base.en", "small.en"] });

      await click("Delete Small (English)");
      const question = within(row("Small (English)")).getByRole("group", {
        name: "Delete Small (English)?",
      });
      expect(question).toHaveTextContent(
        "Delete Small (English) from this Mac?",
      );
      // Small (English) is not the default, so nothing is said about the default.
      expect(question).not.toHaveTextContent("default");
      // The question replaces the Delete button and opens on the safe answer.
      expect(queryButton("Delete Small (English)")).toBeNull();
      expect(
        within(question).getByRole("button", { name: "Keep" }),
      ).toHaveFocus();
      expect(deleteModel).not.toHaveBeenCalled();

      fireEvent.click(within(question).getByRole("button", { name: "Keep" }));
      await advance();
      expect(
        screen.queryByRole("group", { name: "Delete Small (English)?" }),
      ).toBeNull();
      expect(button("Delete Small (English)")).toHaveFocus();
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
      expect(deleteModel).not.toHaveBeenCalled();
      // No browser dialog at any point.
      expect(confirm).not.toHaveBeenCalled();
    });

    test("Escape answers the question with no", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      await click("Delete Small (English)");

      fireEvent.keyDown(screen.getByRole("button", { name: "Keep" }), {
        key: "Escape",
      });
      await advance();
      expect(
        screen.queryByRole("group", { name: "Delete Small (English)?" }),
      ).toBeNull();
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
    });

    test("confirming deletes the model, which can then be downloaded again", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      await click("Delete Small (English)");
      fireEvent.click(
        within(row("Small (English)")).getByRole("button", { name: "Delete" }),
      );
      await advance();

      expect(names(installedList())).toEqual(["Base (English)"]);
      expect(names(downloadList())).toContain("Small (English)");
      expect(button("Download Small (English)")).toBeEnabled();
      expect(
        (await mockApi.listModels()).find((m) => m.id === "small.en")
          ?.installed,
      ).toBe(false);
      expect(defaultRadio("Base (English)")).toBeChecked();
    });

    test("deleting the default model moves the default to another installed model", async () => {
      await openSettings({
        installedModelIds: ["base.en", "small.en", "medium.en"],
        settings: { defaultModelId: "small.en" },
      });
      expect(defaultRadio("Small (English)")).toBeChecked();

      await click("Delete Small (English)");
      // Said before it happens.
      expect(
        screen.getByRole("group", { name: "Delete Small (English)?" }),
      ).toHaveTextContent(
        "Delete Small (English) from this Mac? Base (English) becomes the default.",
      );
      fireEvent.click(
        within(row("Small (English)")).getByRole("button", { name: "Delete" }),
      );
      await advance();

      expect(names(installedList())).toEqual([
        "Base (English)",
        "Medium (English)",
      ]);
      expect(defaultRadio("Base (English)")).toBeChecked();
      expect(within(row("Base (English)")).getByText("Default")).toBeVisible();
      expect(defaultRadio("Medium (English)")).not.toBeChecked();
      expect((await savedSettings()).defaultModelId).toBe("base.en");
    });

    test("deleting the only model leaves no default", async () => {
      await openSettings();

      await click("Delete Base (English)");
      expect(
        screen.getByRole("group", { name: "Delete Base (English)?" }),
      ).toHaveTextContent(
        "No model will be left, and Mneme needs one to record.",
      );
      fireEvent.click(
        within(row("Base (English)")).getByRole("button", { name: "Delete" }),
      );
      await advance();

      expect(screen.getByText(/No model is installed/)).toBeVisible();
      expect(screen.queryByRole("radio")).toBeNull();
      expect(names(downloadList())).toHaveLength(6);
      expect((await savedSettings()).defaultModelId).toBeNull();
    });

    test("a delete that fails says why on the row and keeps the model", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      mockApi.mock.failNext("deleteModel", "The file is in use.");

      await click("Delete Small (English)");
      fireEvent.click(
        within(row("Small (English)")).getByRole("button", { name: "Delete" }),
      );
      await advance();

      expect(
        within(row("Small (English)")).getByRole("alert"),
      ).toHaveTextContent(
        "Could not delete Small (English): The file is in use.",
      );
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
      expect(button("Delete Small (English)")).toBeEnabled();
    });

    test("while a recording runs, no model can be deleted", async () => {
      mockApi.mock.reset({ installedModelIds: ["base.en", "small.en"] });
      await mockApi.startMeeting();
      await openSettings();

      expect(button("Delete Base (English)")).toBeDisabled();
      expect(button("Delete Small (English)")).toBeDisabled();
      expect(
        screen.getByText(
          /Models cannot be deleted while a recording is running\./,
        ),
      ).toBeVisible();
      // Everything else still works: the default, and downloads.
      expect(defaultRadio("Small (English)")).toBeEnabled();
      expect(button("Download Tiny (English)")).toBeEnabled();

      await click("Stop");
      await advance(600);
      expect(button("Delete Base (English)")).toBeEnabled();
      expect(button("Delete Small (English)")).toBeEnabled();
      expect(screen.queryByText(/Models cannot be deleted/)).toBeNull();
    });

    test("a recording that starts while the question is open withdraws it", async () => {
      const deleteModel = vi.spyOn(mockApi, "deleteModel");
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      await click("Delete Small (English)");
      expect(
        screen.getByRole("group", { name: "Delete Small (English)?" }),
      ).toBeInTheDocument();

      // Started from the menu bar or the global shortcut.
      await mockApi.startMeeting();
      await advance();

      expect(
        screen.queryByRole("group", { name: "Delete Small (English)?" }),
      ).toBeNull();
      expect(button("Delete Small (English)")).toBeDisabled();
      expect(deleteModel).not.toHaveBeenCalled();
    });
  });
});

describe("global shortcut", () => {
  const field = () => screen.findByRole("button", { name: /^Global shortcut/ });
  const clearButton = () =>
    screen.queryByRole("button", { name: "Clear the shortcut" });
  const savedShortcut = async () => (await savedSettings()).globalShortcut;

  test("is None until one is set", async () => {
    renderSettings();

    expect(await field()).toHaveTextContent("None");
    expect(await field()).toHaveAccessibleName("Global shortcut: None");
    expect(clearButton()).toBeNull();
    expect(await savedShortcut()).toBeNull();
  });

  test("click, press a combination, and it is saved and shown in Mac symbols", async () => {
    const user = userEvent.setup();
    renderSettings();

    await user.click(await field());
    expect(await field()).toHaveTextContent("Type shortcut");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Press a key combination. Esc cancels.",
    );

    // The modifiers show as they go down.
    await user.keyboard("{Meta>}{Shift>}");
    expect(await field()).toHaveTextContent("⇧⌘");
    await user.keyboard("r{/Shift}{/Meta}");

    await waitFor(async () =>
      expect(await savedShortcut()).toBe("Shift+Command+KeyR"),
    );
    expect(await field()).toHaveTextContent("⇧⌘R");
    expect(await field()).toHaveAccessibleName(
      "Global shortcut: Shift Command R",
    );
    // Recording is over: the help is gone and the field still has the focus.
    expect(screen.queryByRole("status")).toBeNull();
    expect(await field()).toHaveFocus();
    expect(clearButton()).toBeInTheDocument();
  });

  test("the field can be started from the keyboard, and records what is pressed next", async () => {
    const user = userEvent.setup();
    renderSettings();
    (await field()).focus();

    await user.keyboard("{Enter}");
    expect(await field()).toHaveTextContent("Type shortcut");
    await user.keyboard("{Control>}{Alt>} {/Alt}{/Control}");

    await waitFor(async () =>
      expect(await savedShortcut()).toBe("Control+Alt+Space"),
    );
    expect(await field()).toHaveTextContent("⌃⌥Space");
  });

  test("Clear removes the shortcut", async () => {
    const user = userEvent.setup();
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });
    expect(await field()).toHaveTextContent("⌥F5");

    await user.click(clearButton()!);

    await waitFor(async () => expect(await savedShortcut()).toBeNull());
    expect(await field()).toHaveTextContent("None");
    expect(clearButton()).toBeNull();
    expect(await field()).toHaveFocus();
  });

  test("a key without a modifier is not accepted, and says why", async () => {
    const user = userEvent.setup();
    const updateSettings = vi.spyOn(mockApi, "updateSettings");
    renderSettings();
    await user.click(await field());

    await user.keyboard("r");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Hold ⌘, ⌃ or ⌥ with the key.",
    );
    // Still listening, and nothing was saved.
    expect(await field()).toHaveTextContent("Type shortcut");
    expect(updateSettings).not.toHaveBeenCalled();

    // Shift alone would take a letter away from typing.
    await user.keyboard("{Shift>}r{/Shift}");
    expect(screen.getByRole("status")).toHaveTextContent(
      "Shift alone is not enough. Add ⌘, ⌃ or ⌥.",
    );
    expect(updateSettings).not.toHaveBeenCalled();

    // Space and Return do not click the field off while it listens.
    await user.keyboard(" {Enter}");
    expect(await field()).toHaveTextContent("Type shortcut");

    // With a modifier it goes through.
    await user.keyboard("{Alt>}r{/Alt}");
    await waitFor(async () => expect(await savedShortcut()).toBe("Alt+KeyR"));
    expect(await field()).toHaveTextContent("⌥R");
  });

  test("Escape stops recording and keeps what was set", async () => {
    const user = userEvent.setup();
    const updateSettings = vi.spyOn(mockApi, "updateSettings");
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });

    await user.click(await field());
    expect(clearButton()).toBeNull();
    await user.keyboard("{Meta>}{Escape}{/Meta}");

    expect(await field()).toHaveTextContent("⌥F5");
    expect(screen.queryByRole("status")).toBeNull();
    expect(updateSettings).not.toHaveBeenCalled();
    // Escape was the recorder's: Settings is still the screen.
    expect(
      screen.getByRole("heading", { name: "Settings" }),
    ).toBeInTheDocument();
  });

  test("Tab is left alone: the focus moves on and recording stops", async () => {
    const user = userEvent.setup();
    const updateSettings = vi.spyOn(mockApi, "updateSettings");
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });
    await user.click(await field());

    // The key is not taken from the browser...
    const notPrevented = fireEvent.keyDown(await field(), {
      key: "Tab",
      code: "Tab",
    });
    expect(notPrevented).toBe(true);
    // ...where another key is.
    expect(fireEvent.keyDown(await field(), { key: "a", code: "KeyA" })).toBe(
      false,
    );

    await user.tab({ shift: true });
    expect(await field()).not.toHaveFocus();
    expect(await field()).toHaveTextContent("⌥F5");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  test("a shortcut the backend refuses is said in place, and the previous one comes back", async () => {
    const user = userEvent.setup();
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });
    await user.click(await field());
    mockApi.mock.failNext(
      "updateSettings",
      "⌘Space is already used by the system.",
    );

    await user.keyboard("{Meta>} {/Meta}");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not set the shortcut: ⌘Space is already used by the system.",
    );
    expect(await field()).toHaveTextContent("⌥F5");
    expect(await savedShortcut()).toBe("Alt+F5");
    expect(clearButton()).toBeInTheDocument();

    // Trying again clears the message.
    await user.click(await field());
    await user.keyboard("{Meta>}{Shift>}k{/Shift}{/Meta}");
    await waitFor(async () =>
      expect(await savedShortcut()).toBe("Shift+Command+KeyK"),
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(await field()).toHaveTextContent("⇧⌘K");
  });

  test("a failed Clear keeps the shortcut", async () => {
    const user = userEvent.setup();
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });
    await field();
    mockApi.mock.failNext("updateSettings", "The settings file is locked.");

    await user.click(clearButton()!);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not set the shortcut: The settings file is locked.",
    );
    expect(await field()).toHaveTextContent("⌥F5");
    expect(await savedShortcut()).toBe("Alt+F5");
  });

  test("pressing the shortcut that is already set saves nothing", async () => {
    const user = userEvent.setup();
    const updateSettings = vi.spyOn(mockApi, "updateSettings");
    renderSettings({ settings: { globalShortcut: "Alt+F5" } });
    await user.click(await field());

    await user.keyboard("{Alt>}[F5]{/Alt}");

    expect(await field()).toHaveTextContent("⌥F5");
    expect(screen.queryByRole("status")).toBeNull();
    expect(updateSettings).not.toHaveBeenCalled();
  });

  test("a saved shortcut it cannot read is shown as it is", async () => {
    renderSettings({ settings: { globalShortcut: "Hyper+MediaPlayPause" } });
    expect(await field()).toHaveTextContent("Hyper+MediaPlayPause");
  });
});
