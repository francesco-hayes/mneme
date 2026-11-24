import "@testing-library/jest-dom/vitest";
import "./jsdomGaps";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { mockApi } from "../lib/api/mock";
import { resetRememberedFolder } from "../screens/home/folderView";
import { resetNotesAutosave } from "../screens/meeting/notesAutosave";

afterEach(() => {
  // Unmount what the test rendered, drop notes that were still waiting to be
  // saved, then put the shared mock back to its default seed, so no state or
  // timer leaks into the next test.
  cleanup();
  resetNotesAutosave();
  resetRememberedFolder();
  mockApi.mock.reset();
});
