// The meetings and folders the sidebar and Home show (issue 036), read once
// for both so the folder counts and the list never disagree.
// `LibraryProvider` owns it; components read it with `useLibrary`.

import { createContext, useContext } from "react";
import type { MeetingSummary } from "../lib/api";
import type { LoadResult } from "../lib/useLoad";
import type { FolderView } from "../screens/home/folderView";

export interface LibraryData {
  meetings: MeetingSummary[];
  /** Every folder (collection), empty ones too, sorted by name. */
  collections: string[];
}

export interface Library {
  /** The last read of the meetings and folders. */
  data: LoadResult<LibraryData>;
  /**
   * The folder Home shows. Kept while the app runs, so opening a meeting and
   * coming back lands in the same folder. A folder that is gone (renamed or
   * removed in Finder) reads as All.
   */
  view: FolderView;
  setView: (view: FolderView) => void;
}

export const LibraryContext = createContext<Library | null>(null);

export function useLibrary(): Library {
  const library = useContext(LibraryContext);
  if (library === null) {
    throw new Error("useLibrary must be used inside a LibraryProvider");
  }
  return library;
}
