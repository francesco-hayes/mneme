// The meeting view after a meeting: the Transcript button, the transcript
// pane, the stop transition, the interrupted note and Show in Finder. The live
// view and the editor are covered in MeetingScreen.test.tsx.

import { fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { TranscriptLine } from "../lib/api";
import { mockApi } from "../lib/api/mock";
import { generateTranscript, type MockMeetingSeed } from "../lib/api/mockSeed";
import { formatDuration } from "../lib/format";
import {
  notesEditor,
  notesText,
  pressInNotes,
  queryNotesEditor,
  typeInNotes,
} from "../test/editor";
import { advance, renderShell } from "../test/render";

// Autosave, the mock's recording clock and its stop delay all run on timers.
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.getSelection()?.removeAllRanges();
});

/** How long typing has to pause before notes are saved. */
const PAUSE = 800;
/** How long the mock takes to finish a stop. */
const STOP = 600;

const NOTES = "# Retro\n\n- Export work is **merged**\n";

const TRANSCRIPT: TranscriptLine[] = [
  { t: "00:00:04", speaker: "Them", text: "Can you hear me?" },
  { t: "00:00:09", speaker: "Me", text: "Yes, loud and clear." },
  { t: "00:00:15", speaker: "Me", text: "Let's start." },
  { t: "00:06:40", speaker: "Them", text: "The export work is merged." },
  { t: "01:02:03", speaker: "Them", text: "One more thing." },
];

function seedMeeting(seed: Partial<MockMeetingSeed> = {}) {
  mockApi.mock.putMeeting({
    id: "retro",
    title: "Retro",
    startedAt: "2026-11-12T15:00:00Z",
    durationSecs: 600,
    status: "complete",
    notes: NOTES,
    transcript: TRANSCRIPT,
    ...seed,
  });
}

const button = (name: string | RegExp) => screen.getByRole("button", { name });
const click = async (name: string | RegExp) => {
  fireEvent.click(button(name));
  await advance();
};
const topBar = () => within(screen.getByRole("banner"));
const transcriptButton = () =>
  topBar().getByRole("button", { name: /^Transcript/ });
const queryTranscriptButton = () =>
  topBar().queryByRole("button", { name: /^Transcript/ });
/** The transcript pane, or null while it is closed. */
const pane = () => screen.queryByRole("complementary", { name: "Transcript" });
const openPane = () => {
  const element = pane();
  if (element === null) throw new Error("The transcript pane is closed.");
  return element;
};
const recordingBar = () => screen.queryByRole("region", { name: "Recording" });

/** Shift-Command-T, pressed with the focus outside the editor. */
async function pressShortcut(init: KeyboardEventInit = {}) {
  fireEvent.keyDown(document.body, {
    key: "T",
    metaKey: true,
    shiftKey: true,
    ...init,
  });
  await advance();
}

/** The lines on screen: who the turn belongs to, the time shown, the text. */
function shownLines() {
  return [...openPane().querySelectorAll<HTMLElement>("[data-line]")].map(
    (line) => ({
      index: Number(line.dataset.line),
      speaker: line.closest("[data-speaker]")?.getAttribute("data-speaker"),
      time: line.firstElementChild?.textContent,
      text: line.querySelector("[data-text]")?.textContent,
    }),
  );
}

