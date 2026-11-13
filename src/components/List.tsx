import type { ComponentProps, ReactNode } from "react";
import { cx } from "../lib/cx";
import styles from "./List.module.css";

/** A list of `ListRow`s. Takes every `<ul>` attribute. */
export function List({ className, ...rest }: ComponentProps<"ul">) {
  return <ul className={cx(styles.list, className)} {...rest} />;
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
