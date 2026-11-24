import { type Ref, useEffect, useState, useSyncExternalStore } from "react";
import { Button, WarningIcon } from "../../components";
import { NotesEditor, type NotesEditorHandle } from "../../editor/NotesEditor";
import type { Meeting } from "../../lib/api";
import { cx } from "../../lib/cx";
import { useWindowEvent } from "../../lib/useWindowEvent";
import { useModelLibrary } from "../../state/modelLibrary";
import { useBeforeStop } from "../../state/recording";
import { notesAutosave } from "./notesAutosave";
import notices from "./notices.module.css";

/** The longest Stop is held up by a save of the notes. */
const STOP_WAIT_MS = 3000;

const wait = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

export interface MeetingNotesProps {
  /**
   * The meeting whose notes to edit. Its id and notes are read once, when
   * this mounts; a newer copy of the same meeting does not reset the editor.
   */
  meeting: Meeting;
  /** Puts the cursor in the editor when it opens. */
  autoFocus?: boolean;
  editorRef?: Ref<NotesEditorHandle>;
}

/**
 * The notes of one meeting: the editor, and saving what is typed in it.
 *
 * Notes are saved a little after typing pauses, when the editor or the window
 * loses focus, when this view is left, and before a recording is stopped. A
 * save that fails keeps the text, says so above the editor, and is tried again
 * until it works.
 */
export function MeetingNotes({
  meeting,
  autoFocus,
  editorRef,
}: MeetingNotesProps) {
  const [{ saver, initialText }] = useState(() => {
    const saver = notesAutosave(meeting.id, meeting.notes);
    // Usually `meeting.notes`. Newer, when an earlier visit to this meeting
    // left text that has not reached the backend yet.
    return { saver, initialText: saver.text };
  });
  const status = useSyncExternalStore(saver.subscribe, () => saver.status);
  const spellcheck = useSpellcheckSetting();

  // Stop waits for this, so the notes are on disk before the meeting ends.
  // It does not wait for long: a save that hangs must not keep the microphone
  // on, and it carries on after the stop anyway.
  useBeforeStop(() => Promise.race([saver.flush(), wait(STOP_WAIT_MS)]));
  // Leaving the view. The save carries on after the view is gone.
  useEffect(() => () => void saver.flush(), [saver]);
  // The window is closing or reloading: the last chance to send the text.
  useWindowEvent("pagehide", () => void saver.flush());

  return (
    <>
      {status.state === "failed" && (
        <div role="status" className={cx(notices.notice, notices.warning)}>
          <WarningIcon />
          <span className="selectable">
            Notes not saved yet, still trying: {status.message}
          </span>
          <Button
            variant="plain"
            size="small"
            onClick={() => void saver.flush()}
          >
            Try now
          </Button>
        </div>
      )}
      <NotesEditor
        ref={editorRef}
        initialText={initialText}
        onChange={(text) => saver.change(text)}
        onBlur={() => void saver.flush()}
        autoFocus={autoFocus}
        label="Notes"
        placeholder="Notes"
        spellcheck={spellcheck}
      />
    </>
  );
}

/**
 * The spellcheck setting, off until the settings have been read. The settings
 * live in the model library, which only reads when asked: a meeting opened
 * before Settings was ever shown asks once.
 */
function useSpellcheckSetting(): boolean {
  const { status, settings, load } = useModelLibrary();
  useEffect(() => {
    if (status === "idle") void load();
  }, [status, load]);
  return settings?.spellcheck ?? false;
}
