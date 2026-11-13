import type { Ref } from "react";
import { Button, TranscriptIcon } from "../../components";
import { formatLineCount } from "../../lib/format";
import styles from "./TranscriptButton.module.css";

export interface TranscriptButtonProps {
  /** Lines in the transcript, or null when that is not known. */
  lineCount: number | null;
  /** Whether the pane is open. */
  expanded: boolean;
  /** The id of the pane. */
  controls: string;
  onClick: () => void;
  ref?: Ref<HTMLButtonElement>;
}

/**
 * The top bar's way to the transcript. Its line count says the transcript is
 * there and ready ("Transcript · 214 lines"); pressing it opens and closes
 * the pane.
 */
export function TranscriptButton({
  lineCount,
  expanded,
  controls,
  onClick,
  ref,
}: TranscriptButtonProps) {
  return (
    <Button
      ref={ref}
      className={styles.toggle}
      aria-expanded={expanded}
      aria-controls={expanded ? controls : undefined}
      aria-keyshortcuts="Meta+Shift+T"
      title={`${expanded ? "Hide" : "Show"} transcript (⇧⌘T)`}
      onClick={onClick}
    >
      <TranscriptIcon />
      <span>
        Transcript
        {lineCount !== null && (
          <span className={styles.count}> · {formatLineCount(lineCount)}</span>
        )}
      </span>
    </Button>
  );
}