async function openMeeting(meetingId = "retro") {
  const view = renderShell({ route: { name: "meeting", meetingId } });
  await advance();
  return view;
}

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
  const { meetingId } = await mockApi.getRecordingState();
  if (meetingId === null) throw new Error("Nothing is being recorded.");
  return meetingId;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("the Transcript button", () => {
  test("shows the line count, and the pane is closed", async () => {
    seedMeeting({ transcript: generateTranscript(214) });
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await openMeeting();

    expect(transcriptButton()).toHaveTextContent(/^Transcript · 214 lines$/);
    expect(transcriptButton()).toBeEnabled();
    expect(transcriptButton()).toHaveAttribute("aria-expanded", "false");
    // Closed by default: the notes have the view to themselves, and the
    // transcript has not even been read.
    expect(pane()).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
    expect(notesText()).toBe(NOTES);
  });

  test("says one line as one line", async () => {
    seedMeeting({ transcript: TRANSCRIPT.slice(0, 1) });
    await openMeeting();

    expect(transcriptButton()).toHaveTextContent(/^Transcript · 1 line$/);
  });

  test("with no lines it says so and still opens the pane", async () => {
    seedMeeting({ transcript: [] });
    await openMeeting();

    expect(transcriptButton()).toHaveTextContent(/^Transcript · no lines$/);
    expect(transcriptButton()).toBeEnabled();

    await click(/^Transcript/);
    expect(openPane()).toHaveTextContent(
      "Nothing was transcribed in this meeting.",
    );
    expect(openPane().querySelector("[data-line]")).toBeNull();
    expect(within(openPane()).queryByRole("alert")).toBeNull();
  });

  test("the pane is closed again the next time the meeting is opened", async () => {
    seedMeeting();
    await openMeetingFromHome();
    await click(/^Transcript/);
    expect(pane()).not.toBeNull();

    await click("Back");
    await click(/^Retro/);

    expect(pane()).toBeNull();
    expect(transcriptButton()).toHaveAttribute("aria-expanded", "false");
  });
});

describe("the transcript pane", () => {
  test("opens and closes with the button, and reads the transcript once", async () => {
    seedMeeting();
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await openMeeting();

    await click(/^Transcript/);
    expect(pane()).not.toBeNull();
    expect(transcriptButton()).toHaveAttribute("aria-expanded", "true");
    expect(transcriptButton()).toHaveAttribute("aria-controls", openPane().id);
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
    expect(getTranscript).toHaveBeenCalledExactlyOnceWith("retro");

    await click(/^Transcript/);
    expect(pane()).toBeNull();
    expect(transcriptButton()).toHaveAttribute("aria-expanded", "false");

    // Opened again: the same lines, without asking for them again.
    await click(/^Transcript/);
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
    expect(getTranscript).toHaveBeenCalledTimes(1);
  });

  test("sits beside the notes, which stay as they are", async () => {
    seedMeeting();
    await openMeeting();
    const editor = notesEditor();

    await click(/^Transcript/);

    const notes = screen.getByRole("region", { name: "Notes" });
    expect(notes.nextElementSibling).toBe(openPane());
    // The same editor, not a new one: opening the pane resets nothing.
    expect(notesEditor()).toBe(editor);
    expect(notesText()).toBe(NOTES);
  });

  test("shows every line with its time, under the speaker of its turn", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    expect(shownLines()).toEqual([
      { index: 0, speaker: "Them", time: "0:04", text: "Can you hear me?" },
      { index: 1, speaker: "Me", time: "0:09", text: "Yes, loud and clear." },
      { index: 2, speaker: "Me", time: "0:15", text: "Let's start." },
      {
        index: 3,
        speaker: "Them",
        time: "6:40",
        text: "The export work is merged.",
      },
      { index: 4, speaker: "Them", time: "1:02:03", text: "One more thing." },
    ]);
  });

  test("groups consecutive lines of one speaker into a turn that names the speaker", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    const turns = within(openPane()).getAllByRole("listitem");
    // Them, Me (two lines), Them (two lines).
    expect(
      turns.map((turn) => [
        turn.getAttribute("data-speaker"),
        // The name is written out: the two styles do not rest on color.
        turn.firstElementChild?.textContent,
        turn.querySelectorAll("[data-line]").length,
      ]),
    ).toEqual([
      ["Them", "Them", 1],
      ["Me", "Me", 2],
      ["Them", "Them", 2],
    ]);
  });

  test("says how many lines and how long, once the lines are there", async () => {
    seedMeeting({ durationSecs: 600 });
    await openMeeting();
    await click(/^Transcript/);

    expect(openPane()).toHaveTextContent(`5 lines · ${formatDuration(600)}`);
  });

  test("says it is loading while the transcript is on its way", async () => {
    seedMeeting();
    const slow = deferred<TranscriptLine[]>();
    vi.spyOn(mockApi, "getTranscript").mockReturnValueOnce(slow.promise);
    await openMeeting();

    await click(/^Transcript/);
    expect(within(openPane()).getByRole("status")).toHaveTextContent(
      "Loading transcript…",
    );
    expect(shownLines()).toHaveLength(0);

    slow.resolve(TRANSCRIPT);
    await advance();
    expect(within(openPane()).queryByRole("status")).toBeNull();
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
  });

  test("a transcript that cannot be read says why in the pane, and Retry reads it", async () => {
    seedMeeting();
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await openMeeting();

    mockApi.mock.failNext("getTranscript", "transcript.md is missing.");
    await click(/^Transcript/);

    expect(within(openPane()).getByRole("alert")).toHaveTextContent(
      "Could not load the transcript: transcript.md is missing.",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    // The notes are untouched by it.
    expect(notesText()).toBe(NOTES);

    fireEvent.click(within(openPane()).getByRole("button", { name: "Retry" }));
    await advance();
    expect(within(openPane()).queryByRole("alert")).toBeNull();
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
    expect(getTranscript).toHaveBeenCalledTimes(2);
  });

  test("opened again, it is scrolled to where it was left", async () => {
    seedMeeting({ transcript: generateTranscript(200) });
    await openMeeting();
    await click(/^Transcript/);
    const lines = within(openPane()).getByRole("list");
    lines.scrollTop = 1234;
    fireEvent.scroll(lines);

    await click(/^Transcript/);
    await click(/^Transcript/);

    const reopened = within(openPane()).getByRole("list");
    expect(reopened).not.toBe(lines);
    expect(reopened.scrollTop).toBe(1234);
  });

  test("opening the pane again after a failure tries again", async () => {
    seedMeeting();
    await openMeeting();

    mockApi.mock.failNext("getTranscript");
    await click(/^Transcript/);
    expect(within(openPane()).getByRole("alert")).toBeInTheDocument();

    await click(/^Transcript/);
    await click(/^Transcript/);
    expect(within(openPane()).queryByRole("alert")).toBeNull();
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
  });
});

