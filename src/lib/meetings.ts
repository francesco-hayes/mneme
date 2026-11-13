import type { MeetingSummary } from "./api";

/** Start time in milliseconds; a date that cannot be read counts as oldest. */
function startTime(meeting: Pick<MeetingSummary, "startedAt">): number {
  const time = Date.parse(meeting.startedAt);
  return Number.isNaN(time) ? Number.NEGATIVE_INFINITY : time;
}

/**
 * A copy of `meetings` with the most recently started first. Meetings that
 * started at the same moment keep the order they came in.
 */
export function newestFirst<T extends Pick<MeetingSummary, "startedAt">>(
  meetings: readonly T[],
): T[] {
  return [...meetings].sort((a, b) => {
    const [timeA, timeB] = [startTime(a), startTime(b)];
    return timeA === timeB ? 0 : timeA > timeB ? -1 : 1;
  });
}
