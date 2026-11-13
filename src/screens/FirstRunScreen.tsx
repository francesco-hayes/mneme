import { useEffect } from "react";
import { InlineError, Screen } from "../components";
import { cx } from "../lib/cx";
import { useModelLibrary } from "../state/modelLibrary";
import { useNavigation } from "../state/navigation";
import styles from "./firstRun/FirstRun.module.css";
import { ModelStep } from "./firstRun/ModelStep";
import { PermissionsStep } from "./firstRun/PermissionsStep";

/**
 * First run: the app opens here while no model is installed (see
 * `startupRoute`). Two steps: download a model, then allow the microphone and
 * system audio. Finish leaves with `navigation.reset`, so there is no way back
 * into it.
 *
 * The step is not kept anywhere: it is the permissions step as soon as the
 * model library has a default model, which the library sets by itself when
 * the first download finishes. So quitting during a download and opening the
 * app again starts at the model step, with nothing left over.
 */
export function FirstRunScreen() {
  const library = useModelLibrary();
  const navigation = useNavigation();
  const { load } = library;
  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Screen title="Welcome to Mneme" back={false} settings={false}>
      {library.status === "error" ? (
        <InlineError
          className={styles.message}
          message={`Could not load the models: ${library.loadError ?? ""}`}
          onRetry={() => void load()}
        />
      ) : library.settings === null ? (
        <p className={cx(styles.loading, "appears-late")}>Loading…</p>
      ) : (
        <>
          {library.defaultModel === undefined ? (
            <ModelStep />
          ) : (
            <PermissionsStep
              model={library.defaultModel}
              onFinish={() => navigation.reset({ name: "home" })}
            />
          )}
          <p className={styles.privacy}>
            Nothing you record leaves this Mac. Downloading the model is the
            only time Mneme uses the network.
          </p>
        </>
      )}
    </Screen>
  );
}
