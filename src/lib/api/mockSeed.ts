// Seed data for the mock API. Nothing here is read by the real app.

import type { MeetingStatus, ModelInfo, TranscriptLine } from "./types";

/** A meeting as the mock stores it. The line count and folder path are derived. */
export interface MockMeetingSeed {
  id: string;
  title: string;
  /** False while the meeting still has its default date-time title. Defaults to true. */
  titleEdited?: boolean;
  /** ISO 8601 start time. */
  startedAt: string;
  durationSecs: number | null;
  status: MeetingStatus;
  notes: string;
  transcript: TranscriptLine[];
}

/**
 * The mock's model catalog. Sizes are the file sizes listed by the model
 * source (ggerganov/whisper.cpp on Hugging Face) on 2026-11-12; ids, names and
 * notes are placeholders until issue 007 defines the real catalog.
 */
export const MOCK_MODEL_CATALOG: readonly Omit<ModelInfo, "installed">[] = [
  {
    id: "tiny.en",
    name: "Tiny (English)",
    sizeBytes: 77_704_715,
    note: "Fastest. Misses more words.",
    englishOnly: true,
  },
  {
    id: "base",
    name: "Base",
    sizeBytes: 147_951_465,
    note: "Fast. Fine for clear audio.",
    englishOnly: false,
  },
  {
    id: "small",
    name: "Small",
    sizeBytes: 487_601_967,
    note: "Balanced speed and accuracy.",
    englishOnly: false,
  },
  {
    id: "medium",
    name: "Medium",
    sizeBytes: 1_533_763_059,
    note: "More accurate. Slower.",
    englishOnly: false,
  },
  {
    id: "large-v3-turbo",
    name: "Large v3 Turbo",
    sizeBytes: 1_624_555_275,
    note: "Most accurate here. Needs a recent Mac.",
    englishOnly: false,
  },
];

/** The model the default seed has installed. */
export const DEFAULT_INSTALLED_MODEL_ID = "base";

const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Seconds of meeting time as `hh:mm:ss`, the transcript's timestamp format. */
export function formatTimestamp(totalSecs: number): string {
  const secs = Math.max(0, Math.floor(totalSecs));
  return `${pad2(Math.floor(secs / 3600))}:${pad2(Math.floor(secs / 60) % 60)}:${pad2(secs % 60)}`;
}

/** The title a meeting gets when it starts, for example `Meeting, Oct 27 2:30 PM`. */
export function defaultMeetingTitle(start: Date): string {
  const hours = start.getHours();
  const time = `${hours % 12 || 12}:${pad2(start.getMinutes())} ${hours < 12 ? "AM" : "PM"}`;
  return `Meeting, ${MONTHS[start.getMonth()]} ${start.getDate()} ${time}`;
}

/** The meetings root the mock pretends to use. */
export const MOCK_MEETINGS_ROOT = "/Users/you/Documents/Mneme";

/** `YYYY-MM-DD HHMM Title` in local time; `Meeting` while the title is the default one. */
export function meetingFolderName(
  startedAt: string,
  title: string,
  titleEdited: boolean,
): string {
  const start = new Date(startedAt);
  const date = `${start.getFullYear()}-${pad2(start.getMonth() + 1)}-${pad2(start.getDate())}`;
  const time = `${pad2(start.getHours())}${pad2(start.getMinutes())}`;
  const safeTitle = title
    .replace(/[/\\:]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return `${date} ${time} ${titleEdited && safeTitle ? safeTitle : "Meeting"}`;
}

const SCRIPT: readonly Pick<TranscriptLine, "speaker" | "text">[] = [
  { speaker: "Them", text: "Can you hear me all right?" },
  { speaker: "Me", text: "Yes, loud and clear." },
  { speaker: "Me", text: "Let's start with where we left off last week." },
  { speaker: "Them", text: "Sure. The export work is done and merged." },
  { speaker: "Them", text: "The part that slipped is the settings screen." },
  { speaker: "Me", text: "What is holding it up?" },
  {
    speaker: "Them",
    text: "Mostly the shortcut recorder. It needs more testing on older keyboards, and I want to be sure a bad combination cannot lock anyone out of typing.",
  },
  { speaker: "Me", text: "That makes sense. Is Thursday realistic?" },
  { speaker: "Them", text: "Thursday works if nothing else lands on me." },
  { speaker: "Me", text: "Good. I will keep the review slot free." },
  {
    speaker: "Them",
    text: "One more thing: the model download is slow on hotel Wi-Fi.",
  },
  {
    speaker: "Me",
    text: "Noted. I will add a line about it to the first-run screen.",
  },
];

/**
 * A deterministic transcript of `lineCount` lines for both speakers, one line
 * every `secondsPerLine` seconds. Use it for long-transcript checks, for
 * example `generateTranscript(2000)`.
 */
export function generateTranscript(
  lineCount: number,
  secondsPerLine = 5,
): TranscriptLine[] {
  return Array.from({ length: lineCount }, (_, i) => ({
    t: formatTimestamp((i + 1) * secondsPerLine),
    ...SCRIPT[i % SCRIPT.length],
  }));
}

/** The line the mock appends as the `index`-th transcript line of a recording. */
export function scriptedLine(index: number, atSecs: number): TranscriptLine {
  return { t: formatTimestamp(atSecs), ...SCRIPT[index % SCRIPT.length] };
}

const HOUR = 60 * 60_000;

/** `hour:minute` local time, `days` calendar days before `now`. */
function daysAgo(now: Date, days: number, hour: number, minute = 0): Date {
  return new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - days,
    hour,
    minute,
  );
}

