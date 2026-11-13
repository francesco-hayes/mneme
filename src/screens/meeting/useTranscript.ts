import { useCallback, useEffect, useRef, useState } from "react";
import { api, type TranscriptLine } from "../../lib/api";
import { errorMessage } from "../../lib/errors";

export type TranscriptLoad =
  /** Not asked for yet: the pane has never been opened. */
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; lines: TranscriptLine[] }
  | { status: "error"; message: string };

export interface Transcript {
  load: TranscriptLoad;
  /**
   * Reads the transcript unless it is already here or on its way. Called each
   * time the pane opens, so the first opening loads it, a later one shows the
   * same lines again, and one after a failure tries again.
   */
  ensureLoaded: () => void;
  /** Reads the transcript again after a failure. */
  retry: () => void;
}

/**
 * A meeting's transcript, read from `api.getTranscript` the first time it is
 * wanted and kept for as long as the meeting view is open. Nothing is read
 * until `ensureLoaded` is called.
 */
export function useTranscript(meetingId: string): Transcript {
  const [load, setLoad] = useState<TranscriptLoad>({ status: "idle" });
  const status = useRef<TranscriptLoad["status"]>("idle");
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const read = useCallback(() => {
    const settle = (next: TranscriptLoad) => {
      if (!mounted.current) return;
      status.current = next.status;
      setLoad(next);
    };
    settle({ status: "loading" });
    api.getTranscript(meetingId).then(
      (lines) => settle({ status: "ready", lines }),
      (error: unknown) =>
        settle({ status: "error", message: errorMessage(error) }),
    );
  }, [meetingId]);

  const ensureLoaded = useCallback(() => {
    if (status.current === "idle" || status.current === "error") read();
  }, [read]);

  return { load, ensureLoaded, retry: read };
}
