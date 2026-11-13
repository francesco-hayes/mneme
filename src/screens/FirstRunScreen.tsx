import { EmptyState, Screen } from "../components";

/**
 * Placeholder for issue 020. The app opens here while no model is installed
 * (see `startupRoute`). When its steps are done, First run leaves with
 * `navigation.reset({ name: "home" })`, so there is no way back into it.
 */
export function FirstRunScreen() {
  return (
    <Screen title="Welcome to Mneme" back={false} settings={false}>
      <EmptyState
        title="Mneme needs a transcription model"
        description="Placeholder. Issue 020 builds the model download and the permission steps."
      />
    </Screen>
  );
}
