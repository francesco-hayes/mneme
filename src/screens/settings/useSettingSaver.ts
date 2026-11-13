import { useCallback, useRef, useState } from "react";
import type { Settings } from "../../lib/api";
import { errorMessage } from "../../lib/errors";
import { useModelLibrary } from "../../state/modelLibrary";

export interface SettingSaver<T> {
  /** What to show: the value being saved while a save runs, otherwise the saved one. */
  value: T;
  saving: boolean;
  /** Why the last save failed, or null. */
  error: string | null;
  /**
   * Saves a new value through `api.updateSettings`. The control shows it at
   * once; if the backend refuses, the reason is in `error` and `value` goes
   * back to what was saved before. Resolves to whether it was saved.
   */
  save: (value: T) => Promise<boolean>;
}

/**
 * One field of the settings, for a control that saves on change. Call it only
 * once the model library is ready (`settings` is then not null).
 */
export function useSettingSaver<K extends keyof Settings>(
  key: K,
  saved: Settings[K],
): SettingSaver<Settings[K]> {
  const { updateSettings } = useModelLibrary();
  const [pending, setPending] = useState<{ value: Settings[K] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Only the latest save may end the pending state or report a failure.
  const latest = useRef(0);

  const save = useCallback(
    async (value: Settings[K]) => {
      const mine = (latest.current += 1);
      setPending({ value });
      setError(null);
      try {
        await updateSettings({ [key]: value } as Partial<Settings>);
        return true;
      } catch (failure) {
        if (mine === latest.current) setError(errorMessage(failure));
        return false;
      } finally {
        if (mine === latest.current) setPending(null);
      }
    },
    [key, updateSettings],
  );

  return {
    value: pending !== null ? pending.value : saved,
    saving: pending !== null,
    error,
    save,
  };
}
