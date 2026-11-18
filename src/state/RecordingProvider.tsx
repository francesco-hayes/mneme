import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { api, type RecordingError, type RecordingState } from "../lib/api";
import { errorMessage } from "../lib/errors";
import {
  type BeforeStopHandler,
  IDLE_RECORDING,
  type Recording,
  RecordingContext,
} from "./recording";

export function RecordingProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<RecordingState>(IDLE_RECORDING);
  const [stopRequested, setStopRequested] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const [outsideError, setOutsideError] = useState<RecordingError | null>(null);
  const stopInFlight = useRef(false);
  const beforeStop = useRef(new Set<BeforeStopHandler>());

  useEffect(() => {
    let mounted = true;
    let sawEvent = false;
    // The meeting of the last state seen, to tell a recording that starts
    // from one that goes on (a state arrives every second).
    let meetingId: string | null = null;
    const unsubscribe = api.onRecordingState((next) => {
      sawEvent = true;
      setState(next);
      // A start or stop that failed outside the window is old news once a
      // recording starts.
      if (next.meetingId !== null && next.meetingId !== meetingId) {
        setOutsideError(null);
      }
      meetingId = next.meetingId;
      // A failed stop belongs to one recording; it is not shown on the next.
      if (next.status === "idle") setStopError(null);
    });
    api.getRecordingState().then(
      (first) => {
        // An event that arrived while this call was in flight is newer.
        if (mounted && !sawEvent) {
          setState(first);
          meetingId = first.meetingId;
        }
      },
      () => {
        // Stay idle; the next event from the subscription sets the state.
      },
    );
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, []);

  // A second error replaces the first.
  useEffect(() => api.onRecordingError(setOutsideError), []);
  const dismissOutsideError = useCallback(() => setOutsideError(null), []);

  const onBeforeStop = useCallback((handler: BeforeStopHandler) => {
    const handlers = beforeStop.current;
    handlers.add(handler);
    return () => {
      handlers.delete(handler);
    };
  }, []);

  const stop = useCallback(async () => {
    if (stopInFlight.current) return null;
    stopInFlight.current = true;
    setStopRequested(true);
    setStopError(null);
    try {
      // A handler that fails (a notes save, say) must not keep the microphone
      // on, so the stop goes ahead whatever the handlers do.
      await Promise.allSettled(
        [...beforeStop.current].map(async (handler) => handler()),
      );
      return await api.stopMeeting();
    } catch (error) {
      setStopError(errorMessage(error));
      return null;
    } finally {
      stopInFlight.current = false;
      setStopRequested(false);
    }
  }, []);

  const recording = useMemo<Recording>(
    () => ({
      state,
      isActive: state.status !== "idle",
      isStopping:
        state.status === "stopping" ||
        (stopRequested && state.status === "recording"),
      stop,
      stopError: state.status === "idle" ? null : stopError,
      outsideError,
      dismissOutsideError,
      onBeforeStop,
    }),
    [
      state,
      stopRequested,
      stopError,
      outsideError,
      dismissOutsideError,
      stop,
      onBeforeStop,
    ],
  );

  return <RecordingContext value={recording}>{children}</RecordingContext>;
}
