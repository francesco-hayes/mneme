import { describe, expect, test } from "vitest";
import {
  formatDuration,
  formatElapsed,
  formatFullDate,
  formatLineCount,
  formatMeetingDate,
} from "./format";

test("formats recording time as m:ss, and h:mm:ss from an hour on", () => {
  expect(formatElapsed(0)).toBe("0:00");
  expect(formatElapsed(7)).toBe("0:07");
  expect(formatElapsed(754)).toBe("12:34");
  expect(formatElapsed(3599)).toBe("59:59");
  expect(formatElapsed(3723)).toBe("1:02:03");
});

test("treats negative and fractional seconds as whole seconds from zero", () => {
  expect(formatElapsed(-4)).toBe("0:00");
  expect(formatElapsed(61.9)).toBe("1:01");
});

// Thursday, November 12, 2026, 3:00 PM in New York (UTC-5).
const now = new Date("2026-11-12T20:00:00Z");
const newYork = { now, locale: "en-US", timeZone: "America/New_York" };

describe("formatMeetingDate", () => {
  test("today and yesterday show the time", () => {
    expect(formatMeetingDate("2026-11-12T19:30:00Z", newYork)).toBe(
      "Today 2:30 PM",
    );
    expect(formatMeetingDate("2026-11-12T05:05:00Z", newYork)).toBe(
      "Today 12:05 AM",
    );
    expect(formatMeetingDate("2026-11-11T16:00:00Z", newYork)).toBe(
      "Yesterday 11:00 AM",
    );
  });

  test("earlier days show the date, with the year when it is not this one", () => {
    expect(formatMeetingDate("2026-11-10T16:00:00Z", newYork)).toBe("Nov 10");
    expect(formatMeetingDate("2026-10-27T14:00:00Z", newYork)).toBe("Oct 27");
    expect(formatMeetingDate("2026-01-01T12:00:00Z", newYork)).toBe("Jan 1");
    expect(formatMeetingDate("2025-12-31T23:00:00Z", newYork)).toBe(
      "Dec 31, 2025",
    );
  });

  test("days are counted in the given time zone, not in UTC or the machine's", () => {
    // 03:30 UTC on the 12th is 10:30 PM on the 11th in New York.
    const lateEvening = "2026-11-12T03:30:00Z";
    expect(formatMeetingDate(lateEvening, newYork)).toBe("Yesterday 10:30 PM");
    expect(
      formatMeetingDate(lateEvening, { ...newYork, timeZone: "UTC" }),
    ).toBe("Today 3:30 AM");
    // In Tokyo it is already 5:00 AM on the 13th, so the 12th is yesterday.
    expect(
      formatMeetingDate(lateEvening, { ...newYork, timeZone: "Asia/Tokyo" }),
    ).toBe("Yesterday 12:30 PM");
  });

  test("follows the locale for the clock and the order of day and month", () => {
    const london = { now, locale: "en-GB", timeZone: "Europe/London" };
    expect(formatMeetingDate("2026-11-12T14:30:00Z", london)).toBe(
      "Today 14:30",
    );
    expect(formatMeetingDate("2026-10-27T14:00:00Z", london)).toBe("27 Oct");
  });

  test("a meeting dated after now shows its date", () => {
    expect(formatMeetingDate("2026-11-13T15:00:00Z", newYork)).toBe("Nov 13");
  });

  test("a date that cannot be read gives an empty string", () => {
    expect(formatMeetingDate("not a date", newYork)).toBe("");
    expect(formatMeetingDate("", newYork)).toBe("");
  });
});

test("formatFullDate gives the whole date and time", () => {
  expect(formatFullDate("2026-10-27T14:00:00Z", newYork)).toBe(
    "Tuesday, October 27, 2026 at 10:00 AM",
  );
  expect(formatFullDate("not a date", newYork)).toBe("");
});

describe("formatDuration", () => {
  test("minutes, to the nearest one", () => {
    expect(formatDuration(32 * 60, "en-US")).toBe("32 min");
    expect(formatDuration(60, "en-US")).toBe("1 min");
    expect(formatDuration(89, "en-US")).toBe("1 min");
    expect(formatDuration(95, "en-US")).toBe("2 min");
    expect(formatDuration(14 * 60 + 5, "en-US")).toBe("14 min");
  });

  test("hours from an hour on", () => {
    expect(formatDuration(3600, "en-US")).toBe("1 hr");
    expect(formatDuration(3900, "en-US")).toBe("1 hr 5 min");
    expect(formatDuration(2 * 3600 + 29 * 60 + 40, "en-US")).toBe(
      "2 hr 30 min",
    );
    // 59 min 40 s rounds up to the hour, not to "60 min".
    expect(formatDuration(3580, "en-US")).toBe("1 hr");
  });

  test("seconds under a minute", () => {
    expect(formatDuration(45, "en-US")).toBe("45 sec");
    expect(formatDuration(0, "en-US")).toBe("0 sec");
    expect(formatDuration(59.9, "en-US")).toBe("59 sec");
  });

  test("no duration gives null", () => {
    expect(formatDuration(null, "en-US")).toBeNull();
    expect(formatDuration(-5, "en-US")).toBeNull();
    expect(formatDuration(Number.NaN, "en-US")).toBeNull();
  });
});

test("counts transcript lines in words", () => {
  expect(formatLineCount(0)).toBe("no lines");
  expect(formatLineCount(1)).toBe("1 line");
  expect(formatLineCount(2)).toBe("2 lines");
  expect(formatLineCount(214, "en-US")).toBe("214 lines");
  expect(formatLineCount(2000, "en-US")).toBe("2,000 lines");
  expect(formatLineCount(2000, "de-DE")).toBe("2.000 lines");
});
