// Text formats shared by more than one screen.

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Recording time for the timer: `0:07`, `12:34`, and `1:02:03` from an hour on. */
export function formatElapsed(totalSecs: number): string {
  const secs = Math.max(0, Math.floor(totalSecs));
  const hours = Math.floor(secs / 3600);
  const minutes = Math.floor(secs / 60) % 60;
  const seconds = pad2(secs % 60);
  return hours > 0
    ? `${hours}:${pad2(minutes)}:${seconds}`
    : `${minutes}:${seconds}`;
}

/** Where and when a date is being read. Tests pass all three; the app passes `now`. */
export interface DateContext {
  /** The moment to compare against, for "Today" and "Yesterday". */
  now: Date;
  /** BCP 47 locale for month names and the 12 or 24 hour clock. Default: the system's. */
  locale?: string;
  /** IANA time zone the calendar days are counted in. Default: the system's. */
  timeZone?: string;
}

const DAY_MS = 24 * 60 * 60_000;

// Building an Intl formatter costs far more than using one, and a list formats
// every row with the same few, so they are kept.
const dateFormats = new Map<string, Intl.DateTimeFormat>();
const unitFormats = new Map<string, Intl.NumberFormat>();

function dateFormat(
  locale: string | undefined,
  timeZone: string | undefined,
  options: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
  const key = JSON.stringify([locale, timeZone, options]);
  let format = dateFormats.get(key);
  if (format === undefined) {
    format = new Intl.DateTimeFormat(locale, { ...options, timeZone });
    dateFormats.set(key, format);
  }
  return format;
}

/**
 * WebKit puts a narrow no-break space before AM and PM and other engines a
 * plain one; one kind everywhere keeps the text the same in the app, the
 * browser and tests.
 */
const plainSpaces = (text: string) => text.replace(/[\u00a0\u202f]/g, " ");

/** The calendar day `date` falls on in `timeZone`, as a count of days. */
function dayNumber(date: Date, timeZone: string | undefined): number {
  const parts = dateFormat("en-US-u-ca-gregory", timeZone, {
    year: "numeric",
    month: "numeric",
    day: "numeric",
  }).formatToParts(date);
  const part = (type: Intl.DateTimeFormatPartTypes) =>
    Number(parts.find((p) => p.type === type)?.value);
  return Date.UTC(part("year"), part("month") - 1, part("day")) / DAY_MS;
}

/**
 * When a meeting started, the way a list shows it: `Today 2:30 PM`,
 * `Yesterday 11:00 AM`, `Oct 27` for an earlier day of this year, and
 * `Oct 27, 2025` for another year. Empty for a date that cannot be read.
 */
export function formatMeetingDate(
  startedAt: string,
  { now, locale, timeZone }: DateContext,
): string {
  const date = new Date(startedAt);
  if (Number.isNaN(date.getTime())) return "";

  const daysAgo = dayNumber(now, timeZone) - dayNumber(date, timeZone);
  if (daysAgo === 0 || daysAgo === 1) {
    const time = dateFormat(locale, timeZone, {
      hour: "numeric",
      minute: "2-digit",
    }).format(date);
    return plainSpaces(`${daysAgo === 0 ? "Today" : "Yesterday"} ${time}`);
  }

  const year = (d: Date) =>
    dateFormat("en-US-u-ca-gregory", timeZone, { year: "numeric" }).format(d);
  const sameYear = year(date) === year(now);
  return plainSpaces(
    dateFormat(locale, timeZone, {
      month: "short",
      day: "numeric",
      year: sameYear ? undefined : "numeric",
    }).format(date),
  );
}

/**
 * The whole date and time, for a tooltip: `Tuesday, October 27, 2026 at
 * 2:30 PM`. Empty for a date that cannot be read.
 */
export function formatFullDate(
  startedAt: string,
  { locale, timeZone }: Omit<DateContext, "now">,
): string {
  const date = new Date(startedAt);
  if (Number.isNaN(date.getTime())) return "";
  return plainSpaces(
    dateFormat(locale, timeZone, {
      dateStyle: "full",
      timeStyle: "short",
    }).format(date),
  );
}

function unit(
  value: number,
  name: "second" | "minute" | "hour",
  locale: string | undefined,
): string {
  const key = `${locale}|${name}`;
  let format = unitFormats.get(key);
  if (format === undefined) {
    format = new Intl.NumberFormat(locale, {
      style: "unit",
      unit: name,
      unitDisplay: "short",
    });
    unitFormats.set(key, format);
  }
  return plainSpaces(format.format(value));
}

/**
 * How long a meeting lasted, to the nearest minute: `32 min`, `1 hr 5 min`,
 * `2 hr`, and `45 sec` under a minute. Null when there is no duration (a
 * meeting still being recorded, say), so the caller shows nothing.
 */
export function formatDuration(
  totalSecs: number | null,
  locale?: string,
): string | null {
  if (totalSecs === null || !Number.isFinite(totalSecs) || totalSecs < 0) {
    return null;
  }
  const secs = Math.floor(totalSecs);
  if (secs < 60) return unit(secs, "second", locale);

  const minutes = Math.round(secs / 60);
  if (minutes < 60) return unit(minutes, "minute", locale);

  const hours = unit(Math.floor(minutes / 60), "hour", locale);
  const rest = minutes % 60;
  return rest === 0 ? hours : `${hours} ${unit(rest, "minute", locale)}`;
}

const countFormats = new Map<string | undefined, Intl.NumberFormat>();

/**
 * How many lines a transcript has, as words: `no lines`, `1 line`,
 * `214 lines`, `2,000 lines`.
 */
export function formatLineCount(count: number, locale?: string): string {
  if (count <= 0) return "no lines";
  if (count === 1) return "1 line";
  let format = countFormats.get(locale);
  if (format === undefined) {
    format = new Intl.NumberFormat(locale);
    countFormats.set(locale, format);
  }
  return `${plainSpaces(format.format(count))} lines`;
}
