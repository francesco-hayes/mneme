import { cx } from "../lib/cx";
import { formatBytes } from "../lib/format";
import type { DownloadStatus } from "../lib/models";
import styles from "./DownloadProgress.module.css";

export interface DownloadProgressProps {
  /** What is being downloaded, for the bar's accessible name: `Small`. */
  name: string;
  /** From `useModelLibrary().downloads`. */
  status: DownloadStatus;
  className?: string;
}

function progressText({ phase, receivedBytes, totalBytes }: DownloadStatus) {
  switch (phase) {
    case "starting":
      return "Starting…";
    case "verifying":
      return "Verifying…";
    case "cancelling":
      return "Cancelling…";
    default:
      return totalBytes > 0
        ? `${formatBytes(receivedBytes)} of ${formatBytes(totalBytes)}`
        : formatBytes(receivedBytes);
  }
}

/**
 * A download's bar and the words under it: `312 MB of 1.53 GB` while bytes
 * arrive, then `Verifying…`; `Starting…` before the first byte and
 * `Cancelling…` after Cancel. Renders nothing for a failed download: the
 * caller shows the error.
 *
 * The bar is a `<progress>` element, whose fill comes from its `value` and
 * `max` attributes. Nothing here sets a width from script, which the app's
 * `style-src 'self'` policy would block.
 */
export function DownloadProgress({
  name,
  status,
  className,
}: DownloadProgressProps) {
  if (status.phase === "error") return null;
  // Without a value the bar is indeterminate: nothing is known yet.
  const known = status.phase !== "starting" && status.totalBytes > 0;

  return (
    <div className={cx(styles.progress, className)} data-phase={status.phase}>
      <progress
        className={styles.bar}
        aria-label={`Downloading ${name}`}
        max={known ? status.totalBytes : undefined}
        value={
          known ? Math.min(status.receivedBytes, status.totalBytes) : undefined
        }
      />
      <span className={styles.text}>{progressText(status)}</span>
    </div>
  );
}
