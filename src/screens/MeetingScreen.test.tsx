import { fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { mockApi } from "../lib/api/mock";
import type { MockMeetingSeed } from "../lib/api/mockSeed";
import {
  blurNotes,
  notesEditor,
  notesHaveFocus,
  notesText,
  pressInNotes,
  queryNotesEditor,
  typeInNotes,
} from "../test/editor";
import { advance, renderShell } from "../test/render";

// Autosave, the mock's recording clock and its stop delay all run on timers,
// so these tests use fake ones: `advance(800)` is 800 ms of the user not typing.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/** How long typing has to pause before notes are saved. */
const PAUSE = 800;

const NOTES = "# Retro\n\n- Export work is **merged**\n";

function seedMeeting(seed: Partial<MockMeetingSeed> = {}) {
  mockApi.mock.putMeeting({
    id: "retro",
    title: "Retro",
    startedAt: "2026-11-12T15:00:00Z",
    durationSecs: 600,
    status: "complete",
    notes: NOTES,
    transcript: [{ t: "00:00:05", speaker: "Them", text: "Can you hear me?" }],
    ...seed,
  });
}

const button = (name: string | RegExp) => screen.getByRole("button", { name });
const click = async (name: string | RegExp) => {
  fireEvent.click(button(name));
  await advance();
};
const titleField = () =>
  screen.getByRole<HTMLInputElement>("textbox", { name: "Meeting title" });
const recordingBar = () => screen.queryByRole("region", { name: "Recording" });

/** Opens the seeded meeting straight away: there is no screen to go back to. */
async function openMeeting(meetingId = "retro") {
  const view = renderShell({ route: { name: "meeting", meetingId } });
  await advance();
  return view;
}

/** Opens the seeded meeting from Home, so Back leads somewhere. */
async function openMeetingFromHome(title = /^Retro/) {
  renderShell();
  await advance();
  await click(title);
}

/** Starts a recording from Home, which opens the new meeting. */
async function startMeeting() {
  renderShell();
  await advance();
  await click("New meeting");
}

/** The id of the meeting being recorded. */
async function liveMeetingId() {
  const { meetingId } = await mockApi.getRecordingState();
  if (meetingId === null) throw new Error("Nothing is being recorded.");
  return meetingId;
}

const saveNotesSpy = () => vi.spyOn(mockApi, "saveNotes");
const savedNotes = async (meetingId = "retro") =>
  (await mockApi.getMeeting(meetingId)).notes;

/** A promise settled by hand, to hold a save in flight. */
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("the editor", () => {
  test("opens with the meeting's notes and saves nothing until they change", async () => {
    seedMeeting();
    const saveNotes = saveNotesSpy();
    await openMeeting();

    expect(notesText()).toBe(NOTES);

    blurNotes();
    await advance(60_000);
    expect(saveNotes).not.toHaveBeenCalled();
  });

  test("Enter continues a list and Enter on an empty item ends it", async () => {
    seedMeeting({ notes: "" });
    await openMeeting();

    typeInNotes("- first");
    pressInNotes("Enter");
    expect(notesText()).toBe("- first\n- ");

    typeInNotes("second");
    pressInNotes("Enter");
    expect(notesText()).toBe("- first\n- second\n- ");
    pressInNotes("Enter");
    expect(notesText()).toBe("- first\n- second\n");
  });

  test("Tab indents a list item and Shift-Tab brings it back", async () => {
    seedMeeting({ notes: "- one\n- two" });
    await openMeeting();
    const view = notesEditor();
    view.dispatch({ selection: { anchor: view.state.doc.length } });

    pressInNotes("Tab");
    expect(notesText()).toBe("- one\n  - two");
    pressInNotes("Tab", { shiftKey: true });
    expect(notesText()).toBe("- one\n- two");
  });

  test("a meeting that cannot be opened shows why, and Retry opens it", async () => {
    seedMeeting();
    mockApi.mock.failNext("getMeeting", "The folder was moved.");
    await openMeeting();

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not open this meeting: The folder was moved.",
    );
    expect(queryNotesEditor()).toBeNull();

    await click("Retry");
    expect(notesText()).toBe(NOTES);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("autosave", () => {
  test("saves the exact text shortly after typing pauses", async () => {
    seedMeeting({ notes: "" });
    const saveNotes = saveNotesSpy();
    await openMeeting();

    // Nothing here may be trimmed or tidied on the way to the backend.
    const typed = "  # Notes \n\n\t- [ ] call  back\n\n\n";
    typeInNotes(typed);
    await advance(PAUSE - 1);
    expect(saveNotes).not.toHaveBeenCalled();

    await advance(1);
    expect(saveNotes).toHaveBeenCalledExactlyOnceWith("retro", typed);
    expect(await savedNotes()).toBe(typed);

    // And nothing more is sent while nothing changes.
    await advance(60_000);
    expect(saveNotes).toHaveBeenCalledTimes(1);
  });

  test("waits for the pause: typing on puts the save off", async () => {
    seedMeeting({ notes: "" });
    const saveNotes = saveNotesSpy();
    await openMeeting();

    typeInNotes("one");
    await advance(PAUSE - 100);
    typeInNotes(" two");
    await advance(PAUSE - 100);
    expect(saveNotes).not.toHaveBeenCalled();

    await advance(100);
    expect(saveNotes).toHaveBeenCalledExactlyOnceWith("retro", "one two");
  });

  test("saves at once when the editor loses focus", async () => {
    seedMeeting();
    const saveNotes = saveNotesSpy();
    await openMeeting();

    typeInNotes("x");
    blurNotes();
    await advance();

    expect(saveNotes).toHaveBeenCalledExactlyOnceWith("retro", `x${NOTES}`);
  });

  test("leaving the view saves what was still waiting", async () => {
    seedMeeting();
    const saveNotes = saveNotesSpy();
    await openMeetingFromHome();

    typeInNotes("Typed and gone. ");
    await click("Back");

    expect(screen.getByRole("heading", { name: "Meetings" })).toBeVisible();
    expect(saveNotes).toHaveBeenCalledExactlyOnceWith(
      "retro",
      `Typed and gone. ${NOTES}`,
    );
    expect(await savedNotes()).toBe(`Typed and gone. ${NOTES}`);
  });

  test("closing the view saves too, without any navigation", async () => {
    seedMeeting();
    const saveNotes = saveNotesSpy();
    const view = await openMeeting();

    typeInNotes("x");
    view.unmount();
    await advance();

    expect(saveNotes).toHaveBeenCalledExactlyOnceWith("retro", `x${NOTES}`);
  });

  test("a failed save keeps the text, warns quietly and retries until it works", async () => {
    seedMeeting({ notes: "" });
    const saveNotes = saveNotesSpy();
    await openMeeting();

    mockApi.mock.failNext("saveNotes", "The disk is full.");
    typeInNotes("Do not lose this.");
    await advance(PAUSE);

    expect(saveNotes).toHaveBeenCalledTimes(1);
    expect(await savedNotes()).toBe("");
    // Said in place, and not as an alert: no popup, nothing modal.
    expect(screen.getByRole("status")).toHaveTextContent(
      "Notes not saved yet, still trying: The disk is full.",
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(notesText()).toBe("Do not lose this.");

    // The retry comes on its own, and it may fail again.
    mockApi.mock.failNext("saveNotes", "The disk is full.");
    await advance(2000);
    expect(saveNotes).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(notesText()).toBe("Do not lose this.");

    await advance(4000);
    expect(saveNotes).toHaveBeenCalledTimes(3);
    expect(saveNotes).toHaveBeenLastCalledWith("retro", "Do not lose this.");
    expect(await savedNotes()).toBe("Do not lose this.");
    expect(screen.queryByRole("status")).toBeNull();
  });

  test("text typed after a failed save goes out with the retry", async () => {
    seedMeeting({ notes: "" });
    await openMeeting();

    mockApi.mock.failNext("saveNotes");
    typeInNotes("first");
    await advance(PAUSE);
    expect(screen.getByRole("status")).toBeInTheDocument();

    typeInNotes(" second");
    await click("Try now");

    expect(await savedNotes()).toBe("first second");
    expect(screen.queryByRole("status")).toBeNull();
  });

  test("saves do not overlap, so they cannot arrive out of order", async () => {
    seedMeeting({ notes: "" });
    const first = deferred();
    const saveNotes = saveNotesSpy().mockReturnValueOnce(first.promise);
    await openMeeting();

    typeInNotes("a");
    await advance(PAUSE);
    expect(saveNotes).toHaveBeenCalledTimes(1);

    // The first save is still on its way. More typing, a pause, even a blur:
    // nothing else is sent until it has come back.
    typeInNotes("b");
    await advance(PAUSE);
    typeInNotes("c");
    blurNotes();
    await advance(10_000);
    expect(saveNotes).toHaveBeenCalledTimes(1);

    first.resolve();
    await advance();
    expect(saveNotes.mock.calls).toEqual([
      ["retro", "a"],
      ["retro", "abc"],
    ]);
    expect(await savedNotes()).toBe("abc");
  });

  test("keeps Windows line endings in a file that has them", async () => {
    seedMeeting({ notes: "one\r\ntwo\r\n" });
    const saveNotes = saveNotesSpy();
    await openMeeting();

    expect(notesText()).toBe("one\r\ntwo\r\n");
    const view = notesEditor();
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    typeInNotes("three");
    pressInNotes("Enter");
    await advance(PAUSE);

    expect(saveNotes).toHaveBeenLastCalledWith(
      "retro",
      "one\r\ntwo\r\nthree\r\n",
    );
  });

  test("notes still on their way are there when the meeting is opened again", async () => {
    seedMeeting();
    const slow = deferred();
    saveNotesSpy().mockReturnValueOnce(slow.promise);
    await openMeetingFromHome();

    typeInNotes("Unsaved so far. ");
    await click("Back");
    // The backend still has the old notes when the meeting is opened again.
    await click(/^Retro/);
    expect(notesText()).toBe(`Unsaved so far. ${NOTES}`);

    typeInNotes("More. ");
    slow.resolve();
    await advance(PAUSE);
    expect(await savedNotes()).toBe(`More. Unsaved so far. ${NOTES}`);
  });
});

describe("the title", () => {
  test("a new meeting has its date-time title, with no prompt for one", async () => {
    await startMeeting();

    expect(titleField().value).toMatch(/^Meeting, \w{3} \d{1,2} /);
    expect(screen.queryByRole("dialog")).toBeNull();
    // The title is there to be changed, but nothing asks for it.
    expect(titleField()).not.toHaveFocus();
  });

  test("Enter saves the new title and returns to the notes", async () => {
    seedMeeting();
    const renameMeeting = vi.spyOn(mockApi, "renameMeeting");
    await openMeetingFromHome();

    titleField().focus();
    fireEvent.change(titleField(), { target: { value: "Weekly sync" } });
    fireEvent.keyDown(titleField(), { key: "Enter" });
    await advance();

    expect(renameMeeting).toHaveBeenCalledExactlyOnceWith(
      "retro",
      "Weekly sync",
    );
    expect(titleField()).toHaveValue("Weekly sync");
    expect(notesHaveFocus()).toBe(true);

    await click("Back");
    expect(button(/^Weekly sync/)).toBeInTheDocument();
  });

  test("leaving the field saves the new title", async () => {
    seedMeeting();
    const renameMeeting = vi.spyOn(mockApi, "renameMeeting");
    await openMeeting();

    fireEvent.change(titleField(), { target: { value: "  Weekly sync " } });
    fireEvent.blur(titleField());
    await advance();

    expect(renameMeeting).toHaveBeenCalledExactlyOnceWith(
      "retro",
      "Weekly sync",
    );
    expect(titleField()).toHaveValue("Weekly sync");
  });

  test("a rename that fails says why in place and puts the old title back", async () => {
    seedMeeting();
    await openMeeting();

    mockApi.mock.failNext(
      "renameMeeting",
      'A meeting folder named "2026-11-12 1000 Weekly sync" already exists.',
    );
    fireEvent.change(titleField(), { target: { value: "Weekly sync" } });
    fireEvent.keyDown(titleField(), { key: "Enter" });
    await advance();

    expect(screen.getByRole("alert")).toHaveTextContent(
      'Could not rename the meeting: A meeting folder named "2026-11-12 1000 Weekly sync" already exists.',
    );
    expect(titleField()).toHaveValue("Retro");
    expect((await mockApi.getMeeting("retro")).title).toBe("Retro");

    // Taking the field up again clears the message.
    fireEvent.focus(titleField());
    expect(screen.queryByRole("alert")).toBeNull();
    fireEvent.change(titleField(), { target: { value: "Sync" } });
    fireEvent.keyDown(titleField(), { key: "Enter" });
    await advance();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(titleField()).toHaveValue("Sync");
  });

  test("an empty title and Escape both put the old title back without a rename", async () => {
    seedMeeting();
    const renameMeeting = vi.spyOn(mockApi, "renameMeeting");
    await openMeeting();

    fireEvent.change(titleField(), { target: { value: "   " } });
    fireEvent.blur(titleField());
    await advance();
    expect(titleField()).toHaveValue("Retro");

    titleField().focus();
    fireEvent.change(titleField(), { target: { value: "Something else" } });
    fireEvent.keyDown(titleField(), { key: "Escape" });
    await advance();
    expect(titleField()).toHaveValue("Retro");

    expect(renameMeeting).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("the title can be changed while recording", async () => {
    await startMeeting();
    const meetingId = await liveMeetingId();

    fireEvent.change(titleField(), { target: { value: "Kickoff" } });
    fireEvent.keyDown(titleField(), { key: "Enter" });
    await advance();

    expect((await mockApi.getMeeting(meetingId)).title).toBe("Kickoff");
    expect(recordingBar()).toBeInTheDocument();
  });
});

describe("while recording", () => {
  test("a new meeting opens with the cursor in the editor", async () => {
    await startMeeting();

    expect(notesText()).toBe("");
    expect(notesHaveFocus()).toBe(true);
  });

  test("coming back to the live meeting puts the cursor at the end of the notes", async () => {
    await startMeeting();
    typeInNotes("So far");
    await click("Back");
    await click("Open meeting");

    expect(notesHaveFocus()).toBe(true);
    typeInNotes(", and more");
    expect(notesText()).toBe("So far, and more");
  });

  test("a finished meeting does not take the focus when opened", async () => {
    seedMeeting();
    await openMeetingFromHome();

    expect(notesHaveFocus()).toBe(false);
  });

  test("shows Back, the title, the editor and the recording bar, and no transcript", async () => {
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await startMeeting();
    const meetingId = await liveMeetingId();
    // Long enough for the mock to have transcribed several lines.
    await advance(30_000);
    expect(
      (await mockApi.getMeeting(meetingId)).transcriptLineCount,
    ).toBeGreaterThan(0);

    expect(screen.queryByText(/transcript/i)).toBeNull();
    expect(screen.queryByText(/lines?\b/i)).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
    expect(screen.queryByRole("complementary")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();

    // Everything that can be pressed or typed in: nothing else is on screen.
    const topBar = within(screen.getByRole("banner"));
    expect(
      topBar.getAllByRole("button").map((b) => b.getAttribute("aria-label")),
    ).toEqual(["Back"]);
    expect(topBar.getByRole("textbox")).toBe(titleField());
    expect(
      within(screen.getByRole("main")).queryAllByRole("button"),
    ).toHaveLength(0);
    expect(queryNotesEditor()).not.toBeNull();
    expect(
      within(recordingBar()!)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Stop"]);
  });

  test("Stop saves the notes first, then stops, and stays on the meeting", async () => {
    const saveNotes = saveNotesSpy();
    const stopMeeting = vi.spyOn(mockApi, "stopMeeting");
    await startMeeting();
    const meetingId = await liveMeetingId();

    typeInNotes("Last words");
    await click("Stop");

    expect(saveNotes).toHaveBeenCalledExactlyOnceWith(meetingId, "Last words");
    expect(stopMeeting).toHaveBeenCalledTimes(1);
    expect(saveNotes.mock.invocationCallOrder[0]).toBeLessThan(
      stopMeeting.mock.invocationCallOrder[0],
    );

    await advance(600);
    expect(recordingBar()).toBeNull();
    const meeting = await mockApi.getMeeting(meetingId);
    expect(meeting.status).toBe("complete");
    expect(meeting.notes).toBe("Last words");
    // Still the same view, with the notes as they were and still editable.
    expect(notesText()).toBe("Last words");
    typeInNotes(", and an afterthought");
    await advance(PAUSE);
    expect(await savedNotes(meetingId)).toBe("Last words, and an afterthought");
  });

  test("Stop waits for the save to come back before it stops", async () => {
    const slow = deferred();
    saveNotesSpy().mockReturnValueOnce(slow.promise);
    const stopMeeting = vi.spyOn(mockApi, "stopMeeting");
    await startMeeting();

    typeInNotes("Last words");
    await click("Stop");
    expect(stopMeeting).not.toHaveBeenCalled();

    slow.resolve();
    await advance();
    expect(stopMeeting).toHaveBeenCalledTimes(1);
  });

  test("Stop does not wait long for a save that hangs", async () => {
    saveNotesSpy().mockReturnValueOnce(new Promise<void>(() => {}));
    const stopMeeting = vi.spyOn(mockApi, "stopMeeting");
    await startMeeting();

    typeInNotes("Last words");
    await click("Stop");
    await advance(2999);
    expect(stopMeeting).not.toHaveBeenCalled();

    await advance(1);
    expect(stopMeeting).toHaveBeenCalledTimes(1);
    expect(notesText()).toBe("Last words");
  });

  test("a save that fails does not keep the recording running", async () => {
    const stopMeeting = vi.spyOn(mockApi, "stopMeeting");
    await startMeeting();
    const meetingId = await liveMeetingId();

    mockApi.mock.failNext("saveNotes", "The disk is full.");
    typeInNotes("Last words");
    await click("Stop");
    await advance(600);

    expect(stopMeeting).toHaveBeenCalledTimes(1);
    expect(recordingBar()).toBeNull();
    // The notes are still here, and still being saved.
    expect(notesText()).toBe("Last words");
    expect(screen.getByRole("status")).toHaveTextContent("Notes not saved yet");
    await advance(2000);
    expect(await savedNotes(meetingId)).toBe("Last words");
  });
});