describe("the keyboard shortcut", () => {
  test("Shift-Command-T opens and closes the pane", async () => {
    seedMeeting();
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await openMeeting();

    await pressShortcut();
    expect(pane()).not.toBeNull();
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);

    await pressShortcut();
    expect(pane()).toBeNull();

    // Lowercase, as the key arrives on some layouts and with Caps Lock.
    await pressShortcut({ key: "t" });
    expect(pane()).not.toBeNull();
    expect(getTranscript).toHaveBeenCalledTimes(1);
  });

  test("works with the cursor in the notes, and types nothing there", async () => {
    seedMeeting();
    await openMeeting();
    notesEditor().focus();

    // The key is pressed inside the editor's shadow root.
    pressInNotes("T", { metaKey: true, shiftKey: true });
    await advance();
    expect(pane()).not.toBeNull();

    pressInNotes("T", { metaKey: true, shiftKey: true });
    await advance();
    expect(pane()).toBeNull();
    expect(notesText()).toBe(NOTES);
  });

  test("other keys with T do nothing", async () => {
    seedMeeting();
    await openMeeting();

    await pressShortcut({ shiftKey: false });
    await pressShortcut({ metaKey: false });
    await pressShortcut({ altKey: true });
    await pressShortcut({ ctrlKey: true });
    await pressShortcut({ key: "Y" });
    await pressShortcut({ repeat: true });

    expect(pane()).toBeNull();
  });

  test("closing the pane from inside it leaves the focus on the button", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);
    const lines = within(openPane()).getByRole("list");
    lines.focus();
    expect(lines).toHaveFocus();

    fireEvent.keyDown(lines, { key: "T", metaKey: true, shiftKey: true });
    await advance();

    expect(pane()).toBeNull();
    expect(transcriptButton()).toHaveFocus();
  });
});

