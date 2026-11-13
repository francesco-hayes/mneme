import type { ComponentProps, ReactNode } from "react";
import { cx } from "../lib/cx";
import styles from "./List.module.css";
import { moveListFocus } from "./listFocus";

/**
 * A list of `ListRow`s. Takes every `<ul>` attribute. With focus on a row that
 * opens something, the up and down arrow keys move to the row before or after
 * it, and Enter opens it.
 */
export function List({ className, onKeyDown, ...rest }: ComponentProps<"ul">) {
  return (
    <ul
      className={cx(styles.list, className)}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (event.defaultPrevented) return;
        if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
          return;
        }
        const by =
          event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : null;
        if (by !== null && moveListFocus(event.currentTarget, by)) {
          // Keeps the arrow key from also scrolling the screen.
          event.preventDefault();
        }
      }}
      {...rest}
    />
  );
}

export interface ListRowProps {
  title: ReactNode;
  /** A second, quieter line under the title. */
  detail?: ReactNode;
  /** Short text at the end of the row, such as a date or a size. */
  meta?: ReactNode;
  /** Controls at the end of the row, such as a Delete button. */
  trailing?: ReactNode;
  /**
   * Makes the row something to open: the title, detail and meta become one
   * button. Controls in `trailing` stay separate from it.
   */
  onActivate?: () => void;
}

export function ListRow({
  title,
  detail,
  meta,
  trailing,
  onActivate,
}: ListRowProps) {
  const body = (
    <>
      <span className={styles.text}>
        <span className={styles.title}>{title}</span>
        {detail !== undefined && (
          <span className={styles.detail}>{detail}</span>
        )}
      </span>
      {meta !== undefined && <span className={styles.meta}>{meta}</span>}
    </>
  );

  return (
    <li className={styles.row}>
      {onActivate ? (
        <button
          type="button"
          className={cx(styles.main, styles.action)}
          onClick={onActivate}
        >
          {body}
        </button>
      ) : (
        <div className={styles.main}>{body}</div>
      )}
      {trailing !== undefined && (
        <div className={styles.trailing}>{trailing}</div>
      )}
    </li>
  );
}
