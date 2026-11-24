import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { MeetingSummary } from "../lib/api";
import { mockApi, type MockOptions, mockScenarios } from "../lib/api/mock";
import type { MockMeetingSeed } from "../lib/api/mockSeed";
import { RecordingBar } from "../shell/RecordingBar";
import { advance, renderShell, renderWithProviders } from "../test/render";
import { HomeScreen } from "./HomeScreen";

// A fixed clock, locale and time zone, so "Today" and "2:30 PM" do not depend
// on when or where the tests run: Thursday, November 12, 2026, 3:00 PM in
// New York (UTC-5).
const NOW = new Date("2026-11-12T20:00:00Z");
const now = () => NOW;

function meeting(
  seed: Pick<MockMeetingSeed, "id" | "title" | "startedAt"> &
    Partial<MockMeetingSeed>,
): MockMeetingSeed {
  return {
    durationSecs: 60,
    status: "complete",
    notes: "",
    transcript: [],
    ...seed,
  };
}

/** Deliberately not in date order. */
const MEETINGS: MockMeetingSeed[] = [
  meeting({
    id: "planning",
    title: "Quarterly planning",
    startedAt: "2026-10-27T14:00:00Z",
    durationSecs: 65 * 60,
  }),
  meeting({
    id: "standup",
    title: "Standup",
    startedAt: "2026-11-12T19:30:00Z",
    durationSecs: 32 * 60,
  }),
  meeting({
    id: "kickoff",
    title: "Kickoff",
    startedAt: "2025-12-03T15:00:00Z",
    durationSecs: null,
  }),
  meeting({
    id: "review",
    title: "Design review",
    startedAt: "2026-11-11T16:00:00Z",
    durationSecs: 21 * 60,
    status: "interrupted",
  }),
];

/** The mock on the fixed clock, holding `MEETINGS` unless told otherwise. */
function resetMock(options: MockOptions = {}) {
  mockApi.mock.reset({ now, meetings: MEETINGS, ...options });
}

/** Home with fixed dates, and the recording bar under it as in the shell. */
function renderHome() {
  return renderWithProviders(
    <>
      <HomeScreen now={now} locale="en-US" timeZone="America/New_York" />
      <RecordingBar />
    </>,
  );
}

const list = () => screen.getByRole("list", { name: "Meetings" });
const findList = () => screen.findByRole("list", { name: "Meetings" });
const rows = () => within(list()).getAllByRole("listitem");
const rowTexts = () => rows().map((row) => row.textContent);
const row = (title: RegExp) =>
  within(list()).getByRole("button", { name: title });
const topBar = () => within(screen.getByRole("banner"));
const content = () => within(screen.getByRole("main"));
/** The New meeting button in the top bar: the one that is always there. */
const newMeeting = () => topBar().getByRole("button", { name: "New meeting" });
/** The same button while a start loads its model. */
const startingButton = () =>
  topBar().getByRole("button", { name: "Starting…" });
