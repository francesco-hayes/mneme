import { EmptyState, Screen } from "../components";

/** Placeholder for issue 019. */
export function SettingsScreen() {
  return (
    <Screen title="Settings" settings={false}>
      <EmptyState
        title="Nothing to set yet"
        description="Placeholder. Issue 019 builds models, language and the global shortcut."
      />
    </Screen>
  );
}
