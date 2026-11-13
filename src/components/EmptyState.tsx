import type { ReactNode } from "react";
import styles from "./EmptyState.module.css";

export interface EmptyStateProps {
  title: string;
  description?: ReactNode;
  /** Usually one button: the way out of the empty state. */
  action?: ReactNode;
}

/** What a screen shows in place of content it does not have yet. */
export function EmptyState({ title, description, action }: EmptyStateProps) {
  return (
    <div className={styles.emptyState}>
      <h2 className={styles.title}>{title}</h2>
      {description !== undefined && (
        <p className={styles.description}>{description}</p>
      )}
      {action !== undefined && <div className={styles.action}>{action}</div>}
    </div>
  );
}
