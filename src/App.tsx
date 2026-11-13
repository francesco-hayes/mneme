import { InlineError } from "./components";
import { api } from "./lib/api";
import { useLoad } from "./lib/useLoad";
import { AppShell } from "./shell/AppShell";
import styles from "./shell/AppShell.module.css";
import { startupRoute } from "./shell/startup";
import { NavigationProvider } from "./state/NavigationProvider";
import { RecordingProvider } from "./state/RecordingProvider";

const loadModels = () => api.listModels();

/** Finds out which screen to open on (the first-run gate), then shows the shell. */
function Startup() {
  const models = useLoad(loadModels);

  if (models.status === "loading") {
    return <div className={styles.startup} aria-busy="true" />;
  }
  if (models.status === "error") {
    return (
      <div className={styles.startup}>
        <InlineError
          message={`Mneme could not start: ${models.message}`}
          onRetry={models.reload}
        />
      </div>
    );
  }
  return (
    <NavigationProvider initialRoute={startupRoute(models.data)}>
      <AppShell />
    </NavigationProvider>
  );
}

export default function App() {
  return (
    <RecordingProvider>
      <Startup />
    </RecordingProvider>
  );
}
