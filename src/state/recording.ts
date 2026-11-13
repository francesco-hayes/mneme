// App-wide recording state.
//
// `RecordingProvider` holds the app's one subscription to
// `api.getRecordingState` / `api.onRecordingState`. Anything that needs to know
// whether a meeting is being recorded reads it with `useRecording` instead of
// calling the API again.

import { createContext, useContext, useEffect, useRef } from "react";
import type { Meeting, RecordingState } from "../lib/api";

export type BeforeStopHandler = () => void | Promise<void>;

export interface Recording {
  /** The latest state from the backend. `idle` until the first one arrives. */
  state: RecordingState;
  /** True while recording or stopping: the recording bar is showing. */
  isActive: boolean;
  /** True from the moment Stop is pressed until the meeting is complete. */
  isStopping: boolean;
  /**
   * Stops the recording: waits for the before-stop handlers, then calls
   * `api.stopMeeting`. Resolves to the finished meeting, or to null when the
   * stop failed (the message is in `stopError`) or one was already under way.
   * It never rejects.
   */
  stop: () => Promise<Meeting | null>;
  /** Why the last Stop of the current recording failed, or null. */
  stopError: string | null;
  /**
   * Registers work to finish before a stop, and returns the function that
   * removes it. A handler that fails does not block the stop. Components
   * normally use `useBeforeStop` instead.
   */
  onBeforeStop: (handler: BeforeStopHandler) => () => void;
}

export const IDLE_RECORDING: RecordingState = {
  status: "idle",
  meetingId: null,
  elapsedSecs: 0,
};

export const RecordingContext = createContext<Recording | null>(null);

export function useRecording(): Recording {
  const recording = useContext(RecordingContext);
  if (recording === null) {
    throw new Error("useRecording needs a RecordingProvider above it.");
  }
  return recording;
}

/**
 * Runs `handler` before any stop while the calling component is mounted, and
 * waits for the promise it returns. The meeting view uses it to save notes
 * before `api.stopMeeting` is called. The latest `handler` is always the one
 * that runs, so it does not need `useCallback`.
 */
export function useBeforeStop(handler: BeforeStopHandler): void {
  const { onBeforeStop } = useRecording();
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => onBeforeStop(() => latest.current()), [onBeforeStop]);
}