describe("copying from the transcript", () => {
  /** Selects from `startOffset` in one line's text to `endOffset` in another's. */
  function select(
    startLine: number,
    startOffset: number,
    endLine: number,
    endOffset: number,
  ) {
    const textNode = (index: number) =>
      openPane().querySelector(`[data-line="${index}"] [data-text]`)!
        .firstChild!;
    document
      .getSelection()!
      .setBaseAndExtent(
        textNode(startLine),
        startOffset,
        textNode(endLine),
        endOffset,
      );
  }

  /** Copies, and returns what the page put on the clipboard (null: the browser's own copy). */
  function copy(): string | null {
    const setData = vi.fn();
    const taken = !fireEvent.copy(within(openPane()).getByRole("list"), {
      clipboardData: { setData },
    });
    if (!taken) {
      expect(setData).not.toHaveBeenCalled();
      return null;
    }
    expect(setData).toHaveBeenCalledExactlyOnceWith(
      "text/plain",
      expect.any(String),
    );
    return setData.mock.calls[0][1] as string;
  }

  test("a selection across lines is copied as whole lines, in the transcript file's format", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    // From the middle of the second line to the middle of the fourth.
    select(1, 5, 3, 10);

    expect(copy()).toBe(
      [
        "[00:00:09] Me: Yes, loud and clear.",
        "[00:00:15] Me: Let's start.",
        "[00:06:40] Them: The export work is merged.",
      ].join("\n"),
    );
  });

  test("a few words of one line are left to the browser to copy", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    select(3, 4, 3, 15);

    expect(copy()).toBeNull();
  });

  test("with nothing selected in the transcript, copying is left alone", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    expect(copy()).toBeNull();
  });

  test("a copy made in the title field is not taken over", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);
    // The keyboard is in the title, with lines of the transcript still selected.
    screen.getByRole("textbox", { name: "Meeting title" }).focus();
    select(1, 5, 3, 10);
    expect(document.getSelection()?.isCollapsed).toBe(false);

    expect(copy()).toBeNull();
  });

  test("the lines can be selected", async () => {
    seedMeeting();
    await openMeeting();
    await click(/^Transcript/);

    // The page turns selection off everywhere but under this class.
    expect(within(openPane()).getByRole("list")).toHaveClass("selectable");
  });
});

describe("an interrupted meeting", () => {
  test("says quietly that the transcript may be incomplete", async () => {
    seedMeeting({ status: "interrupted" });
    await openMeeting();

    expect(screen.getByRole("note")).toHaveTextContent(
      "The recording ended unexpectedly. The transcript may be incomplete.",
    );
    // A note, not an alarm.
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    // Everything else is as for any other meeting.
    expect(transcriptButton()).toHaveTextContent(/^Transcript · 5 lines$/);
    await click(/^Transcript/);
    expect(shownLines()).toHaveLength(TRANSCRIPT.length);
    expect(screen.getByRole("note")).toBeInTheDocument();
  });

  test("a meeting that ended normally has no such note", async () => {
    seedMeeting();
    await openMeeting();

    expect(screen.queryByRole("note")).toBeNull();
    expect(screen.queryByText(/unexpectedly/)).toBeNull();
  });
});

