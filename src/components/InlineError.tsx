import { cx } from "../lib/cx";
import { Button } from "./Button";
import { WarningIcon } from "./icons";
import styles from "./InlineError.module.css";

export interface InlineErrorProps {
  message: string;
  /** Adds a Retry button that calls this. */
  onRetry?: () => void;
  className?: string;
}

/**
 * An error shown in place, next to what failed. The app never uses a popup
 * for errors. Announced by screen readers when it appears.
 */
export function InlineError({ message, onRetry, className }: InlineErrorProps) {
  return (
    <div role="alert" className={cx(styles.inlineError, className)}>
      <WarningIcon />
      <span className={cx(styles.message, "selectable")}>{message}</span>
      {onRetry && (
        <Button variant="plain" size="small" onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  );
}