const heading = (name: string | RegExp) =>
  screen.findByRole("heading", { name });

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the list", () => {
  beforeEach(() => resetMock());

  test("shows title, date and duration, newest first", async () => {
    renderHome();
    await findList();

    expect(rowTexts()).toEqual([
      "StandupToday 2:30 PM32 min",
      "Design reviewYesterday 11:00 AM · Interrupted21 min",
      "Quarterly planningOct 271 hr 5 min",
      // No duration: nothing is shown in its place.
      "KickoffDec 3, 2025",
    ]);
  });

  test("orders by start time itself, whatever order the backend returns", async () => {
    const summaries = MEETINGS.map(
      ({ id, title, startedAt, durationSecs, status }): MeetingSummary => ({
        id,
        title,
        startedAt,
        durationSecs,
        status,
        collection: null,
      }),
    );
    // The mock sorts; hand the screen an unsorted list directly.
    vi.spyOn(mockApi, "listMeetings").mockResolvedValue(summaries);
    renderHome();
    await findList();

    expect(rowTexts().map((text) => text.match(/^[A-Z][a-z]+/)?.[0])).toEqual([
      "Standup",
      "Design",
      "Quarterly",
      "Kickoff",
    ]);
  });

  test("the date carries the full date and time for a tooltip", async () => {
    renderHome();
    await findList();

    const date = within(row(/^Quarterly planning/)).getByText("Oct 27");
    expect(date).toHaveAttribute("datetime", "2026-10-27T14:00:00Z");
    expect(date).toHaveAttribute(
      "title",
      "Tuesday, October 27, 2026 at 10:00 AM",
    );
  });

  test("marks an interrupted meeting, and only that one", async () => {
    renderHome();
    await findList();

    expect(
      within(row(/^Design review/)).getByText("Interrupted"),
    ).toBeVisible();
    expect(within(list()).getAllByText("Interrupted")).toHaveLength(1);
  });

  test("a meeting being recorded shows that it is live, with no duration", async () => {
    await mockApi.startMeeting();
    renderHome();
    await findList();

    expect(rows()).toHaveLength(5);
    // It started at the mock's "now", so it is first.
    expect(rows()[0]).toHaveTextContent(/Today 3:00 PMRecording$/);
    expect(within(list()).getAllByText("Recording")).toHaveLength(1);
  });

  test("a meeting whose date cannot be read is still listed, last", async () => {
    mockApi.mock.putMeeting(
      meeting({ id: "odd", title: "Odd one", startedAt: "soon" }),
    );
    renderHome();
    await findList();

    expect(rowTexts().at(-1)).toBe("Odd one1 min");
  });

  test("clicking a row opens that meeting", async () => {
    const user = userEvent.setup();
    renderShell();

    await user.click(within(await findList()).getByText("Design review"));

    expect(await heading("Design review")).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Meetings" })).toBeNull();
    expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
  });
});