describe("Show in Finder", () => {
  test("asks the backend to show this meeting's folder", async () => {
    seedMeeting();
    await openMeeting();

    await click("Show in Finder");

    expect(mockApi.mock.revealedInFinder).toEqual(["retro"]);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("a failure is said in place and goes with the next try", async () => {
    seedMeeting();
    await openMeeting();

    mockApi.mock.failNext("showInFinder", "The folder was moved.");
    await click("Show in Finder");

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not show the meeting in Finder: The folder was moved.",
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(mockApi.mock.revealedInFinder).toEqual([]);
    expect(notesText()).toBe(NOTES);

    await click("Show in Finder");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(mockApi.mock.revealedInFinder).toEqual(["retro"]);
  });
});

describe("the notes after a meeting", () => {
  test("can still be edited, and are saved as before", async () => {
    seedMeeting();
    const saveNotes = vi.spyOn(mockApi, "saveNotes");
    await openMeeting();

    typeInNotes("Afterthought. ");
    await advance(PAUSE);

    expect(saveNotes).toHaveBeenCalledExactlyOnceWith(
      "retro",
      `Afterthought. ${NOTES}`,
    );
    expect((await mockApi.getMeeting("retro")).notes).toBe(
      `Afterthought. ${NOTES}`,
    );
  });

  test("can be edited with the transcript open beside them", async () => {
    seedMeeting({ status: "interrupted" });
    await openMeeting();
    await click(/^Transcript/);

    typeInNotes("Checked against the transcript. ");
    await advance(PAUSE);

    expect((await mockApi.getMeeting("retro")).notes).toBe(
      `Checked against the transcript. ${NOTES}`,
    );
    expect(pane()).not.toBeNull();
  });
});

describe("when the recording stops", () => {
  test("while recording, the shortcut does nothing and nothing about the transcript shows", async () => {
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await startMeeting();
    await advance(30_000);

    await pressShortcut();
    pressInNotes("T", { metaKey: true, shiftKey: true });
    await advance();

    expect(pane()).toBeNull();
    expect(queryTranscriptButton()).toBeNull();
    expect(screen.queryByText(/transcript/i)).toBeNull();
    expect(screen.queryByRole("button", { name: "Show in Finder" })).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
  });

  test("between Stop and the finished transcript, the button says it is finishing and cannot be used", async () => {
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await startMeeting();
    await advance(30_000);

    await click("Stop");

    const finishing = topBar().getByRole("button", {
      name: "Finishing transcript…",
    });
    expect(finishing).toBeDisabled();
    // Not the ready state: no count yet, and no way into the pane.
    expect(queryTranscriptButton()).toBeNull();
    expect(screen.queryByText(/lines?\b/)).toBeNull();
    await pressShortcut();
    expect(pane()).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
    // The notes are still there to type in.
    typeInNotes("Still typing");
    expect(notesText()).toBe("Still typing");
  });

  test("when the stop completes the button is ready with the count, and the pane stays closed", async () => {
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    const meetingId = await startMeeting();
    // The mock transcribes a line every five seconds.
    await advance(30_000);

    await click("Stop");
    await advance(STOP);

    expect(recordingBar()).toBeNull();
    expect((await mockApi.getMeeting(meetingId)).transcriptLineCount).toBe(6);
    expect(transcriptButton()).toHaveTextContent(/^Transcript · 6 lines$/);
    expect(transcriptButton()).toBeEnabled();
    expect(transcriptButton()).toHaveAttribute("aria-expanded", "false");
    expect(
      topBar().queryByRole("button", { name: "Finishing transcript…" }),
    ).toBeNull();
    // Nothing opened by itself, and nothing was read for a pane nobody asked for.
    expect(pane()).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
    expect(button("Show in Finder")).toBeEnabled();
  });

  test("the editor and unsaved text survive the stop", async () => {
    const saveNotes = vi.spyOn(mockApi, "saveNotes");
    const meetingId = await startMeeting();
    typeInNotes("During the meeting.");
    const editor = notesEditor();
    await advance(30_000);
    saveNotes.mockClear();

    await click("Stop");
    // Typed while the transcript is finishing, and not saved before it is done.
    typeInNotes(" While stopping.");
    await advance(STOP);

    expect(transcriptButton()).toHaveTextContent(/^Transcript · 6 lines$/);
    expect(saveNotes).not.toHaveBeenCalled();
    // The same editor with the same text: the view was not rebuilt around it.
    expect(notesEditor()).toBe(editor);
    expect(notesText()).toBe("During the meeting. While stopping.");

    // And it is saved in its own time, like any other typing.
    await advance(PAUSE);
    expect(saveNotes).toHaveBeenCalledExactlyOnceWith(
      meetingId,
      "During the meeting. While stopping.",
    );
  });

  test("the finished meeting's lines and duration are what the pane then shows", async () => {
    await startMeeting();
    await advance(30_000);
    await click("Stop");
    await advance(STOP);

    await click(/^Transcript/);

    expect(shownLines()).toHaveLength(6);
    expect(openPane()).toHaveTextContent(`6 lines · ${formatDuration(30)}`);
  });

  test("it stays finishing until the finished meeting has been read", async () => {
    const meetingId = await startMeeting();
    await advance(30_000);
    await click("Stop");
    const finished = deferred<Awaited<ReturnType<typeof mockApi.getMeeting>>>();
    const getMeeting = vi
      .spyOn(mockApi, "getMeeting")
      .mockReturnValueOnce(finished.promise);

    // The recording is over, and the view is asking how it ended.
    await advance(STOP);
    expect(recordingBar()).toBeNull();
    expect(getMeeting).toHaveBeenCalledExactlyOnceWith(meetingId);
    expect(
      topBar().getByRole("button", { name: "Finishing transcript…" }),
    ).toBeDisabled();
    expect(queryTranscriptButton()).toBeNull();

    getMeeting.mockRestore();
    finished.resolve(await mockApi.getMeeting(meetingId));
    await advance();
    expect(transcriptButton()).toHaveTextContent(/^Transcript · 6 lines$/);
  });

  test("a meeting marked as recording that nothing is recording stays quiet", async () => {
    // What the view has before the recording state arrives, or after a crash
    // that recovery has not tidied up yet: no transcript UI on a guess.
    seedMeeting({ status: "recording", durationSecs: null });
    const getTranscript = vi.spyOn(mockApi, "getTranscript");
    await openMeeting();

    await pressShortcut();

    expect(queryTranscriptButton()).toBeNull();
    expect(screen.queryByRole("button", { name: "Show in Finder" })).toBeNull();
    expect(pane()).toBeNull();
    expect(getTranscript).not.toHaveBeenCalled();
    expect(notesText()).toBe(NOTES);
  });

  test("a recording stopped from outside the window ends the same way", async () => {
    await startMeeting();
    await advance(10_000);

    // The menu bar or the global shortcut: not this window's Stop button.
    void mockApi.stopMeeting();
    await advance();
    expect(
      topBar().getByRole("button", { name: "Finishing transcript…" }),
    ).toBeDisabled();

    await advance(STOP);
    expect(transcriptButton()).toHaveTextContent(/^Transcript · 2 lines$/);
    expect(pane()).toBeNull();
  });

  test("if the finished meeting cannot be read, the button still works, without a count", async () => {
    await startMeeting();
    typeInNotes("Notes");
    await advance(30_000);
    await click("Stop");

    mockApi.mock.failNext("getMeeting", "The disk is busy.");
    await advance(STOP);

    // No count it cannot vouch for, and no error screen in place of the notes.
    expect(transcriptButton()).toHaveTextContent(/^Transcript$/);
    expect(transcriptButton()).toBeEnabled();
    expect(queryNotesEditor()).not.toBeNull();
    expect(notesText()).toBe("Notes");
    expect(screen.queryByRole("alert")).toBeNull();

    // The lines themselves then give the count.
    await click(/^Transcript/);
    expect(shownLines()).toHaveLength(6);
    expect(transcriptButton()).toHaveTextContent(/^Transcript · 6 lines$/);
  });

  test("a stop that fails goes back to the quiet live view", async () => {
    await startMeeting();
    await advance(10_000);

    mockApi.mock.failNext("stopMeeting", "The recorder did not answer.");
    await click("Stop");

    expect(recordingBar()).toHaveTextContent("Could not stop");
    expect(topBar().queryAllByRole("button")).toHaveLength(1);
    expect(queryTranscriptButton()).toBeNull();
    expect(pane()).toBeNull();
  });
});
