// The one module the UI imports to talk to the backend.

import { isTauri } from "@tauri-apps/api/core";
import { mockApi } from "./mock";
import { tauriApi } from "./tauri";
import type { Api } from "./types";

export type * from "./types";

/**
 * Tauri inside the app, the mock everywhere else (browser dev server, tests).
 * `VITE_MNEME_API=mock` or `=tauri` overrides the choice at build time;
 * `pnpm tauri dev` sets it to `mock` until issue 021 implements the backend.
 */
export function chooseApi(
  override: string | undefined,
  insideTauri: boolean,
): Api {
  if (override === "mock") return mockApi;
  if (override === "tauri") return tauriApi;
  return insideTauri ? tauriApi : mockApi;
}

export const api: Api = chooseApi(import.meta.env.VITE_MNEME_API, isTauri());

if (import.meta.env.DEV && api === mockApi) {
  // Lets you drive the mock from the browser console, for example
  // `__mnemeMock.setInstalledModels([])` or `__mnemeMock.failNextDownload()`.
  Object.assign(globalThis, { __mnemeMock: mockApi.mock });
}
