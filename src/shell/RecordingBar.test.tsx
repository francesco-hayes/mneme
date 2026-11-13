import { fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { mockApi } from "../lib/api/mock";
import { useBeforeStop } from "../state/recording";
import { advance, renderShell, renderWithProviders } from "../test/render";
import { RecordingBar } from "./RecordingBar";

// The mock's recording clock and stop delay run on timers, so these tests use
// fake ones: `advance(3000)` is three seconds of recording.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const bar = () => screen.queryByRole("region", { name: "Recording" });
/** The title of the meeting on screen: a field in the top bar. */
const meetingTitle = () =>
  screen.getByRole<HTMLInputElement>("textbox", { name: "Meeting title" });
const button = (name: string) => screen.getByRole("button", { name });
const click = async (name: string) => {
  fireEvent.click(button(name));
  await advance();
};

/** Renders the shell on Home and starts a meeting from there. */
async function startRecording() {
  renderShell();
  await advance();
  await click("New meeting");
}

test("is hidden when idle", async () => {
  renderShell();
  await advance();

  expect(screen.getByRole("heading", { name: "Meetings" })).toBeInTheDocument();
  expect(bar()).toBeNull();
  expect(screen.queryByRole("timer")).toBeNull();
});

test("shows the indicator and a running timer while recording", async () => {
  await startRecording();

  const recording = bar();
  expect(recording).toBeInTheDocument();
  expect(within(recording!).getByRole("status")).toHaveTextContent("Recording");
  expect(within(recording!).getByRole("timer")).toHaveTextContent("0:00");

  await advance(3000);
  expect(screen.getByRole("timer")).toHaveTextContent("0:03");

  await advance(62_000);
  expect(screen.getByRole("timer")).toHaveTextContent("1:05");
  expect(button("Stop")).toBeEnabled();
});

test("on the live meeting there is no link to it; elsewhere there is", async () => {
  await startRecording();
  const title = meetingTitle().value;
  expect(title).toMatch(/^Meeting, /);

  // New meeting opened the live meeting.
  expect(screen.queryByRole("button", { name: "Open meeting" })).toBeNull();

  await click("Back");
  expect(screen.getByRole("heading", { name: "Meetings" })).toBeInTheDocument();
  expect(bar()).toBeInTheDocument();
  expect(button("Open meeting")).toBeInTheDocument();

  await click("Settings");
  expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
  expect(bar()).toBeInTheDocument();

  await click("Open meeting");
  expect(meetingTitle()).toHaveValue(title);
  expect(screen.queryByRole("button", { name: "Open meeting" })).toBeNull();
});

test("an older meeting is not the live one: the link stays", async () => {
  await startRecording();
  await click("Back");
  fireEvent.click(
    within(screen.getByRole("list", { name: "Meetings" })).getByRole("button", {
      name: /^Standup/,
    }),
  );
  await advance();

  expect(screen.getByRole("heading", { name: "Standup" })).toBeInTheDocument();
  expect(button("Open meeting")).toBeInTheDocument();
});

test("shows a stopping state, then goes away", async () => {
  await startRecording();
  await advance(5000);

  await click("Stop");
  const recording = bar();
  expect(within(recording!).getByRole("status")).toHaveTextContent("Stopping…");
  expect(within(recording!).getByRole("timer")).toHaveTextContent("0:05");
  expect(button("Stop")).toBeDisabled();

  // The mock's stop takes 600 ms.
  await advance(599);
  expect(bar()).toBeInTheDocument();
  await advance(1);
  expect(bar()).toBeNull();
});

test("a failed stop is shown in the bar and Stop can be pressed again", async () => {
  await startRecording();
  mockApi.mock.failNext("stopMeeting", "The transcript could not be written.");

  await click("Stop");
  expect(within(bar()!).getByRole("alert")).toHaveTextContent(
    "Could not stop: The transcript could not be written.",
  );
  expect(within(bar()!).getByRole("status")).toHaveTextContent("Recording");
  expect(button("Stop")).toBeEnabled();

  await click("Stop");
  expect(screen.queryByRole("alert")).toBeNull();
  await advance(600);
  expect(bar()).toBeNull();
});

test("picks up a recording that was already running", async () => {
  const meeting = await mockApi.startMeeting();
  await vi.advanceTimersByTimeAsync(4000);

  renderShell({ route: { name: "settings" } });
  await advance();

  expect(screen.getByRole("timer")).toHaveTextContent("0:04");
  await click("Open meeting");
  expect(meetingTitle()).toHaveValue(meeting.title);
});

test("follows a recording started and stopped outside the window", async () => {
  renderShell();
  await advance();
  expect(bar()).toBeNull();

  // As the menu bar icon or the global shortcut will do.
  await mockApi.startMeeting();
  await advance(2000);
  expect(screen.getByRole("timer")).toHaveTextContent("0:02");

  void mockApi.stopMeeting();
  await advance();
  expect(screen.getByRole("status")).toHaveTextContent("Stopping…");
  await advance(600);
  expect(bar()).toBeNull();
});

test("before-stop handlers finish before the meeting is stopped", async () => {
  const order: string[] = [];
  let finishSave = () => {};
  function SavesBeforeStop() {
    useBeforeStop(async () => {
      order.push(`save while ${(await mockApi.getRecordingState()).status}`);
      await new Promise<void>((resolve) => {
        finishSave = resolve;
      });
      order.push("saved");
    });
    return null;
  }
  await mockApi.startMeeting();
  renderWithProviders(
    <>
      <SavesBeforeStop />
      <RecordingBar />
    </>,
  );
  await advance();

  await click("Stop");
  // Waiting for the handler: shown as stopping, but still recording.
  expect(order).toEqual(["save while recording"]);
  expect(screen.getByRole("status")).toHaveTextContent("Stopping…");
  expect(button("Stop")).toBeDisabled();
  expect((await mockApi.getRecordingState()).status).toBe("recording");

  finishSave();
  await advance();
  expect(order).toEqual(["save while recording", "saved"]);
  expect((await mockApi.getRecordingState()).status).toBe("stopping");
  await advance(600);
  expect(bar()).toBeNull();
});

test("a before-stop handler that fails does not block the stop", async () => {
  function FailsBeforeStop() {
    useBeforeStop(() => {
      throw new Error("Disk full.");
    });
    return null;
  }
  await mockApi.startMeeting();
  renderWithProviders(
    <>
      <FailsBeforeStop />
      <RecordingBar />
    </>,
  );
  await advance();

  await click("Stop");
  await advance(600);
  expect(bar()).toBeNull();
  expect((await mockApi.getRecordingState()).status).toBe("idle");
});
