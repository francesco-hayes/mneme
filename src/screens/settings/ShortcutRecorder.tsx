import { type KeyboardEvent, useId, useRef, useState } from "react";
import { Button, InlineError } from "../../components";
import { cx } from "../../lib/cx";
import {
  captureShortcut,
  describeShortcut,
  modifierSymbols,
  type ShortcutModifier,
} from "../../lib/shortcut";
import styles from "./ShortcutRecorder.module.css";

export interface ShortcutRecorderProps {
  /** The saved shortcut string, or null for none. */
  value: string | null;
  /** Saves a new shortcut, or null to clear it. */
  onChange: (value: string | null) => void;
  /** A save is under way: the field shows the new value and waits. */
  saving?: boolean;
  /** Why the backend refused the last change. `value` is then the previous shortcut again. */
  error?: string | null;
}

const RECORDING_HELP = "Press a key combination. Esc cancels.";

/**
 * The field that records the global shortcut. It shows the shortcut in Mac
 * symbols, or "None". Click it (or press Space or Return on it), then press a
 * combination: that is saved at once. Escape, clicking again, or moving the
 * focus away stops recording and keeps what was set. Tab is never taken, so
 * the keyboard focus can always leave the field.
 */
export function ShortcutRecorder({
  value,
  onChange,
  saving = false,
  error = null,
}: ShortcutRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState<ShortcutModifier[]>([]);
  const [rejection, setRejection] = useState<string | null>(null);
  const field = useRef<HTMLButtonElement>(null);
  const helpId = useId();

  function stopRecording() {
    setRecording(false);
    setHeld([]);
    setRejection(null);
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (!recording) return;
    const capture = captureShortcut(event.nativeEvent);
    // Tab goes on to move the focus; leaving the field ends the recording.
    if (capture.kind === "pass") return;

    // Everything else belongs to the recorder: the key must not click the
    // field, scroll the screen or reach a shortcut of the app.
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;

    switch (capture.kind) {
      case "cancel":
        stopRecording();
        break;
      case "modifiers":
        setHeld(capture.modifiers);
        break;
      case "rejected":
        setHeld([]);
        setRejection(capture.reason);
        break;
      case "shortcut":
        stopRecording();
        if (capture.value !== value) onChange(capture.value);
        break;
    }
  }

  function onKeyUp(event: KeyboardEvent<HTMLButtonElement>) {
    if (!recording) return;
    // Space clicks a button when it is released.
    event.preventDefault();
    setHeld(captureHeld(event));
  }

  const label = value === null ? null : describeShortcut(value);
  const shown = recording
    ? held.length > 0
      ? modifierSymbols(held)
      : "Type shortcut"
    : value === null
      ? "None"
      : (label?.symbols ?? value);
  const spoken = recording
    ? "recording, press the keys"
    : value === null
      ? "None"
      : (label?.spoken ?? value);

  return (
    <div className={styles.recorder}>
      <div className={styles.controls}>
        {value !== null && !recording && (
          <Button
            size="small"
            variant="plain"
            aria-label="Clear the shortcut"
            onClick={() => {
              if (saving) return;
              onChange(null);
              // Clear is about to go; the focus stays with the field.
              field.current?.focus();
            }}
          >
            Clear
          </Button>
        )}
        <button
          ref={field}
          type="button"
          className={cx(
            styles.field,
            recording && styles.recording,
            value === null && !recording && styles.empty,
          )}
          aria-label={`Global shortcut: ${spoken}`}
          aria-describedby={recording ? helpId : undefined}
          aria-pressed={recording}
          aria-busy={saving}
          title={
            recording
              ? undefined
              : "Click, then press the keys for the shortcut"
          }
          onClick={(event) => {
            if (recording) {
              stopRecording();
              return;
            }
            // Not disabled while a save runs, so the focus is not thrown out
            // of the field; a click in that moment does nothing.
            if (saving) return;
            // WebKit does not focus a button when it is clicked, and the keys
            // go to whatever has the focus.
            event.currentTarget.focus();
            setRecording(true);
          }}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
          onBlur={() => {
            if (recording) stopRecording();
          }}
        >
          {shown}
        </button>
      </div>
      {recording && (
        <p
          id={helpId}
          role="status"
          className={cx(styles.help, rejection !== null && styles.rejected)}
        >
          {rejection ?? RECORDING_HELP}
        </p>
      )}
      {!recording && error !== null && (
        <InlineError
          className={styles.error}
          message={`Could not set the shortcut: ${error}`}
        />
      )}
    </div>
  );
}

function captureHeld(event: KeyboardEvent<HTMLButtonElement>) {
  const capture = captureShortcut(event.nativeEvent);
  return capture.kind === "modifiers" ? capture.modifiers : [];
}
