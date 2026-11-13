import { type ComponentProps, type ReactNode, useId } from "react";
import type { ModelInfo } from "../lib/api";
import { cx } from "../lib/cx";
import { formatBytes } from "../lib/format";
import { type DownloadStatus, modelNote } from "../lib/models";
import { Button } from "./Button";
import { DownloadProgress } from "./DownloadProgress";
import { CheckIcon, WarningIcon } from "./icons";
import styles from "./ModelRow.module.css";

/**
 * The box that holds `ModelRow`s: a list drawn as one rounded group. Takes
 * every `<ul>` attribute; give it an `aria-label`.
 */
export function ModelList({ className, ...rest }: ComponentProps<"ul">) {
  return <ul className={cx(styles.list, className)} {...rest} />;
}

export interface ModelRowProps {
  model: ModelInfo;
  /** This model's entry in `useModelLibrary().downloads`, if it has one. */
  download?: DownloadStatus;
  /** Something before the name: a radio button that picks the model, say. */
  leading?: ReactNode;
  /** A word after the name, drawn as a tag: `Default`, `Recommended`. */
  badge?: ReactNode;
  /**
   * Starts the download. Shown as Download on a model that is not installed,
   * and as Retry after a failure. Leave it out for a row that cannot download.
   */
  onDownload?: () => void;
  /** Cancels a running download. */
  onCancelDownload?: () => void;
  /** Forgets a failed download. */
  onDismissError?: () => void;
  /**
   * Controls for an installed model, at the end of the row (Settings puts
   * Delete there). Without it an installed model says "Installed"; `false`
   * leaves the place empty.
   */
  installedActions?: ReactNode;
  /** More for this model under its row, at the row's full width: a confirmation, an error. */
  children?: ReactNode;
}

/**
 * One model: name, an English-only tag, the catalog's note and the file size,
 * with the controls and progress that fit its state:
 *
 * - not installed: Download
 * - starting, downloading, verifying: the bar and Cancel
 * - cancelling: the bar, grayed, and Cancel disabled
 * - failed: the reason, Retry and Dismiss
 * - installed: `installedActions`, or the word "Installed"
 *
 * It holds no state of its own. The download state comes from the model
 * library, so the row looks the same wherever it is shown and whenever the
 * user comes back to it.
 */
export function ModelRow({
  model,
  download,
  leading,
  badge,
  onDownload,
  onCancelDownload,
  onDismissError,
  installedActions,
  children,
}: ModelRowProps) {
  const nameId = useId();
  const failed = download?.phase === "error";
  const active = download !== undefined && !failed;
  const state = model.installed
    ? "installed"
    : (download?.phase ?? "available");

  let actions: ReactNode = null;
  if (model.installed) {
    actions = installedActions ?? (
      <span className={styles.installed}>
        <CheckIcon />
        Installed
      </span>
    );
  } else if (active) {
    actions = onCancelDownload && (
      <Button
        size="small"
        aria-label={`Cancel downloading ${model.name}`}
        // Nothing to cancel until the backend has the request, and nothing
        // more to ask for once Cancel has been pressed.
        disabled={
          download.phase === "starting" || download.phase === "cancelling"
        }
        onClick={onCancelDownload}
      >
        Cancel
      </Button>
    );
  } else if (failed) {
    actions = (
      <>
        {onDismissError && (
          <Button
            size="small"
            variant="plain"
            aria-label={`Dismiss the error for ${model.name}`}
            onClick={onDismissError}
          >
            Dismiss
          </Button>
        )}
        {onDownload && (
          <Button
            size="small"
            aria-label={`Retry downloading ${model.name}`}
            onClick={onDownload}
          >
            Retry
          </Button>
        )}
      </>
    );
  } else if (onDownload) {
    actions = (
      <Button
        size="small"
        aria-label={`Download ${model.name}`}
        onClick={onDownload}
      >
        Download
      </Button>
    );
  }

  return (
    <li className={styles.row} aria-labelledby={nameId} data-state={state}>
      <div className={styles.line}>
        {leading !== undefined && (
          <div className={styles.leading}>{leading}</div>
        )}
        <div className={styles.text}>
          <div className={styles.nameLine}>
            <span id={nameId} className={styles.name}>
              {model.name}
            </span>
            {model.englishOnly && (
              <span className={styles.tag}>English only</span>
            )}
            {badge !== undefined && badge !== null && badge !== false && (
              <span className={cx(styles.tag, styles.badge)}>{badge}</span>
            )}
          </div>
          <div className={styles.note}>{modelNote(model)}</div>
        </div>
        {/* While a download runs, its own line gives received and total. */}
        {!active && (
          <span className={styles.size}>{formatBytes(model.sizeBytes)}</span>
        )}
        {/* `false` keeps the slot, empty, so the size before it stays put
            while the row's controls are swapped for something under it. */}
        {actions !== null && actions !== undefined && (
          <div className={styles.actions}>{actions}</div>
        )}
      </div>
      {active && (
        <DownloadProgress
          className={styles.below}
          name={model.name}
          status={download}
        />
      )}
      {active && download.error !== null && (
        <p role="alert" className={cx(styles.below, styles.error)}>
          <WarningIcon />
          <span className="selectable">{download.error}</span>
        </p>
      )}
      {failed && (
        <p role="alert" className={cx(styles.below, styles.error)}>
          <WarningIcon />
          <span className="selectable">
            {`Could not download ${model.name}: ${download.error ?? "The download failed."}`}
          </span>
        </p>
      )}
      {children !== undefined && children !== null && children !== false && (
        <div className={styles.below}>{children}</div>
      )}
    </li>
  );
}
