import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "./errors";

export type Load<T> =
  | { status: "loading" }
  | { status: "ready"; data: T }
  | { status: "error"; message: string };

export type LoadResult<T> = Load<T> & {
  /** Runs the call again. Data already shown stays up until the new result arrives. */
  reload: () => void;
};

interface Settled<T> {
  load: () => Promise<T>;
  result: Exclude<Load<T>, { status: "loading" }>;
}

/**
 * Runs an API read when the component mounts and whenever `load` changes, and
 * reports it as loading, ready or error. Pass a stable function: one defined
 * outside the component, or wrapped in `useCallback` with the ids it reads.
 *
 *     const load = useCallback(() => api.getMeeting(id), [id]);
 *     const meeting = useLoad(load);
 */
export function useLoad<T>(load: () => Promise<T>): LoadResult<T> {
  const [attempt, setAttempt] = useState(0);
  const [settled, setSettled] = useState<Settled<T> | null>(null);

  useEffect(() => {
    let current = true;
    load().then(
      (data) => {
        if (current) setSettled({ load, result: { status: "ready", data } });
      },
      (error: unknown) => {
        if (current) {
          setSettled({
            load,
            result: { status: "error", message: errorMessage(error) },
          });
        }
      },
    );
    return () => {
      current = false;
    };
  }, [load, attempt]);

  const reload = useCallback(() => {
    // Retrying after an error shows loading again; refreshing good data does not.
    setSettled((previous) =>
      previous?.result.status === "error" ? null : previous,
    );
    setAttempt((n) => n + 1);
  }, []);

  // A result for a different `load` (another meeting id, say) is never shown.
  const result: Load<T> =
    settled !== null && settled.load === load
      ? settled.result
      : { status: "loading" };
  return { ...result, reload };
}
