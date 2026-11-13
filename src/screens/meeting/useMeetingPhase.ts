import { useEffect, useState } from "react";
import { api, type Meeting } from "../../lib/api";
import { useRecording } from "../../state/recording";

/**
 * Where a meeting is in its life, as far as the meeting view cares:
 *
 * - `live`: being recorded. The view shows nothing about the transcript.
 * - `finishing`: Stop was pressed and the transcript is not complete yet.
 * - `after`: the meeting is over and its transcript can be read.
 */
export type MeetingPhase = "live" | "finishing" | "after";

export interface MeetingPhaseResult {
  phase: MeetingPhase;
  /** True while this meeting is being recorded or is stopping. */
  isLive: boolean;
  /**
   * The newest copy of the meeting: the one read again when its recording
   * ended, or else the one the view opened with.
   */
  meeting: Meeting;
  /**
   * How many lines the transcript has, or null when that is not known: the
   * recording ended while the view was open and the meeting could not be
   * read again.
   */
  lineCount: number | null;
}

/**
 * Follows a meeting from recording to finished while its view stays open.
 *
 * When the recording of this meeting ends (Stop in the recording bar, or a
 * stop from anywhere else), the meeting is read again for its final status,
 * duration and line count. Only this hook's result changes: the view is not
 * reloaded, so the editor and whatever is typed in it stay as they are.
 */
export function useMeetingPhase(opened: Meeting): MeetingPhaseResult {
  const meetingId = opened.id;
  const { state, isActive, isStopping } = useRecording();
  const isLive = isActive && state.meetingId === meetingId;

  // Set once this view has seen the meeting being recorded.
  const [sawLive, setSawLive] = useState(false);
  if (isLive && !sawLive) setSawLive(true);

  /** What reading the meeting again gave, once its recording had ended. */
  const [ended, setEnded] = useState<{ meeting: Meeting | null } | null>(null);
  const hasEnded = sawLive && !isLive;
  useEffect(() => {
    if (!hasEnded) return;
    let current = true;
    api.getMeeting(meetingId).then(
      (meeting) => {
        if (current) setEnded({ meeting });
      },
      () => {
        // The view carries on with what it knows: the meeting is over.
        if (current) setEnded({ meeting: null });
      },
    );
    return () => {
      current = false;
    };
  }, [hasEnded, meetingId]);

  if (isLive) {
    return {
      phase: isStopping ? "finishing" : "live",
      isLive,
      meeting: opened,
      lineCount: null,
    };
  }
  if (sawLive) {
    // Between the end of the recording and the fresh copy of the meeting.
    if (ended === null) {
      return { phase: "finishing", isLive, meeting: opened, lineCount: null };
    }
    return {
      phase: "after",
      isLive,
      meeting: ended.meeting ?? opened,
      lineCount: ended.meeting?.transcriptLineCount ?? null,
    };
  }
  // A meeting that says it is being recorded when nothing is (or before the
  // recording state has arrived) is treated as live: quiet is the safe side.
  return {
    phase: opened.status === "recording" ? "live" : "after",
    isLive,
    meeting: opened,
    lineCount: opened.transcriptLineCount,
  };
}
