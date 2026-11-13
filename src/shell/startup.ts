import type { ModelInfo } from "../lib/api";
import type { Route } from "../state/navigation";

/**
 * The first-run gate: with no model installed there is nothing to transcribe
 * with, so the app opens on First run instead of Home.
 *
 * It decides the screen the app opens on, and only that. First run leaves with
 * `navigation.reset({ name: "home" })` when its steps are done, and deleting
 * the last model in Settings later does not send the user back to First run:
 * Home explains the disabled New meeting button instead.
 */
export function startupRoute(models: readonly ModelInfo[]): Route {
  return models.some((model) => model.installed)
    ? { name: "home" }
    : { name: "first-run" };
}
