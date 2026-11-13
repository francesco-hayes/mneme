// Reading a transcript: grouping its lines for display, and the text forms of
// a line. No React in here.

import type { TranscriptLine } from "./api";
import { formatElapsed } from "./format";

/** Consecutive lines from one speaker, shown as one block. */
export interface SpeakerTurn {
  speaker: TranscriptLine["speaker"];
  /** The index of the turn's first line in the transcript. */
  start: number;
  lines: TranscriptLine[];
}

/** Splits a transcript into turns: a new one starts whenever the speaker changes. */
export function groupBySpeaker(
  lines: readonly TranscriptLine[],
): SpeakerTurn[] {
  const turns: SpeakerTurn[] = [];
  lines.forEach((line, index) => {
    const last = turns.at(-1);
    if (last !== undefined && last.speaker === line.speaker) {
      last.lines.push(line);
    } else {
      turns.push({ speaker: line.speaker, start: index, lines: [line] });
    }
  });
  return turns;
}

/** Seconds in a transcript timestamp (`hh:mm:ss`), or null when it is not one. */
export function timestampSecs(t: string): number | null {
  const match = /^(\d+):([0-5]\d):([0-5]\d)$/.exec(t);
  if (match === null) return null;
  return Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3]);
}

/**
 * A transcript timestamp as it is shown beside a line: `6:40` for `00:06:40`,
 * `1:02:03` from an hour on. Anything that is not a timestamp is shown as it is.
 */
export function shortTimestamp(t: string): string {
  const secs = timestampSecs(t);
  return secs === null ? t : formatElapsed(secs);
}

/**
 * A line as plain text, in the format of the transcript file on disk:
 * `[00:06:40] Them: The side pane feels heavy.` Copying from the transcript
 * pane puts this on the clipboard.
 */
export function transcriptLineText(line: TranscriptLine): string {
  return `[${line.t}] ${line.speaker}: ${line.text}`;
}
