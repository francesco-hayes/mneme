import { useEffect, useRef } from "react";

/**
 * Listens for `type` on the window while the calling component is mounted.
 * The latest `handler` is always the one that runs, so it does not need
 * `useCallback`. A screen uses it for its keyboard shortcuts:
 *
 *     useWindowEvent("keydown", (event) => {
 *       if (event.metaKey && event.key === "n") startMeeting();
 *     });
 */
export function useWindowEvent<K extends keyof WindowEventMap>(
  type: K,
  handler: (event: WindowEventMap[K]) => void,
): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    const listener = (event: WindowEventMap[K]) => latest.current(event);
    window.addEventListener(type, listener);
    return () => window.removeEventListener(type, listener);
  }, [type]);
}
