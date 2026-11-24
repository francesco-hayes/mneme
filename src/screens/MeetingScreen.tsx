import { useCallback, useId, useRef, useState } from "react";
import {
  Button,
  InlineError,
  Screen,
  TranscriptIcon,
  WarningIcon,
} from "../components";
import type { NotesEditorHandle } from "../editor/NotesEditor";
import { api, type Meeting } from "../lib/api";
import { cx } from "../lib/cx";
import { errorMessage } from "../lib/errors";
import { useLoad } from "../lib/useLoad";
import { useWindowEvent } from "../lib/useWindowEvent";
import { useRecording } from "../state/recording";
import { MeetingNotes } from "./meeting/MeetingNotes";
import notices from "./meeting/notices.module.css";
import { TitleField } from "./meeting/TitleField";
import { TranscriptButton } from "./meeting/TranscriptButton";
import { TranscriptPane } from "./meeting/TranscriptPane";
import { isTranscriptShortcut } from "./meeting/transcriptShortcut";
import { useMeetingPhase } from "./meeting/useMeetingPhase";
import { useTranscript } from "./meeting/useTranscript";
import styles from "./MeetingScreen.module.css";

/**
 * One meeting: its title in the top bar and its notes filling the window.
 *
 * While the meeting is being recorded this is the live view, and it is kept
 * quiet: Back, the title, the editor, and the shell's recording bar under
 * them. There is no transcript, no counter and nothing that moves. Stop in
 * the recording bar saves the notes first and leaves this view where it is.
 *
 * After the meeting the layout is the same and the notes still have the whole
 * width. The top bar gains Show in Finder and the Transcript button, which
 * says how many lines there are and opens the transcript in a pane beside the
 * notes. The pane is closed every time a meeting is opened, and stays closed
 * when a recording stops.
 */
export function MeetingScreen({ meetingId }: { meetingId: string }) {
  const load = useCallback(() => api.getMeeting(meetingId), [meetingId]);
  const meeting = useLoad(load);
  const { state: recording, isActive } = useRecording();

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
  if (meeting.status === "loading") {
    const isLive = isActive && recording.meetingId === meetingId;
    return (
      <Screen layout="fill" settings={!isLive} title={null}>
        {null}
      </Screen>
    );
  }
  return <MeetingView opened={meeting.data} />;
}

/** The view of a meeting that has been read. `opened` is the copy it opened with. */
function MeetingView({ opened }: { opened: Meeting }) {
  const meetingId = opened.id;
  const { phase, isLive, meeting, lineCount } = useMeetingPhase(opened);
  const transcript = useTranscript(meetingId);
  const { state: recording } = useRecording();
  // A channel that is not being recorded (system audio refused, say): said
  // for as long as this meeting records.
  const channelWarning =
    isLive && recording.meetingId === meetingId ? recording.warning : null;

  // Closed whenever a meeting is opened, and nothing but the user opens it.
  const [paneOpen, setPaneOpen] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [finderError, setFinderError] = useState<string | null>(null);
  const editor = useRef<NotesEditorHandle>(null);
  const transcriptButton = useRef<HTMLButtonElement>(null);
  const transcriptScroll = useRef(0);
  const paneId = useId();

  const isAfter = phase === "after";
  const transcriptLoad = transcript.load;
  // Once the lines are here, their number is the one that counts.
  const knownLineCount =
    transcriptLoad.status === "ready" ? transcriptLoad.lines.length : lineCount;

  /** Only reachable after the meeting: from the button, and from the shortcut below. */
  function togglePane() {
    if (paneOpen) {
      // Focus that was in the pane would be lost with it.
      const pane = document.getElementById(paneId);
      if (pane !== null && pane.contains(document.activeElement)) {
        transcriptButton.current?.focus();
      }
      setPaneOpen(false);
    } else {
      transcript.ensureLoaded();
      setPaneOpen(true);
    }
  }

  // On the window, so it works wherever the focus is. A key pressed in the
  // editor reaches the window too: the editor's shadow root does not hold
  // keyboard events back.
  useWindowEvent("keydown", (event) => {
    if (!isAfter || !isTranscriptShortcut(event)) return;
    event.preventDefault();
    togglePane();
  });

  async function showInFinder() {
    setFinderError(null);
    try {
      await api.showInFinder(meetingId);
    } catch (error) {
      setFinderError(errorMessage(error));
    }
  }

  return (
    <Screen
      layout="fill"
      // Nothing to wander off to in the middle of a meeting.
      settings={!isLive}
      title={
        <TitleField
          meetingId={meetingId}
          initialTitle={opened.title}
          onError={setRenameError}
          onDone={() => editor.current?.focus()}
        />
      }
      actions={
        phase === "finishing" ? (
          // Stop was pressed and the last of the audio is being transcribed:
          // the transcript is on its way, and not to be opened yet.
          <Button disabled>
            <TranscriptIcon />
            Finishing transcript…
          </Button>
        ) : (
          isAfter && (
            <>
              <Button variant="plain" onClick={() => void showInFinder()}>
                Show in Finder
              </Button>
              <TranscriptButton
                ref={transcriptButton}
                lineCount={knownLineCount}
                expanded={paneOpen}
                controls={paneId}
                onClick={togglePane}
              />
            </>
          )
        )
      }
    >
      <div className={styles.body}>
        <section className={styles.notes} aria-label="Notes">
          {renameError !== null && (
            <InlineError
              className={notices.notice}
              message={`Could not rename the meeting: ${renameError}`}
            />
          )}
          {finderError !== null && (
            <InlineError
              className={notices.notice}
              message={`Could not show the meeting in Finder: ${finderError}`}
            />
          )}
          {channelWarning !== null && (
            <div role="status" className={cx(notices.notice, notices.warning)}>
              <WarningIcon />
              <span className="selectable">{channelWarning}</span>
            </div>
          )}
          {isAfter && meeting.status === "interrupted" && (
            <p role="note" className={cx(notices.notice, notices.quiet)}>
              The recording ended unexpectedly. The transcript may be
              incomplete.
            </p>
          )}
          <MeetingNotes
            meeting={opened}
            // A meeting that is being recorded is one to write in.
            autoFocus={opened.status === "recording"}
            editorRef={editor}
          />
        </section>
        {isAfter && paneOpen && (
          <TranscriptPane
            id={paneId}
            load={transcriptLoad}
            onRetry={transcript.retry}
            durationSecs={meeting.durationSecs}
            scrollMemo={transcriptScroll}
          />
        )}
      </div>
    </Screen>
  );
}
