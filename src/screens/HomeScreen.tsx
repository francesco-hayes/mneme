import { useState } from "react";
import {
  Button,
  EmptyState,
  InlineError,
  List,
  ListRow,
  Screen,
} from "../components";
import { api } from "../lib/api";
import { errorMessage } from "../lib/errors";
import { useLoad } from "../lib/useLoad";
import { useNavigation } from "../state/navigation";
import { useRecording } from "../state/recording";
import styles from "./placeholder.module.css";

const loadMeetings = () => api.listMeetings();

/**
 * Placeholder for issue 016. It has just enough to move around the shell: a
 * bare list of meetings to open, and a New meeting button that starts a
 * recording so the recording bar can be seen.
 */
export function HomeScreen() {
  const { navigate } = useNavigation();
  const { isActive } = useRecording();
  const meetings = useLoad(loadMeetings);
  const [startError, setStartError] = useState<string | null>(null);

  async function startMeeting() {
    setStartError(null);
    try {
      const meeting = await api.startMeeting();
      navigate({ name: "meeting", meetingId: meeting.id });
    } catch (error) {
      setStartError(errorMessage(error));
    }
  }

  const newMeeting = (
    <Button variant="primary" disabled={isActive} onClick={startMeeting}>
      New meeting
    </Button>
  );

  return (
    <Screen title="Meetings" actions={newMeeting}>
      <p className={styles.note}>
        Placeholder. Issue 016 builds the Home screen.
      </p>
      {startError !== null && (
        <InlineError className={styles.error} message={startError} />
      )}
      {meetings.status === "error" && (
        <InlineError
          message={`Could not load meetings: ${meetings.message}`}
          onRetry={meetings.reload}
        />
      )}
      {meetings.status === "ready" && meetings.data.length === 0 && (
        <EmptyState
          title="No meetings yet"
          description="A meeting you record shows up here."
        />
      )}
      {meetings.status === "ready" && meetings.data.length > 0 && (
        <List aria-label="Meetings">
          {meetings.data.map((meeting) => (
            <ListRow
              key={meeting.id}
              title={meeting.title}
              onActivate={() =>
                navigate({ name: "meeting", meetingId: meeting.id })
              }
            />
          ))}
        </List>
      )}
    </Screen>
  );
}
