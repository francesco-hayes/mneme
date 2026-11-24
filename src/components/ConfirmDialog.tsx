import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
} from "react";
import { createPortal } from "react-dom";
import { Button } from "./Button";
import styles from "./ConfirmDialog.module.css";
import { InlineError } from "./InlineError";

export interface ConfirmDialogProps {
  /** The question, in a sentence: "Move “Standup” to the Trash?" */
  title: string;
  /** What happens, under the title. */
  message?: ReactNode;
  /** The button that does it: "Move to Trash". */
  confirmLabel: string;
  /** Shown on that button while `busy`: "Moving…". */
  busyLabel?: string;
  cancelLabel?: string;
  /** Draws the confirm button in the danger color. */
  danger?: boolean;
  /** The action is running: both buttons are disabled and Escape does nothing. */
  busy?: boolean;
  /** Why the action failed. The dialog stays open, so it can be tried again. */
  error?: string | null;
  /** The confirm button is off, for example while a name field is empty. */
  confirmDisabled?: boolean;
  /**
   * Controls between the message and the buttons: a name field, a list to
   * choose from. With content, the focus starts on its first field instead
   * of on Cancel.
   */
  children?: ReactNode;
  onConfirm: () => void;
  onCancel: () => void;
}

const FOCUSABLE = "button:not(:disabled), input:not(:disabled)";
const FIELD = "input:not(:disabled)";

/**
 * A modal question over the whole window, for an action that needs a yes
 * first. Render it to open it, stop rendering it to close it.
 *
 * Cancel has the focus when it opens, so Return does not confirm by accident;
 * with fields in `children`, the first field (a checked radio button first)
 * has it instead.
 * Tab stays inside, Escape cancels, and the rest of the window is inert
 * meanwhile. On close the focus goes back to what had it before.
 */
export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  busyLabel,
  cancelLabel = "Cancel",
  danger = false,
  busy = false,
  error = null,
  confirmDisabled = false,
  children,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const titleId = useId();
  const messageId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const app = document.getElementById("root");
    app?.setAttribute("inert", "");
    const field =
      dialog.current?.querySelector<HTMLInputElement>(
        `${FIELD}:checked, input[type="text"]:not(:disabled)`,
      ) ?? dialog.current?.querySelector<HTMLInputElement>(FIELD);
    if (field) {
      field.focus();
      if (field.type === "text") field.select();
    } else {
      cancelButton.current?.focus();
    }
    return () => {
      app?.removeAttribute("inert");
      if (opener?.isConnected) opener.focus();
    };
  }, []);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    // Keys pressed here are the dialog's: the window's shortcuts and the
    // list's arrow keys must not also act on them.
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      if (!busy) onCancel();
    } else if (event.key === "Tab") {
      const controls = Array.from(
        dialog.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [],
      );
      if (controls.length === 0) {
        event.preventDefault();
        return;
      }
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  }

  return createPortal(
    <div className={styles.backdrop}>
      <div
        ref={dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={message !== undefined ? messageId : undefined}
        className={styles.dialog}
        onKeyDown={onKeyDown}
      >
        <h2 id={titleId} className={styles.title}>
          {title}
        </h2>
        {message !== undefined && (
          <p id={messageId} className={styles.message}>
            {message}
          </p>
        )}
        {children}
        {error !== null && <InlineError message={error} />}
        <div className={styles.buttons}>
          <Button ref={cancelButton} disabled={busy} onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button
            variant={danger ? "danger" : "primary"}
            disabled={busy || confirmDisabled}
            onClick={onConfirm}
          >
            {busy && busyLabel !== undefined ? busyLabel : confirmLabel}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
