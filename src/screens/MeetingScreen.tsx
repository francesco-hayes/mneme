import { useCallback } from "react";
import { EmptyState, InlineError, Screen } from "../components";
import { api } from "../lib/api";
import { useLoad } from "../lib/useLoad";

/**
 * Placeholder for issues 017 (live view and editor) and 018 (after view and
 * transcript pane). It shows the meeting's title and nothing else.
 */
export function MeetingScreen({ meetingId }: { meetingId: string }) {
  const load = useCallback(() => api.getMeeting(meetingId), [meetingId]);
  const meeting = useLoad(load);

  return (
    <Screen title={meeting.status === "ready" ? meeting.data.title : "Meeting"}>
      {meeting.status === "error" && (
        <InlineError
          message={`Could not open this meeting: ${meeting.message}`}
          onRetry={meeting.reload}
        />
      )}
      {meeting.status === "ready" && (
        <EmptyState
          title="Notes go here"
          description="Placeholder. Issue 017 builds the live view and the editor, and issue 018 the after view."
        />
      )}
    </Screen>
  );
}
