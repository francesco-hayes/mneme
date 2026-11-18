import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { RecordingError } from "../lib/api";
import { mockApi } from "../lib/api/mock";
import type { Route } from "../state/navigation";
import { advance, renderShell } from "../test/render";

// The mock's recording clock runs on timers, so these tests use fake ones.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const MIC = "Mneme is not allowed to use the microphone.";

/** What the backend does when a start or stop from the menu bar fails. */
async function emit(action: RecordingError["action"], message: string) {
  act(() => mockApi.mock.emitRecordingError(action, message));
  await advance();
}
const alerts = () => screen.queryAllByRole("alert");
const click = async (name: string) => {
  fireEvent.click(screen.getByRole("button", { name }));
  await advance();
};
const heading = (name: string) => screen.getByRole("heading", { name });

async function renderOn(route: Route) {
  renderShell({ route });
  await advance();
}

test("nothing is shown until an error arrives", async () => {
  await renderOn({ name: "home" });

  expect(alerts()).toHaveLength(0);
  expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
});

test.each<[string, Route]>([
  ["Home", { name: "home" }],
  ["a meeting", { name: "meeting", meetingId: "mtg-standup" }],
  ["Settings", { name: "settings" }],
])("a start error is shown on %s", async (_screen, route) => {
  await renderOn(route);
  await emit("start", MIC);

  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(`Could not start recording: ${MIC}`);
});

test("the error stays while moving between screens", async () => {
  await renderOn({ name: "home" });
  await emit("start", MIC);

  await click("Settings");
  expect(heading("Settings")).toBeInTheDocument();
  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(MIC);
});

test("a stop error reads as a stop error, and outlives the recording's ticks", async () => {
  await renderOn({ name: "home" });
  await click("New meeting");
  await emit("stop", "The transcript could not be written.");

  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(
    "Could not stop recording: The transcript could not be written.",
  );

  // The same recording going on is not a recording starting.
  await advance(3000);
  expect(screen.getByRole("timer")).toHaveTextContent("0:03");
  expect(alerts()).toHaveLength(1);
});

test("Dismiss removes it", async () => {
  await renderOn({ name: "home" });
  await emit("start", MIC);

  await click("Dismiss");
  expect(alerts()).toHaveLength(0);
  expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
});

test("a recording starting removes it", async () => {
  await renderOn({ name: "home" });
  await emit("start", MIC);
  expect(alerts()).toHaveLength(1);

  // Started from outside the window, as the menu bar would.
  await act(async () => {
    await mockApi.startMeeting();
  });
  await advance();

  expect(screen.getByRole("region", { name: "Recording" })).toBeInTheDocument();
  expect(alerts()).toHaveLength(0);
});

test("a second error replaces the first", async () => {
  await renderOn({ name: "home" });
  await emit("start", MIC);
  await emit("start", "No transcription model is installed.");

  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(
    "Could not start recording: No transcription model is installed.",
  );
  expect(screen.queryByText(MIC, { exact: false })).toBeNull();
});

test("a start that fails inside the window is shown once, by Home", async () => {
  await renderOn({ name: "home" });
  mockApi.mock.failNext("startMeeting", MIC);

  await click("New meeting");
  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(`Could not start the meeting: ${MIC}`);
  expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
});

test("a stop that fails inside the window is shown once, by the recording bar", async () => {
  await renderOn({ name: "home" });
  await click("New meeting");
  mockApi.mock.failNext("stopMeeting", "The transcript could not be written.");

  await click("Stop");
  expect(alerts()).toHaveLength(1);
  expect(alerts()[0]).toHaveTextContent(
    "Could not stop: The transcript could not be written.",
  );
  expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
});
