import { type ReactNode, useEffect, useState } from "react";
import { api } from "../lib/api";
import { ModelLibraryContext, ModelLibraryStore } from "./modelLibrary";

/**
 * Creates the app's one model library and keeps it listening for download
 * progress for as long as the app is open. It sits above the screens (see
 * `App`), so leaving Settings does not lose a download's progress.
 */
export function ModelLibraryProvider({ children }: { children: ReactNode }) {
  const [store] = useState(() => new ModelLibraryStore(api));
  useEffect(() => store.connect(), [store]);

  return <ModelLibraryContext value={store}>{children}</ModelLibraryContext>;
}
