import { expect, test } from "vitest";
import { api, chooseApi } from "./index";
import { mockApi } from "./mock";
import { tauriApi } from "./tauri";
import type { Api } from "./types";

test("uses the mock outside Tauri and the Tauri implementation inside it", () => {
  expect(chooseApi(undefined, false)).toBe(mockApi);
  expect(chooseApi(undefined, true)).toBe(tauriApi);
});

test("VITE_MNEME_API overrides the choice", () => {
  expect(chooseApi("mock", true)).toBe(mockApi);
  expect(chooseApi("tauri", false)).toBe(tauriApi);
  expect(chooseApi("something else", false)).toBe(mockApi);
});

test("tests run against the mock", () => {
  expect(api).toBe(mockApi);
});

test("every method of the Tauri stub reports that it is not implemented", async () => {
  const methods = Object.keys(tauriApi) as (keyof Api)[];
  expect(methods).toHaveLength(19);

  for (const method of methods) {
    const invoke = tauriApi[method] as (...args: unknown[]) => unknown;
    // Subscriptions throw; everything else returns a rejected promise.
    const attempt = async () => invoke("id", "value");
    await expect(attempt(), method).rejects.toThrow(
      `api.${method} is not implemented`,
    );
  }
});
