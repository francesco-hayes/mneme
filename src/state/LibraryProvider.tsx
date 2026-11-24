import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/api";
import { useLoad } from "../lib/useLoad";
import { useWindowEvent } from "../lib/useWindowEvent";
import { ALL, type FolderView } from "../screens/home/folderView";
import { type Library, LibraryContext, type LibraryData } from "./library";
import { useRecording } from "./recording";

async function loadLibrary(): Promise<LibraryData> {
  const [meetings, collections] = await Promise.all([
    api.listMeetings(),
    api.listCollections(),
  ]);
  return { meetings, collections };
}

/**
 * Reads the meetings and folders for the sidebar and Home, and remembers
 * which folder Home shows. The list is read again when a recording starts or
 * stops and when the window gets focus back; screens call `data.reload()`
 * after their own changes.
 */
export function LibraryProvider({ children }: { children: ReactNode }) {
  const data = useLoad(loadLibrary);
  const reload = data.reload;
  const { state: recording, isActive } = useRecording();

  // A recording that starts adds a meeting and one that stops gives it a
  // duration, wherever it was started or stopped from.
  const recordingMeetingId = isActive ? recording.meetingId : null;
  const listedFor = useRef(recordingMeetingId);
  useEffect(() => {
    if (listedFor.current === recordingMeetingId) return;
    listedFor.current = recordingMeetingId;
    reload();
  }, [recordingMeetingId, reload]);

  useWindowEvent("focus", reload);

  const [chosen, setView] = useState<FolderView>(ALL);
  const collections = data.status === "ready" ? data.data.collections : null;
  const view: FolderView =
    chosen.kind === "collection" &&
    collections !== null &&
    !collections.includes(chosen.name)
      ? ALL
      : chosen;

  const library = useMemo<Library>(
    () => ({ data, view, setView }),
    [data, view],
  );
  return <LibraryContext value={library}>{children}</LibraryContext>;
}
