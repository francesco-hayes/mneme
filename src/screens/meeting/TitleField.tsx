import { type KeyboardEvent, useEffect, useRef } from "react";
import { api } from "../../lib/api";
import { errorMessage } from "../../lib/errors";
import { onTitleSaved } from "./notesAutosave";
import styles from "./TitleField.module.css";

export interface TitleFieldProps {
  meetingId: string;
  /** The meeting's title when the view opened. Read once. */
  initialTitle: string;
  /** Called with why a rename failed, and with null when the field is taken up again. */
  onError: (message: string | null) => void;
  /** Called when Enter or Escape ends the editing, to send focus somewhere useful. */
  onDone?: () => void;
}

/**
 * The meeting's title in the top bar, editable in place at any time. It is
 * saved with `api.renameMeeting` on Enter and when the field loses focus.
 * Escape, an empty title and a failed rename all put the previous title back.
 * It never asks for a title: a meeting keeps its date-time title until the
 * user changes it, here or with a `# ` heading in the notes. A title that a
 * notes save brings is shown at once, unless the field is being edited or a
 * rename from it is on its way: then the user's title wins.
 */
export function TitleField({
  meetingId,
  initialTitle,
  onError,
  onDone,
}: TitleFieldProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  /** The title the backend has. */
  const saved = useRef(initialTitle);
  /** The title last sent to the backend, or `saved` when none is on its way. */
  const asked = useRef(initialTitle);
  /** Renames run one at a time, in the order they were made. */
  const queue = useRef(Promise.resolve());

  useEffect(
    () =>
      onTitleSaved(meetingId, (title) => {
        const input = inputRef.current;
        if (input === null || title === saved.current) return;
        if (document.activeElement === input) return;
        if (asked.current !== saved.current) return;
        saved.current = title;
        asked.current = title;
        input.value = title;
      }),
    [meetingId],
  );

  function commit() {
    const input = inputRef.current;
    if (input === null) return;
    const next = input.value.trim();
    if (next === "") {
      input.value = asked.current;
      return;
    }
    input.value = next;
    if (next === asked.current) return;

    asked.current = next;
    onError(null);
    queue.current = queue.current.then(async () => {
      // Leave the field alone if the user has typed something else meanwhile.
      const untouched = () => input.value.trim() === next;
      try {
        const summary = await api.renameMeeting(meetingId, next);
        saved.current = summary.title;
        if (asked.current === next) asked.current = summary.title;
        if (untouched()) input.value = summary.title;
      } catch (error) {
        onError(errorMessage(error));
        if (asked.current === next) asked.current = saved.current;
        if (untouched()) input.value = saved.current;
      }
    });
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    // Enter that confirms an input method's suggestion is not a commit.
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
      finish(event.currentTarget);
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.currentTarget.value = asked.current;
      finish(event.currentTarget);
    }
  }

  function finish(input: HTMLInputElement) {
    input.blur();
    onDone?.();
  }

  return (
    <h1 className={styles.heading}>
      <input
        ref={inputRef}
        className={styles.input}
        type="text"
        aria-label="Meeting title"
        defaultValue={initialTitle}
        spellCheck={false}
        autoComplete="off"
        // Coming back to the field to try another title: the old message goes.
        onFocus={() => onError(null)}
        onBlur={commit}
        onKeyDown={onKeyDown}
      />
    </h1>
  );
}
