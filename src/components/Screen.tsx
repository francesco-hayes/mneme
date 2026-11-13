import type { ReactNode } from "react";
import { cx } from "../lib/cx";
import { useNavigation } from "../state/navigation";
import { IconButton } from "./IconButton";
import { ChevronLeftIcon, GearIcon } from "./icons";
import styles from "./Screen.module.css";

export interface ScreenProps {
  /**
   * The screen's title in the top bar. A string becomes the page heading; pass
   * an element (a title field, say) to put something else in its place.
   */
  title: ReactNode;
  /** Controls for this screen, at the end of the top bar before Settings. */
  actions?: ReactNode;
  /** Whether to show the Back button. Default: whenever there is a screen to go back to. */
  back?: boolean;
  /** Whether to show the Settings button. Default true. */
  settings?: boolean;
  /**
   * `column` (default) centers the content in a readable, scrolling column.
   * `fill` gives the content the whole area under the top bar with no padding
   * and no scrolling of its own, for an editor or a split view.
   */
  layout?: "column" | "fill";
  children: ReactNode;
}

/**
 * The frame of every screen: the top bar (Back, title, actions, Settings) and
 * the content area under it. Each screen renders one `Screen` as its root.
 */
export function Screen({
  title,
  actions,
  back,
  settings = true,
  layout = "column",
  children,
}: ScreenProps) {
  const navigation = useNavigation();
  const showBack = back ?? navigation.canGoBack;

  return (
    <div className={styles.screen}>
      <header className={styles.topBar}>
        {showBack && (
          <IconButton label="Back" onClick={navigation.back}>
            <ChevronLeftIcon />
          </IconButton>
        )}
        <div className={styles.titleArea}>
          {typeof title === "string" ? (
            <h1 className={styles.title}>{title}</h1>
          ) : (
            title
          )}
        </div>
        <div className={styles.actions}>
          {actions}
          {settings && (
            <IconButton
              label="Settings"
              onClick={() => navigation.navigate({ name: "settings" })}
            >
              <GearIcon />
            </IconButton>
          )}
        </div>
      </header>
      <main className={cx(styles.content, styles[layout])}>
        {layout === "column" ? (
          <div className={styles.columnInner}>{children}</div>
        ) : (
          children
        )}
      </main>
    </div>
  );
}
