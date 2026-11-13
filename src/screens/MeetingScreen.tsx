import { useCallback, useRef, useState } from "react";
import { InlineError, Screen } from "../components";
import type { NotesEditorHandle } from "../editor/NotesEditor";
import { api } from "../lib/api";
import { useLoad } from "../lib/useLoad";
import { useRecording } from "../state/recording";
import { MeetingNotes } from "./meeting/MeetingNotes";
import notices from "./meeting/notices.module.css";
import { TitleField } from "./meeting/TitleField";
import styles from "./MeetingScreen.module.css";

/**
 * One meeting: its title in the top bar and its notes filling the window.
 *
 * While the meeting is being recorded this is the live view, and it is kept
 * quiet: Back, the title, the editor, and the shell's recording bar under
 * them. There is no transcript, no counter and nothing that moves. Stop in
 * the recording bar saves the notes first and leaves this view where it is.
 */
export function MeetingScreen({ meetingId }: { meetingId: string }) {
  const load = useCallback(() => api.getMeeting(meetingId), [meetingId]);
  const meeting = useLoad(load);
  const { state: recording, isActive } = useRecording();
  const isLive = isActive && recording.meetingId === meetingId;

  const [renameError, setRenameError] = useState<string | null>(null);
  const editor = useRef<NotesEditorHandle>(null);

  if (meeting.status === "error") {
    return (
      <Screen title="Meeting">
        <InlineError
          className={styles.loadError}
          message={`Could not open this meeting: ${meeting.message}`}
          onRetry={meeting.reload}
        />
      </Screen>
    );
  }

  return (
    <Screen
      layout="fill"
      // Nothing to wander off to in the middle of a meeting.
      settings={!isLive}
      title={
        meeting.status === "ready" && (
          <TitleField
            meetingId={meetingId}
            initialTitle={meeting.data.title}
            onError={setRenameError}
            onDone={() => editor.current?.focus()}
          />
        )
      }
    >
      {meeting.status === "ready" && (
        <div className={styles.body}>
          <section className={styles.notes} aria-label="Notes">
            {renameError !== null && (
              <InlineError
                className={notices.notice}
                message={`Could not rename the meeting: ${renameError}`}
              />
            )}
            <MeetingNotes
              meeting={meeting.data}
              // A meeting that is being recorded is one to write in.
              autoFocus={meeting.data.status === "recording"}
              editorRef={editor}
            />
          </section>
        </div>
      )}
    </Screen>
  );
}
