import { expect, test } from "vitest";
import type { TranscriptLine } from "./api";
import {
  groupBySpeaker,
  shortTimestamp,
  timestampSecs,
  transcriptLineText,
} from "./transcript";

const line = (
  speaker: TranscriptLine["speaker"],
  text: string,
  t = "00:00:01",
): TranscriptLine => ({ t, speaker, text });

test("consecutive lines from one speaker make one turn", () => {
  const lines = [
    line("Them", "a"),
    line("Me", "b"),
    line("Me", "c"),
    line("Me", "d"),
    line("Them", "e"),
    line("Me", "f"),
  ];

  const turns = groupBySpeaker(lines);

  expect(
    turns.map((turn) => [
      turn.speaker,
      turn.start,
      turn.lines.map((l) => l.text).join(""),
    ]),
  ).toEqual([
    ["Them", 0, "a"],
    ["Me", 1, "bcd"],
    ["Them", 4, "e"],
    ["Me", 5, "f"],
  ]);
  // Every line is in exactly one turn, in order.
  expect(turns.flatMap((turn) => turn.lines)).toEqual(lines);
});

test("no lines make no turns, and the lines given are not changed", () => {
  expect(groupBySpeaker([])).toEqual([]);

  const lines = Object.freeze([line("Me", "a"), line("Me", "b")]);
  expect(groupBySpeaker(lines)).toHaveLength(1);
});

test("reads a timestamp as seconds, and nothing else as one", () => {
  expect(timestampSecs("00:00:00")).toBe(0);
  expect(timestampSecs("00:06:40")).toBe(400);
  expect(timestampSecs("01:02:03")).toBe(3723);
  expect(timestampSecs("100:00:00")).toBe(360_000);
  expect(timestampSecs("6:40")).toBeNull();
  expect(timestampSecs("00:61:00")).toBeNull();
  expect(timestampSecs("")).toBeNull();
});

test("shows a timestamp without the leading zeros", () => {
  expect(shortTimestamp("00:00:05")).toBe("0:05");
  expect(shortTimestamp("00:06:40")).toBe("6:40");
  expect(shortTimestamp("01:02:03")).toBe("1:02:03");
  // Not a timestamp: shown as it came.
  expect(shortTimestamp("later")).toBe("later");
});

test("writes a line the way the transcript file has it", () => {
  expect(
    transcriptLineText(line("Them", "The side pane feels heavy.", "00:06:40")),
  ).toBe("[00:06:40] Them: The side pane feels heavy.");
});
