import { expect, test } from "vitest";
import { newestFirst } from "./meetings";

const ids = (meetings: { id: string }[]) => meetings.map((m) => m.id);

test("puts the most recently started meeting first", () => {
  const meetings = [
    { id: "october", startedAt: "2026-10-27T14:00:00Z" },
    { id: "today", startedAt: "2026-11-12T19:30:00Z" },
    { id: "last-year", startedAt: "2025-12-03T15:00:00Z" },
    { id: "yesterday", startedAt: "2026-11-11T16:00:00Z" },
  ];

  expect(ids(newestFirst(meetings))).toEqual([
    "today",
    "yesterday",
    "october",
    "last-year",
  ]);
  // The list it was given is left alone.
  expect(ids(meetings)[0]).toBe("october");
});

test("compares moments, not text: time zone offsets are honored", () => {
  const meetings = [
    // 09:00 in New York is 14:00 UTC, an hour after 13:00 UTC.
    { id: "earlier", startedAt: "2026-11-12T13:00:00Z" },
    { id: "later", startedAt: "2026-11-12T09:00:00-05:00" },
  ];

  expect(ids(newestFirst(meetings))).toEqual(["later", "earlier"]);
});

test("a date that cannot be read goes last; ties keep their order", () => {
  const meetings = [
    { id: "unreadable", startedAt: "soon" },
    { id: "first", startedAt: "2026-11-12T13:00:00Z" },
    { id: "second", startedAt: "2026-11-12T13:00:00Z" },
  ];

  expect(ids(newestFirst(meetings))).toEqual(["first", "second", "unreadable"]);
});