/**
 * The default meetings, dated relative to `now`: one from three hours ago, an
 * interrupted one from yesterday, one from three days ago with a default title
 * and an empty transcript, and one from 16 days ago with a 214-line transcript.
 */
export function seedMeetings(now: Date): MockMeetingSeed[] {
  const untitledStart = daysAgo(now, 3, 13);

  return [
    {
      id: "mtg-standup",
      title: "Standup",
      startedAt: new Date(now.getTime() - 3 * HOUR).toISOString(),
      durationSecs: 14 * 60 + 5,
      status: "complete",
      notes: [
        "# Standup",
        "",
        "- Export work is **merged**",
        "- Settings screen slips to Thursday",
        "  - shortcut recorder needs more testing",
        "- Add a note about slow downloads to first run",
        "",
      ].join("\n"),
      transcript: generateTranscript(12, 8),
    },
    {
      id: "mtg-design-review",
      title: "Design review",
      startedAt: daysAgo(now, 1, 11).toISOString(),
      // What recovery sets for an interrupted meeting: the last transcript timestamp.
      durationSecs: 21 * 60,
      status: "interrupted",
      notes:
        "## Open questions\n\n- Does the transcript pane need a search field?\n",
      transcript: [
        {
          t: "00:00:04",
          speaker: "Me",
          text: "Thanks for joining. I will share the mockups.",
        },
        {
          t: "00:00:11",
          speaker: "Them",
          text: "Go ahead, I have them open too.",
        },
        {
          t: "00:06:40",
          speaker: "Them",
          text: "The side pane feels heavy when it is open.",
        },
        {
          t: "00:06:52",
          speaker: "Me",
          text: "It is closed by default, so most people never see it.",
        },
        {
          t: "00:13:15",
          speaker: "Them",
          text: "Fair. Then make the button say how many lines there are.",
        },
        { t: "00:13:21", speaker: "Me", text: "Agreed." },
        {
          t: "00:20:48",
          speaker: "Them",
          text: "What about search inside the transcript?",
        },
        {
          t: "00:21:00",
          speaker: "Me",
          text: "Later, I think. Let me write that down.",
        },
      ],
    },
    {
      id: "mtg-untitled",
      title: defaultMeetingTitle(untitledStart),
      titleEdited: false,
      startedAt: untitledStart.toISOString(),
      durationSecs: 95,
      status: "complete",
      notes: "",
      transcript: [],
    },
    {
      id: "mtg-planning",
      title: "Quarterly planning",
      startedAt: daysAgo(now, 16, 10).toISOString(),
      durationSecs: 52 * 60,
      status: "complete",
      notes:
        "# Quarterly planning\n\n1. Ship the MVP\n2. Benchmark the larger models\n",
      transcript: generateTranscript(214, 14),
    },
  ];
}

/** `seedMeetings` plus one meeting whose transcript has `lineCount` lines. */
export function seedMeetingsWithLongTranscript(
  now: Date,
  lineCount = 2000,
): MockMeetingSeed[] {
  return [
    {
      id: "mtg-long",
      title: "All-hands (long transcript)",
      startedAt: new Date(now.getTime() - 5 * HOUR).toISOString(),
      durationSecs: lineCount * 5,
      status: "complete",
      notes: "",
      transcript: generateTranscript(lineCount),
    },
    ...seedMeetings(now),
  ];
}
