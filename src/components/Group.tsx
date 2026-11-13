import { type ComponentProps, type ReactNode, useId } from "react";
import { cx } from "../lib/cx";
import styles from "./Group.module.css";

export interface SectionProps {
  title: string;
  /** One quiet line under the title. */
  description?: ReactNode;
  children: ReactNode;
}

/** A titled part of a settings-style screen: a heading over one or more groups. */
export function Section({ title, description, children }: SectionProps) {
  const titleId = useId();
  return (
    <section className={styles.section} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.sectionTitle}>
        {title}
      </h2>
      {description !== undefined && (
        <p className={styles.sectionDescription}>{description}</p>
      )}
      {children}
    </section>
  );
}

/**
 * The rounded box that holds a group of rows, as in a Mac settings window.
 * Takes every `<div>` attribute. Rows inside it are separated by hairlines.
 */
export function Group({ className, ...rest }: ComponentProps<"div">) {
  return <div className={cx(styles.group, className)} {...rest} />;
}

export interface GroupRowProps {
  /** What the row sets, on the left. */
  label: ReactNode;
  /** The id to give the label, so the control can point at it with `aria-labelledby`. */
  labelId?: string;
  /** A quieter line under the label. */
  description?: ReactNode;
  /** The control, on the right. */
  children: ReactNode;
  /** Anything that belongs to the row and needs its full width: a warning, an error. */
  below?: ReactNode;
}

/** One setting inside a `Group`: its name on the left, its control on the right. */
export function GroupRow({
  label,
  labelId,
  description,
  children,
  below,
}: GroupRowProps) {
  return (
    <div className={styles.row}>
      <div className={styles.rowLine}>
        <div className={styles.rowText}>
          <div id={labelId} className={styles.rowLabel}>
            {label}
          </div>
          {description !== undefined && (
            <div className={styles.rowDescription}>{description}</div>
          )}
        </div>
        <div className={styles.rowControl}>{children}</div>
      </div>
      {below}
    </div>
  );
}
