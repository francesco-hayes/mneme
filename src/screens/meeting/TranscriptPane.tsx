import { memo, type RefObject, useLayoutEffect, useMemo, useRef } from "react";
import { InlineError } from "../../components";
import type { TranscriptLine } from "../../lib/api";
import { cx } from "../../lib/cx";
import { formatDuration, formatLineCount } from "../../lib/format";
import {
  groupBySpeaker,
  shortTimestamp,
  timestampSecs,
  transcriptLineText,
} from "../../lib/transcript";
import { useWindowEvent } from "../../lib/useWindowEvent";
import styles from "./TranscriptPane.module.css";
import type { TranscriptLoad } from "./useTranscript";

export interface TranscriptPaneProps {
  /** The element's id, for the button that opens and closes the pane. */
  id: string;
  load: TranscriptLoad;
  onRetry: () => void;
  /** How long the recording lasted, shown beside the line count. */
  durationSecs: number | null;
  /**
   * Where the transcript was scrolled to when the pane was last closed, so
   * that opening it again returns to the same place. The pane reads it when
   * the lines appear and keeps it up to date.
   */
  scrollMemo: RefObject<number>;
}

/**
 * The transcript, beside the notes: who said what, and when. It is shown
 * only after a meeting, and only when asked for.
 */
export function TranscriptPane({
  id,
  load,
  onRetry,
  durationSecs,
  scrollMemo,
}: TranscriptPaneProps) {
  const summary =
    load.status === "ready"
      ? [formatLineCount(load.lines.length), formatDuration(durationSecs)]
          .filter((part) => part !== null)
          .join(" · ")
      : null;

  return (
    <aside id={id} className={styles.pane} aria-label="Transcript">
      <div className={styles.header}>
        <h2 className={styles.heading}>Transcript</h2>
        {summary !== null && <span className={styles.summary}>{summary}</span>}
      </div>
      {load.status === "loading" && (
        <p className={cx(styles.message, "appears-late")} role="status">
          Loading transcript…
        </p>
      )}
      {load.status === "error" && (
        <InlineError
          className={styles.error}
          message={`Could not load the transcript: ${load.message}`}
          onRetry={onRetry}
        />
      )}
      {load.status === "ready" &&
        (load.lines.length === 0 ? (
          <p className={styles.message}>
            Nothing was transcribed in this meeting.
          </p>
        ) : (
          <TranscriptLines lines={load.lines} scrollMemo={scrollMemo} />
        ))}
    </aside>
  );
}

interface TranscriptLinesProps {
  lines: TranscriptLine[];
  scrollMemo: RefObject<number>;
}

/**
 * Every line of the transcript, in turns: the lines one speaker said in a row
 * share one block under that speaker's name.
 *
 * All of the lines are in the page at once. That is deliberate: a few
 * thousand short paragraphs lay out in tens of milliseconds and then scroll
 * without any script running, selection and copying work across the whole
 * transcript, and nothing needs positioning from script. Memoized, because
 * the screen around it renders again every second while another meeting is
 * being recorded.
 */
const TranscriptLines = memo(function TranscriptLines({
  lines,
  scrollMemo,
}: TranscriptLinesProps) {
  const turns = useMemo(() => groupBySpeaker(lines), [lines]);
  // Timestamps get a wider column once any of them shows hours.
  const hasHours = useMemo(
    () => lines.some((line) => (timestampSecs(line.t) ?? 0) >= 3600),
    [lines],
  );
  const list = useRef<HTMLOListElement>(null);

  useLayoutEffect(() => {
    const element = list.current;
    if (element === null) return;
    element.scrollTop = scrollMemo.current;
    const remember = () => {
      scrollMemo.current = element.scrollTop;
    };
    element.addEventListener("scroll", remember, { passive: true });
    return () => element.removeEventListener("scroll", remember);
  }, [scrollMemo]);

  // Copying more than part of one line puts whole lines on the clipboard, in
  // the format of the transcript file: `[00:06:40] Them: …`. What the browser
  // would copy on its own has the times, names and text on separate lines and
  // leaves the name out of every line but the first of a turn.
  useWindowEvent("copy", (event) => {
    const element = list.current;
    const selection = document.getSelection();
    if (element === null || selection === null || selection.isCollapsed) return;
    // A copy made in a text field or in the notes editor is not this one.
    const active = document.activeElement;
    if (active !== null && isTextEntry(active)) return;

    const range = selection.getRangeAt(0);
    const selected = [
      ...element.querySelectorAll<HTMLElement>("[data-line]"),
    ].filter((line) => range.intersectsNode(line));
    if (selected.length === 0) return;
    // A few words out of one line are copied as they are.
    const text = selected[0].querySelector("[data-text]");
    if (
      selected.length === 1 &&
      text !== null &&
      text.contains(range.startContainer) &&
      text.contains(range.endContainer)
    ) {
      return;
    }
    if (event.clipboardData === null) return;
    event.clipboardData.setData(
      "text/plain",
      selected
        .map((line) => transcriptLineText(lines[Number(line.dataset.line)]))
        .join("\n"),
    );
    event.preventDefault();
  });

  return (
    <ol
      ref={list}
      className={cx(styles.turns, "selectable")}
      aria-label="Transcript lines"
      data-hours={hasHours || undefined}
      // Reachable with Tab, so the arrow keys can scroll it.
      tabIndex={0}
    >
      {turns.map((turn) => (
        <li
          key={turn.start}
          className={styles.turn}
          data-speaker={turn.speaker}
        >
          <div className={styles.speaker}>{turn.speaker}</div>
          {turn.lines.map((line, offset) => (
            <p
              key={offset}
              className={styles.line}
              data-line={turn.start + offset}
            >
              <span className={styles.time}>{shortTimestamp(line.t)}</span>
              <span className={styles.text} data-text="">
                {line.text}
              </span>
            </p>
          ))}
        </li>
      ))}
    </ol>
  );
});

function isTextEntry(element: Element): boolean {
  return (
    element.matches("input, textarea, select, [contenteditable]") ||
    // The notes editor keeps its text in a shadow root.
    element.shadowRoot !== null
  );
}
