import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { mockApi } from "../lib/api/mock";

afterEach(() => {
  // Unmount what the test rendered, then put the shared mock back to its
  // default seed so no state or timer leaks into the next test.
  cleanup();
  mockApi.mock.reset();
});
