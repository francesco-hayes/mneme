// Render helpers for component tests. Everything goes through the shared mock
// API, which `setup.ts` resets after every test.

import { act, render, type RenderResult } from "@testing-library/react";
import type { ReactElement } from "react";
import { vi } from "vitest";
import { AppShell } from "../shell/AppShell";
import { ModelLibraryProvider } from "../state/ModelLibraryProvider";
import { NavigationProvider } from "../state/NavigationProvider";
import type { Route } from "../state/navigation";
import { RecordingProvider } from "../state/RecordingProvider";

interface ShellOptions {
  /** The screen to start on. Default: Home. */
  route?: Route;
}

/**
 * Renders `ui` inside the recording, model library and navigation providers,
 * for a component that calls `useRecording`, `useModelLibrary` or
 * `useNavigation` (anything containing a `Screen` does).
 */
export function renderWithProviders(
  ui: ReactElement,
  { route = { name: "home" } }: ShellOptions = {},
): RenderResult {
  return render(
    <RecordingProvider>
      <ModelLibraryProvider>
        <NavigationProvider initialRoute={route}>{ui}</NavigationProvider>
      </ModelLibraryProvider>
    </RecordingProvider>,
  );
}

/**
 * Renders the whole shell (current screen plus recording bar) starting on
 * `route`, skipping the first-run gate. Use it to test a screen together with
 * navigation and the recording bar; render `<App />` to include the gate.
 */
export function renderShell(options: ShellOptions = {}): RenderResult {
  return renderWithProviders(<AppShell />, options);
}

/**
 * For tests on Vitest fake timers, where `findBy…` and `waitFor` cannot be
 * used: moves the clock forward by `ms` and lets React apply what happened.
 * With no argument it only settles pending API calls.
 */
export async function advance(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}