describe("the ⋯ menu", () => {
  beforeEach(() => resetMock());

  const menuButton = (title: string) =>
    screen.getByRole("button", { name: `Options for ${title}` });
  const menuItem = (name: string | RegExp) =>
    screen.getByRole("menuitem", { name });

  test("every row has one, closed, holding View, Move to… and Delete", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    for (const title of ["Standup", "Design review", "Kickoff"]) {
      expect(menuButton(title)).toHaveAttribute("aria-expanded", "false");
    }
    expect(screen.queryByRole("menu")).toBeNull();

    await user.click(menuButton("Standup"));
    expect(menuButton("Standup")).toHaveAttribute("aria-expanded", "true");
    const items = within(screen.getByRole("menu")).getAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      "View",
      "Move to…",
      "Delete…",
    ]);
    expect(items[0]).toHaveFocus();
  });

  test("View opens the meeting", async () => {
    const user = userEvent.setup();
    renderShell();
    await findList();

    await user.click(menuButton("Design review"));
    await user.click(menuItem("View"));

    expect(await heading("Design review")).toBeInTheDocument();
  });

  const dialog = () => screen.getByRole("alertdialog");

  test("Delete opens a dialog; Cancel leaves the meeting where it is", async () => {
    const user = userEvent.setup();
    const deleteMeeting = vi.spyOn(mockApi, "deleteMeeting");
    renderHome();
    await findList();

    await user.click(menuButton("Standup"));
    await user.click(menuItem("Delete…"));

    expect(dialog()).toHaveAccessibleName("Move “Standup” to the Trash?");
    expect(dialog()).toHaveAccessibleDescription(
      "Its notes and transcript go with it. Put Back in Finder restores it.",
    );
    expect(dialog()).toHaveAttribute("aria-modal", "true");
    const cancel = within(dialog()).getByRole("button", { name: "Cancel" });
    expect(cancel).toHaveFocus();

    await user.click(cancel);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(menuButton("Standup")).toHaveFocus();
    expect(deleteMeeting).not.toHaveBeenCalled();
    expect(rows()).toHaveLength(4);
  });

  test("Escape closes the dialog; Tab stays inside it", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(menuButton("Standup"));
    await user.click(menuItem("Delete…"));
    const cancel = within(dialog()).getByRole("button", { name: "Cancel" });
    const confirm = within(dialog()).getByRole("button", {
      name: "Move to Trash",
    });

    await user.tab();
    expect(confirm).toHaveFocus();
    await user.tab();
    expect(cancel).toHaveFocus();
    await user.tab({ shift: true });
    expect(confirm).toHaveFocus();
    // The list's arrow keys do not reach the rows behind it.
    await user.keyboard("{ArrowDown}");
    expect(confirm).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(menuButton("Standup")).toHaveFocus();
    expect(rows()).toHaveLength(4);
  });

  test("Move to Trash deletes the meeting and the list updates", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(menuButton("Standup"));
    await user.click(menuItem("Delete…"));
    await user.click(screen.getByRole("button", { name: "Move to Trash" }));

    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(rowTexts().join()).not.toContain("Standup");
    expect((await mockApi.listMeetings()).map((m) => m.id)).not.toContain(
      "standup",
    );
  });

  test("a delete that fails says why in the dialog, which stays open", async () => {
    const user = userEvent.setup();
    mockApi.mock.failNext("deleteMeeting", "The Trash is not available.");
    renderHome();
    await findList();

    await user.click(menuButton("Kickoff"));
    await user.click(menuItem("Delete…"));
    await user.click(screen.getByRole("button", { name: "Move to Trash" }));

    expect(await within(dialog()).findByRole("alert")).toHaveTextContent(
      "Could not delete the meeting: The Trash is not available.",
    );
    expect(rows()).toHaveLength(4);

    // Trying again works.
    await user.click(screen.getByRole("button", { name: "Move to Trash" }));
    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  test("Delete is disabled on the meeting being recorded", async () => {
    const user = userEvent.setup();
    resetMock({
      meetings: [
        meeting({
          id: "live",
          title: "Live one",
          startedAt: "2026-11-12T19:55:00Z",
          durationSecs: null,
          status: "recording",
        }),
      ],
    });
    renderHome();
    await findList();

    await user.click(menuButton("Live one"));
    expect(menuItem("Delete…")).toBeDisabled();
    expect(menuItem("Delete…")).toHaveAttribute(
      "title",
      "Stop the recording before deleting this meeting.",
    );
  });

  test("the keyboard moves through the menu and Escape closes it", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    menuButton("Standup").focus();
    await user.keyboard("{ArrowDown}");
    expect(menuItem("View")).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(menuItem("Move to…")).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(menuItem("Delete…")).toHaveFocus();
    // Stays in the menu: the list does not move to the next row.
    await user.keyboard("{ArrowDown}");
    expect(menuItem("Delete…")).toHaveFocus();
    await user.keyboard("{ArrowUp}");
    expect(menuItem("Move to…")).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    expect(menuButton("Standup")).toHaveFocus();
  });

  test("a click elsewhere closes it", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(menuButton("Standup"));
    expect(screen.getByRole("menu")).toBeInTheDocument();
    await user.click(screen.getByRole("heading", { name: "Meetings" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("states", () => {
  beforeEach(() => resetMock());

  test("says it is loading until the list arrives", async () => {
    vi.useFakeTimers();
    try {
      resetMock({ latencyMs: 250 });
      renderHome();
      await advance(249);

      expect(screen.getByRole("status")).toHaveTextContent("Loading meetings…");
      expect(screen.queryByRole("list")).toBeNull();
      expect(screen.queryByText("No meetings yet")).toBeNull();

      await advance(1);
      expect(screen.queryByRole("status")).toBeNull();
      expect(rows()).toHaveLength(4);
    } finally {
      vi.useRealTimers();
    }
  });

  test("with no meetings, says so and offers New meeting", async () => {
    const user = userEvent.setup();
    resetMock(mockScenarios.empty);
    renderShell();

    expect(await heading("No meetings yet")).toBeInTheDocument();
    expect(screen.queryByRole("list")).toBeNull();
    const start = content().getByRole("button", { name: "New meeting" });
    expect(start).toBeEnabled();

    await user.click(start);
    expect(await heading(/^Meeting, /)).toBeInTheDocument();
    expect((await mockApi.getRecordingState()).status).toBe("recording");
  });

  test("shows why the list could not be loaded, and Retry loads it", async () => {
    const user = userEvent.setup();
    mockApi.mock.failNext("listMeetings", "The meetings folder is missing.");
    renderHome();

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load meetings: The meetings folder is missing.",
    );
    expect(screen.queryByRole("list")).toBeNull();
    expect(screen.queryByText("No meetings yet")).toBeNull();
    // Starting a meeting does not need the list.
    expect(newMeeting()).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(await findList()).toBeInTheDocument();
    expect(rows()).toHaveLength(4);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("New meeting", () => {
  beforeEach(() => resetMock());

  test("starts a recording and opens the live meeting", async () => {
    const user = userEvent.setup();
    renderShell();
    await findList();

    await user.click(newMeeting());

    // The meeting's title is a field in the top bar.
    const title = await screen.findByRole<HTMLInputElement>("textbox", {
      name: "Meeting title",
    });
    expect(title.value).toMatch(/^Meeting, /);
    const state = await mockApi.getRecordingState();
    expect(state.status).toBe("recording");
    expect(title).toHaveValue(
      (await mockApi.getMeeting(state.meetingId!)).title,
    );
    // On the live meeting the bar has no link to it: this is that meeting.
    expect(screen.getByRole("region", { name: "Recording" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Open meeting" })).toBeNull();
  });

  test("is disabled without a model, with the reason and a link to Settings", async () => {
    const user = userEvent.setup();
    mockApi.mock.setInstalledModels([]);
    renderShell();
    await findList();

    await waitFor(() => expect(newMeeting()).toBeDisabled());
    expect(
      content().getByText(/Mneme needs a transcription model/),
    ).toBeVisible();
    // The list is still there to read.
    expect(rows()).toHaveLength(4);

    await user.click(newMeeting());
    expect((await mockApi.getRecordingState()).status).toBe("idle");

    await user.click(
      content().getByRole("button", { name: "Download one in Settings" }),
    );
    expect(await heading("Settings")).toBeInTheDocument();
  });

  test("without a model and without meetings, the empty state explains it", async () => {
    resetMock({ meetings: [], installedModelIds: [] });
    renderShell();
    await heading("No meetings yet");

    await waitFor(() => expect(newMeeting()).toBeDisabled());
    expect(
      content().getByRole("button", { name: "New meeting" }),
    ).toBeDisabled();
    expect(
      content().getByText(/Mneme needs a transcription model/),
    ).toBeVisible();
    expect(
      content().getByRole("button", { name: "Download one in Settings" }),
    ).toBeEnabled();
  });

  test("is enabled with a model, and says nothing about models", async () => {
    renderHome();
    await findList();
    // Give the model list time to arrive: it must not turn the button off.
    await mockApi.listModels();

    expect(newMeeting()).toBeEnabled();
    expect(screen.queryByText(/transcription model/)).toBeNull();
  });

  test("stays usable when the model list cannot be read", async () => {
    mockApi.mock.failNext("listModels", "The models folder is missing.");
    renderHome();
    await findList();

    expect(newMeeting()).toBeEnabled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("is disabled while a recording is running; the bar leads back to it", async () => {
    const user = userEvent.setup();
    const live = await mockApi.startMeeting();
    renderShell();
    await findList();

    await waitFor(() => expect(newMeeting()).toBeDisabled());
    expect(screen.queryByText(/transcription model/)).toBeNull();

    await user.click(screen.getByRole("button", { name: "Open meeting" }));
    expect(await heading(live.title)).toBeInTheDocument();
  });

  test("shows a start error in place and stays on Home", async () => {
    const user = userEvent.setup();
    mockApi.mock.setPermission("microphone", "denied");
    renderShell();
    await findList();

    await user.click(newMeeting());

    const error = await screen.findByRole("alert");
    expect(error).toHaveTextContent(
      "Could not start the meeting: Microphone access is denied.",
    );
    expect(content().getByRole("alert")).toBe(error);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("heading", { name: "Meetings" })).toBeVisible();
    expect(rows()).toHaveLength(4);
    expect(newMeeting()).toBeEnabled();
    expect((await mockApi.getRecordingState()).status).toBe("idle");

    // Once the cause is fixed, the next try works and the error is gone.
    mockApi.mock.setPermission("microphone", "granted");
    await user.click(newMeeting());
    expect(await heading(/^Meeting, /)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("a second press while the first is starting does not start twice", async () => {
    vi.useFakeTimers();
    try {
      resetMock({ latencyMs: 100 });
      const start = vi.spyOn(mockApi, "startMeeting");
      renderShell();
      await advance(100);

      fireEvent.click(newMeeting());
      await advance();
      expect(startingButton()).toBeDisabled();
      fireEvent.keyDown(window, { key: "n", metaKey: true });
      // The start settles, then the meeting screen it opens loads.
      await advance(100);
      await advance(100);

      expect(start).toHaveBeenCalledOnce();
      expect(screen.queryByRole("alert")).toBeNull();
      expect(
        screen.getByRole("heading", { level: 1, name: /^Meeting, / }),
      ).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  test("is disabled while a start from the menu bar loads its model", async () => {
    vi.useFakeTimers();
    try {
      const start = vi.spyOn(mockApi, "startMeeting");
      renderHome();
      await advance();
      expect(newMeeting()).toBeEnabled();

      // As the menu bar or the shortcut does: the model takes a while.
      const outside = mockApi.mock.startFromOutside(2000);
      await advance();
      expect(startingButton()).toBeDisabled();
      expect(startingButton()).toHaveAttribute(
        "title",
        "A meeting is starting.",
      );
      fireEvent.keyDown(window, { key: "n", metaKey: true });
      await advance();
      expect(start).not.toHaveBeenCalled();
      expect(screen.queryByRole("alert")).toBeNull();

      // Recording now: still disabled, for the usual reason, and the bar shows.
      await advance(2000);
      await outside;
      expect(newMeeting()).toBeDisabled();
      expect(screen.getByRole("region", { name: "Recording" })).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  test("comes back when a start from the menu bar fails", async () => {
    vi.useFakeTimers();
    try {
      renderHome();
      await advance();
      mockApi.mock.failNext(
        "startMeeting",
        "The transcription model could not be loaded.",
      );

      const outside = mockApi.mock.startFromOutside(1000);
      await advance();
      expect(startingButton()).toBeDisabled();
      await advance(1000);
      await outside;

      expect(newMeeting()).toBeEnabled();
      expect((await mockApi.getRecordingState()).starting).toBe(false);
      fireEvent.click(newMeeting());
      await advance();
      expect((await mockApi.getRecordingState()).status).toBe("recording");
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("refreshing", () => {
  beforeEach(() => resetMock());

  test("reads the list again when coming back to Home", async () => {
    const user = userEvent.setup();
    renderShell();
    await user.click(within(await findList()).getByText("Standup"));
    await heading("Standup");

    await mockApi.renameMeeting("standup", "Weekly sync");
    await user.click(screen.getByRole("button", { name: "Back" }));

    expect(
      await within(await findList()).findByText("Weekly sync"),
    ).toBeVisible();
    expect(within(list()).queryByText("Standup")).toBeNull();
  });

  test("reads the list again when the window gets focus back", async () => {
    renderHome();
    await findList();

    mockApi.mock.putMeeting(
      meeting({
        id: "added",
        title: "Added elsewhere",
        startedAt: "2026-11-12T19:45:00Z",
      }),
    );
    fireEvent.focus(window);

    expect(await within(list()).findByText("Added elsewhere")).toBeVisible();
    expect(rows()).toHaveLength(5);
  });

  describe("with a recording", () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    test("when the recording stops, its row gets a duration and New meeting comes back", async () => {
      await mockApi.startMeeting();
      renderHome();
      await advance();
      expect(rows()[0]).toHaveTextContent(/Recording$/);
      expect(newMeeting()).toBeDisabled();

      await advance(125_000);
      fireEvent.click(screen.getByRole("button", { name: "Stop" }));
      await advance();
      // Stopping: still the live meeting, still no second recording.
      expect(rows()[0]).toHaveTextContent(/Recording$/);
      expect(newMeeting()).toBeDisabled();

      await advance(600);
      expect(rows()).toHaveLength(5);
      expect(rows()[0]).toHaveTextContent(/Today 3:00 PM2 min$/);
      expect(within(list()).queryByText("Recording")).toBeNull();
      expect(newMeeting()).toBeEnabled();
    });

    test("a recording started outside the window appears in the list", async () => {
      renderHome();
      await advance();
      expect(rows()).toHaveLength(4);

      // As the menu bar icon or the global shortcut will do.
      await mockApi.startMeeting();
      await advance();

      expect(rows()).toHaveLength(5);
      expect(rows()[0]).toHaveTextContent(/Recording$/);
      expect(newMeeting()).toBeDisabled();
    });

    test("the timer ticking does not read the list again", async () => {
      await mockApi.startMeeting();
      const listMeetings = vi.spyOn(mockApi, "listMeetings");
      renderHome();
      await advance();
      const reads = listMeetings.mock.calls.length;

      await advance(30_000);

      expect(screen.getByRole("timer")).toHaveTextContent("0:30");
      expect(listMeetings).toHaveBeenCalledTimes(reads);
    });
  });
});

describe("keyboard", () => {
  beforeEach(() => resetMock());

  test("down and up move through the rows, stopping at the ends", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();
    expect(document.body).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(row(/^Standup/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(row(/^Design review/)).toHaveFocus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(row(/^Kickoff/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(row(/^Kickoff/)).toHaveFocus();

    await user.keyboard("{ArrowUp}");
    expect(row(/^Quarterly planning/)).toHaveFocus();
    await user.keyboard("{ArrowUp}{ArrowUp}{ArrowUp}");
    expect(row(/^Standup/)).toHaveFocus();
  });

  test("up with nothing focused goes to the last row", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.keyboard("{ArrowUp}");
    expect(row(/^Kickoff/)).toHaveFocus();
  });

  test("Enter opens the focused row", async () => {
    const user = userEvent.setup();
    renderShell();
    await findList();

    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");

    expect(await heading("Design review")).toBeInTheDocument();
  });

  test("arrow keys with a modifier are left alone", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(document.body).toHaveFocus();
  });

  test("Cmd-N starts a new meeting", async () => {
    const user = userEvent.setup();
    renderShell();
    await findList();

    await user.keyboard("{Meta>}n{/Meta}");

    expect(await heading(/^Meeting, /)).toBeInTheDocument();
    expect((await mockApi.getRecordingState()).status).toBe("recording");
  });

  test("Cmd-N does nothing when New meeting is disabled", async () => {
    const user = userEvent.setup();
    const start = vi.spyOn(mockApi, "startMeeting");
    mockApi.mock.setInstalledModels([]);
    renderShell();
    await findList();
    await waitFor(() => expect(newMeeting()).toBeDisabled());

    await user.keyboard("{Meta>}n{/Meta}");

    expect(start).not.toHaveBeenCalled();
    expect(screen.getByRole("heading", { name: "Meetings" })).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("Cmd-N belongs to Home: it does nothing on another screen", async () => {
    const user = userEvent.setup();
    const start = vi.spyOn(mockApi, "startMeeting");
    renderShell({ route: { name: "settings" } });
    await heading("Settings");

    await user.keyboard("{Meta>}n{/Meta}");

    expect(start).not.toHaveBeenCalled();
  });
});

describe("folders", () => {
  beforeEach(() =>
    resetMock({
      meetings: MEETINGS.map((seed) =>
        seed.id === "standup" || seed.id === "planning"
          ? { ...seed, collection: "Acme" }
          : seed,
      ),
      collections: ["Hiring"],
    }),
  );

  const folderBar = () =>
    within(screen.getByRole("navigation", { name: "Folders" }));
  const folder = (name: string) => folderBar().getByRole("button", { name });

  test("All lists every meeting and names each one's folder", async () => {
    renderHome();
    await findList();

    expect(
      folderBar()
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["All", "Unfiled", "Acme", "Hiring", "New folder…"]);
    expect(folder("All")).toHaveAttribute("aria-pressed", "true");
    expect(rowTexts()).toEqual([
      "StandupToday 2:30 PM · Acme32 min",
      "Design reviewYesterday 11:00 AM · Interrupted21 min",
      "Quarterly planningOct 27 · Acme1 hr 5 min",
      "KickoffDec 3, 2025",
    ]);
  });

  test("a folder narrows the list, and New meeting starts there", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(folder("Acme"));

    expect(await heading("Acme")).toBeInTheDocument();
    expect(folder("Acme")).toHaveAttribute("aria-pressed", "true");
    expect(rowTexts()).toEqual([
      "StandupToday 2:30 PM32 min",
      "Quarterly planningOct 271 hr 5 min",
    ]);

    await user.click(newMeeting());
    await waitFor(async () => {
      const live = (await mockApi.listMeetings()).find(
        (m) => m.status === "recording",
      );
      expect(live?.collection).toBe("Acme");
    });
  });

  test("Unfiled and an empty folder", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(folder("Unfiled"));
    expect(rowTexts()).toEqual([
      "Design reviewYesterday 11:00 AM · Interrupted21 min",
      "KickoffDec 3, 2025",
    ]);

    await user.click(folder("Hiring"));
    expect(
      await content().findByText("No meetings in Hiring"),
    ).toBeInTheDocument();
  });

  test("with no meetings yet, a folder can still be made", async () => {
    resetMock({ meetings: [], collections: [] });
    const user = userEvent.setup();
    renderHome();

    expect(await content().findByText("No meetings yet")).toBeInTheDocument();
    await user.click(folder("New folder…"));
    await user.type(
      screen.getByRole("textbox", { name: "Name" }),
      "Acme{Enter}",
    );

    expect(await heading("Acme")).toBeInTheDocument();
    expect(
      await content().findByText("No meetings in Acme"),
    ).toBeInTheDocument();
  });

  test("New folder… makes one and shows it", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(folder("New folder…"));
    const field = screen.getByRole("textbox", { name: "Name" });
    expect(field).toHaveFocus();
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
    await user.type(field, "Board{Enter}");

    expect(await heading("Board")).toBeInTheDocument();
    expect(folder("Board")).toHaveAttribute("aria-pressed", "true");
    expect(await mockApi.listCollections()).toContain("Board");
  });

  test("a taken name keeps the dialog open with the reason", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(folder("New folder…"));
    await user.type(screen.getByRole("textbox", { name: "Name" }), "acme");
    await user.click(screen.getByRole("button", { name: "Create" }));

    expect(await screen.findByText(/already exists/)).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  test("Move to… moves a meeting into a folder", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();

    await user.click(
      within(list()).getByRole("button", { name: "Options for Kickoff" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Move to…" }));
    const dialog = within(screen.getByRole("alertdialog"));
    expect(dialog.getByRole("radio", { name: "No folder" })).toBeChecked();
    expect(dialog.getByRole("button", { name: "Move" })).toBeDisabled();
    await user.click(dialog.getByRole("radio", { name: "Hiring" }));
    await user.click(dialog.getByRole("button", { name: "Move" }));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect((await mockApi.getMeeting("kickoff")).collection).toBe("Hiring");
    await waitFor(() =>
      expect(rowTexts()).toContain("KickoffDec 3, 2025 · Hiring"),
    );
  });

  test("a folder's menu renames it and deletes it", async () => {
    const user = userEvent.setup();
    renderHome();
    await findList();
    await user.click(folder("Acme"));

    await user.click(
      topBar().getByRole("button", { name: "Options for the folder Acme" }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Rename…" }));
    const field = screen.getByRole("textbox", { name: "Name" });
    expect(field).toHaveValue("Acme");
    await user.clear(field);
    await user.type(field, "Acme Corp{Enter}");
    expect(await heading("Acme Corp")).toBeInTheDocument();
    expect((await mockApi.getMeeting("standup")).collection).toBe("Acme Corp");

    await user.click(
      topBar().getByRole("button", {
        name: "Options for the folder Acme Corp",
      }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Delete…" }));
    expect(
      screen.getByText(/Its 2 meetings, with notes and transcripts/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Move to Trash" }));

    expect(await heading("Meetings")).toBeInTheDocument();
    expect(await mockApi.listCollections()).toEqual(["Hiring"]);
  });
});
