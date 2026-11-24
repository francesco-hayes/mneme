import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mockApi, type MockOptions } from "../lib/api/mock";
import { advance, renderShell } from "../test/render";

const DOWNLOAD_MS = 20 * 150;
const VERIFY_MS = 600;

/** The button that shows the model in use and opens the menu. */
const picker = () => screen.getByRole("button", { name: /^Model / });
const installedList = () =>
  screen.getByRole("list", { name: "Installed models" });
const downloadList = () =>
  screen.getByRole("list", { name: "Models to download" });
const queryList = (name: string) => screen.queryByRole("list", { name });
/** A model's row in the open menu, wherever it is. */
const row = (name: string) => screen.getByRole("listitem", { name });
const names = (list: HTMLElement) =>
  within(list)
    .getAllByRole("listitem")
    .map((item) => item.querySelector("[id]")?.textContent);
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const queryButton = (name: string | RegExp) =>
  screen.queryByRole("button", { name });
const defaultRadio = (model: string) =>
  screen.getByRole("radio", { name: `Use ${model} for new meetings` });
const progressBar = (model: string) =>
  screen.getByRole<HTMLProgressElement>("progressbar", {
    name: `Downloading ${model}`,
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

  async function openMenu() {
    fireEvent.click(picker());
    await advance();
  }

  /** Settings with the model menu open. */
  async function openModels(options?: MockOptions) {
    const view = await openSettings(options);
    await openMenu();
    return view;
  }

  /** Escape, pressed where the focus is. */
  async function pressEscape() {
    fireEvent.keyDown(document.activeElement ?? document.body, {
      key: "Escape",
    });
    await advance();
  }

  describe("the model picker", () => {
    test("closed, it is one control that shows the model new meetings use", async () => {
      await openSettings();

      expect(
        screen.getByRole("heading", { name: "Settings" }),
      ).toBeInTheDocument();
      expect(picker()).toHaveTextContent("Base (English)");
      expect(picker()).toHaveAccessibleName(
        "Model for new meetings Base (English)",
      );
      expect(picker()).toHaveAttribute("aria-expanded", "false");
      // No list of models takes up the page.
      expect(queryList("Installed models")).toBeNull();
      expect(queryList("Models to download")).toBeNull();
      expect(screen.queryByRole("listitem")).toBeNull();
    });

    test("open, it has the installed model as the default and the rest to download", async () => {
      await openModels();

      expect(picker()).toHaveAttribute("aria-expanded", "true");
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
      expect(defaultRadio("Base (English)")).toBeChecked();
      // The menu opens with the focus on the model in use.
      expect(defaultRadio("Base (English)")).toHaveFocus();
      expect(
        base.getByRole("button", { name: "Delete Base (English)" }),
      ).toBeEnabled();
      expect(base.queryByRole("button", { name: /Download/ })).toBeNull();

      const large = within(row("Large v3"));
      expect(large.getByText("3.1 GB")).toBeVisible();
      expect(
        large.getByRole("button", { name: "Download Large v3" }),
      ).toBeEnabled();
      expect(large.queryByRole("radio")).toBeNull();
      expect(large.queryByRole("button", { name: /Delete/ })).toBeNull();
    });

    test("with nothing installed it says so, and every model can be downloaded", async () => {
      await openSettings({ installedModelIds: [] });

      // Said on the page, without opening anything.
      expect(screen.getByText(/No model is installed/)).toBeVisible();
      expect(picker()).toHaveTextContent("No model installed");

      await openMenu();
      expect(queryList("Installed models")).toBeNull();
      expect(screen.queryByRole("radio")).toBeNull();
      expect(names(downloadList())).toHaveLength(6);
    });

    test("Escape closes it and gives the focus back to the button", async () => {
      await openModels();

      await pressEscape();
      expect(queryList("Installed models")).toBeNull();
      expect(picker()).toHaveAttribute("aria-expanded", "false");
      expect(picker()).toHaveFocus();
      // Still Settings: Escape was the menu's.
      expect(
        screen.getByRole("heading", { name: "Settings" }),
      ).toBeInTheDocument();
    });

    test("a press anywhere else closes it, and so does the button", async () => {
      await openModels();
      fireEvent.pointerDown(screen.getByRole("heading", { name: "Settings" }));
      await advance();
      expect(queryList("Installed models")).toBeNull();

      await openMenu();
      // A press inside it does not.
      fireEvent.pointerDown(row("Large v3"));
      await advance();
      expect(installedList()).toBeInTheDocument();

      await openMenu();
      expect(queryList("Installed models")).toBeNull();
    });

    test("the arrow keys, Home and End move between the rows", async () => {
      await openSettings({ installedModelIds: ["base.en", "small.en"] });
      picker().focus();
      // Down on the closed button opens it.
      fireEvent.keyDown(picker(), { key: "ArrowDown" });
      await advance();
      expect(defaultRadio("Base (English)")).toHaveFocus();

      const press = (key: string) =>
        fireEvent.keyDown(document.activeElement!, { key });
      press("ArrowDown");
      expect(defaultRadio("Small (English)")).toHaveFocus();
      // On from the installed models to the ones to download.
      press("ArrowDown");
      expect(button("Download Tiny (English)")).toHaveFocus();
      press("End");
      expect(button("Download Large v3")).toHaveFocus();
      press("ArrowDown");
      expect(button("Download Large v3")).toHaveFocus();
      press("ArrowUp");
      expect(button("Download Large v3 Turbo")).toHaveFocus();
      press("Home");
      expect(defaultRadio("Base (English)")).toHaveFocus();
      // Moving does not choose: the default is as it was.
      expect(defaultRadio("Base (English)")).toBeChecked();
      expect((await savedSettings()).defaultModelId).toBe("base.en");
    });

    test("says it is loading only if the read drags on", async () => {
      renderSettings({ latencyMs: 2000 });
      await advance();

      // In the page from the start, and held back by `appears-late`.
      expect(screen.getByText("Loading settings…")).toHaveClass("appears-late");
      await advance(2000);
      expect(screen.queryByText("Loading settings…")).toBeNull();
      expect(picker()).toHaveTextContent("Base (English)");
    });

    test("a failed read is shown with Retry, which reads again", async () => {
      mockApi.mock.failNext("listModels", "The models folder is missing.");
      await openSettings();

      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not load the settings: The models folder is missing.",
      );
      await click("Retry");
      expect(screen.queryByRole("alert")).toBeNull();
      expect(picker()).toHaveTextContent("Base (English)");
    });
  });

  describe("downloading", () => {
    test("shows progress with received and total, then verifying, then the model is installed", async () => {
      await openModels();
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
      // Shown once: in the menu's row, not also under the Settings row.
      expect(screen.getAllByRole("progressbar")).toHaveLength(1);

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
      expect(picker()).toHaveTextContent("Base (English)");
      expect((await savedSettings()).defaultModelId).toBe("base.en");
    });

    test("before the backend answers, the row says it is starting and cannot be cancelled yet", async () => {
      await openSettings({ latencyMs: 300 });
      await advance(300);
      await openMenu();
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
      await openModels();
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
      await openModels({ download: { cancelMs: 8000 } });
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
      await openModels();
      mockApi.mock.failNextDownload("The connection was lost.");
      await click("Download Small (English)");
      await advance(DOWNLOAD_MS);

      const small = () => within(row("Small (English)"));
      expect(small().getByRole("alert")).toHaveTextContent(
        "Could not download Small (English): The connection was lost.",
      );
      expect(small().queryByRole("progressbar")).toBeNull();
      // Only that row is affected, and it is said once.
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
      await openModels();
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
      await openModels();
      mockApi.mock.failNext("downloadModel", "The models folder is read-only.");
      await click("Download Small (English)");

      expect(
        within(row("Small (English)")).getByRole("alert"),
      ).toHaveTextContent(
        "Could not download Small (English): The models folder is read-only.",
      );
    });

    test("with the menu closed, the download is under the Settings row, with Cancel", async () => {
      await openModels();
      await click("Download Small (English)");
      await advance(10 * 150);
      await pressEscape();

      expect(screen.queryByRole("listitem")).toBeNull();
      expect(progressBar("Small (English)")).toHaveAttribute(
        "value",
        "243807110",
      );
      expect(screen.getByText("244 MB of 488 MB")).toBeVisible();
      expect(screen.getAllByRole("progressbar")).toHaveLength(1);

      await click("Cancel downloading Small (English)");
      expect(screen.queryByRole("progressbar")).toBeNull();
      expect(queryButton(/Small \(English\)$/)).toBeNull();
      expect(picker()).toHaveTextContent("Base (English)");
    });

    test("with the menu closed, a failed download says why under the Settings row, with Retry", async () => {
      await openModels();
      mockApi.mock.failNextDownload("The connection was lost.");
      await click("Download Small (English)");
      await pressEscape();
      await advance(DOWNLOAD_MS);

      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not download Small (English): The connection was lost.",
      );
      expect(screen.queryByRole("progressbar")).toBeNull();

      await click("Retry downloading Small (English)");
      expect(screen.queryByRole("alert")).toBeNull();
      expect(progressBar("Small (English)")).toHaveAttribute("value", "0");

      await advance(DOWNLOAD_MS + VERIFY_MS);
      expect(screen.queryByRole("progressbar")).toBeNull();
      await openMenu();
      expect(names(installedList())).toEqual([
        "Base (English)",
        "Small (English)",
      ]);
    });

    test("progress carries on while Settings is closed, and is there on return", async () => {
      const downloadModel = vi.spyOn(mockApi, "downloadModel");
      renderShell();
      await advance();
      await click("Settings");
      await openMenu();
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
      // Further along than when it was left, not started over, and in sight
      // without opening the menu.
      expect(screen.getByText("244 MB of 488 MB")).toBeVisible();
      expect(progressBar("Small (English)")).toHaveAttribute(
        "value",
        "243807110",
      );
      expect(downloadModel).toHaveBeenCalledTimes(1);

      // And it can finish while the user is elsewhere.
      await click("Back");
      await advance(10 * 150 + VERIFY_MS);
      await click("Settings");
      expect(screen.queryByRole("progressbar")).toBeNull();
      await openMenu();
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
      await openMenu();
      await click("Download Small (English)");
      await advance(5 * 150);
      await click("Cancel downloading Small (English)");

      await click("Back");
      await advance(1000);
      await click("Settings");
      expect(screen.getByText("Cancelling…")).toBeVisible();
      expect(button("Cancel downloading Small (English)")).toBeDisabled();
    });

    test("two models download at the same time", async () => {
      await openModels();
      await click("Download Tiny (English)");
      await advance(10 * 150);
      await click("Download Large v3");
      await advance(5 * 150);

      expect(
        within(row("Tiny (English)")).getByText(/of 77\.7 MB$/),
      ).toBeVisible();
      expect(within(row("Large v3")).getByText(/of 3\.1 GB$/)).toBeVisible();
      expect(screen.getAllByRole("progressbar")).toHaveLength(2);

      // Closed, each has its own line under the row.
      await pressEscape();
      expect(progressBar("Tiny (English)")).toBeInTheDocument();
      expect(progressBar("Large v3")).toBeInTheDocument();

      await advance(DOWNLOAD_MS + VERIFY_MS);
      expect(screen.queryByRole("progressbar")).toBeNull();
      await openMenu();
      expect(names(installedList())).toEqual([
        "Tiny (English)",
        "Base (English)",
        "Large v3",
      ]);
    });
  });

  describe("the default model", () => {
    test("the first model installed becomes the default by itself", async () => {
      await openModels({ installedModelIds: [] });
      expect((await savedSettings()).defaultModelId).toBeNull();

      await click("Download Small (English)");
      await advance(DOWNLOAD_MS + VERIFY_MS);

      expect(names(installedList())).toEqual(["Small (English)"]);
      expect(defaultRadio("Small (English)")).toBeChecked();
      expect(picker()).toHaveTextContent("Small (English)");
      expect(screen.queryByText(/No model is installed/)).toBeNull();
      expect((await savedSettings()).defaultModelId).toBe("small.en");
    });

    test("choosing another installed model saves it as the default and closes the menu", async () => {
      await openModels({ installedModelIds: ["base.en", "small.en"] });
      expect(defaultRadio("Base (English)")).toBeChecked();

      fireEvent.click(defaultRadio("Small (English)"));
      // Shown at once, before the backend has answered.
      expect(picker()).toHaveTextContent("Small (English)");
      expect(queryList("Installed models")).toBeNull();
      expect(picker()).toHaveFocus();
      await advance();

      expect(picker()).toHaveTextContent("Small (English)");
      expect((await savedSettings()).defaultModelId).toBe("small.en");
      await openMenu();
      expect(defaultRadio("Small (English)")).toBeChecked();
      expect(defaultRadio("Small (English)")).toHaveFocus();
      expect(defaultRadio("Base (English)")).not.toBeChecked();
    });

    test("choosing the model already in use saves nothing", async () => {
      const updateSettings = vi.spyOn(mockApi, "updateSettings");
      await openModels({ installedModelIds: ["base.en", "small.en"] });

      fireEvent.click(defaultRadio("Base (English)"));
      await advance();
      expect(queryList("Installed models")).toBeNull();
      expect(picker()).toHaveTextContent("Base (English)");
      expect(updateSettings).not.toHaveBeenCalled();
    });

    test("a default that cannot be saved is said in place, and the old one stays", async () => {
      await openModels({ installedModelIds: ["base.en", "small.en"] });
      mockApi.mock.failNext("updateSettings", "The settings file is locked.");

      fireEvent.click(defaultRadio("Small (English)"));
      await advance();

      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not change the default model: The settings file is locked.",
      );
      expect(picker()).toHaveTextContent("Base (English)");
      expect((await savedSettings()).defaultModelId).toBe("base.en");

      // The next change clears the message.
      await openMenu();
      expect(defaultRadio("Base (English)")).toBeChecked();
      fireEvent.click(defaultRadio("Small (English)"));
      await advance();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(picker()).toHaveTextContent("Small (English)");
    });
  });

  describe("deleting", () => {
    test("Delete asks in the row first, and Keep leaves everything as it was", async () => {
      const confirm = vi.spyOn(window, "confirm");
      const deleteModel = vi.spyOn(mockApi, "deleteModel");
      await openModels({ installedModelIds: ["base.en", "small.en"] });

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

    test("Escape answers the question with no, and the menu stays open", async () => {
      await openModels({ installedModelIds: ["base.en", "small.en"] });
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
      await openModels({ installedModelIds: ["base.en", "small.en"] });
      await click("Delete Small (English)");
      fireEvent.click(
        within(row("Small (English)")).getByRole("button", { name: "Delete" }),
      );
      await advance();

      expect(names(installedList())).toEqual(["Base (English)"]);
      expect(names(downloadList())).toContain("Small (English)");
      expect(button("Download Small (English)")).toBeEnabled();
      // The focus stays in the menu, on the row the model now has.
      expect(button("Download Small (English)")).toHaveFocus();
      expect(
        (await mockApi.listModels()).find((m) => m.id === "small.en")
          ?.installed,
      ).toBe(false);
      expect(defaultRadio("Base (English)")).toBeChecked();
    });

    test("deleting the default model moves the default to another installed model", async () => {
      await openModels({
        installedModelIds: ["base.en", "small.en", "medium.en"],
        settings: { defaultModelId: "small.en" },
      });
      expect(defaultRadio("Small (English)")).toBeChecked();
      expect(picker()).toHaveTextContent("Small (English)");

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
      expect(defaultRadio("Medium (English)")).not.toBeChecked();
      expect(picker()).toHaveTextContent("Base (English)");
      expect((await savedSettings()).defaultModelId).toBe("base.en");
    });

    test("deleting the only model leaves no default", async () => {
      await openModels();

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
      expect(picker()).toHaveTextContent("No model installed");
      expect(screen.queryByRole("radio")).toBeNull();
      expect(names(downloadList())).toHaveLength(6);
      expect((await savedSettings()).defaultModelId).toBeNull();
    });

    test("a delete that fails says why on the row and keeps the model", async () => {
      await openModels({ installedModelIds: ["base.en", "small.en"] });
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

    test("while a recording runs, only the model it uses cannot be deleted", async () => {
      mockApi.mock.reset({ installedModelIds: ["base.en", "small.en"] });
      // The default, Base, is the one the recording uses.
      await mockApi.startMeeting();
      await openModels();

      expect(button("Delete Base (English)")).toBeDisabled();
      expect(button("Delete Small (English)")).toBeEnabled();
      expect(
        screen.getByText(
          "Base (English) is in use by the recording and cannot be deleted until it stops.",
        ),
      ).toBeVisible();
      // Everything else still works: the default, and downloads.
      expect(defaultRadio("Small (English)")).toBeEnabled();
      expect(button("Download Tiny (English)")).toBeEnabled();

      await click("Stop");
      await advance(600);
      expect(button("Delete Base (English)")).toBeEnabled();
      expect(screen.queryByText(/is in use by the recording/)).toBeNull();
    });

    test("a recording that starts with a model while its question is open withdraws it", async () => {
      const deleteModel = vi.spyOn(mockApi, "deleteModel");
      await openModels({ installedModelIds: ["base.en", "small.en"] });
      await click("Delete Small (English)");
      await click("Delete Base (English)");
      expect(
        screen.getByRole("group", { name: "Delete Base (English)?" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("group", { name: "Delete Small (English)?" }),
      ).toBeInTheDocument();

      // Started from the menu bar or the global shortcut, with Base.
      await mockApi.startMeeting();
      await advance();

      expect(
        screen.queryByRole("group", { name: "Delete Base (English)?" }),
      ).toBeNull();
      expect(button("Delete Base (English)")).toBeDisabled();
      // The other model's question is not the recording's business.
      expect(
        screen.getByRole("group", { name: "Delete Small (English)?" }),
      ).toBeInTheDocument();
      expect(deleteModel).not.toHaveBeenCalled();
    });
  });
});

describe("spellcheck", () => {
  test("is off at first, and the switch saves each change", async () => {
    renderSettings();
    const toggle = await screen.findByRole("switch", {
      name: "Check spelling",
    });
    expect(toggle).not.toBeChecked();

    await userEvent.click(toggle);
    await waitFor(async () =>
      expect((await savedSettings()).spellcheck).toBe(true),
    );
    expect(toggle).toBeChecked();

    await userEvent.click(toggle);
    await waitFor(async () =>
      expect((await savedSettings()).spellcheck).toBe(false),
    );
    expect(toggle).not.toBeChecked();
  });

  test("a save that fails says why and shows the saved value again", async () => {
    renderSettings();
    const toggle = await screen.findByRole("switch", {
      name: "Check spelling",
    });
    mockApi.mock.failNext("updateSettings", "The settings file is locked.");

    await userEvent.click(toggle);

    expect(
      await screen.findByText(
        "Could not change spellcheck: The settings file is locked.",
      ),
    ).toBeVisible();
    expect(toggle).not.toBeChecked();
    expect((await savedSettings()).spellcheck).toBe(false);
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
