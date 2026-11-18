import { Button, InlineError } from "../components";
import { useRecording } from "../state/recording";
import styles from "./RecordingErrorNotice.module.css";

/**
 * Why a start or stop asked for from the menu bar or the global shortcut
 * failed: one line at the bottom of every screen, above the recording bar,
 * until it is dismissed or a recording starts. A start or stop pressed inside
 * the window reports its failure where it was pressed, not here.
 */
export function RecordingErrorNotice() {
  const { outsideError, dismissOutsideError } = useRecording();
  if (outsideError === null) return null;

  const what =
    outsideError.action === "start"
      ? "Could not start recording"
      : "Could not stop recording";

  return (
    <div className={styles.notice}>
      <InlineError
        className={styles.message}
        message={`${what}: ${outsideError.message}`}
      />
      <Button variant="plain" size="small" onClick={dismissOutsideError}>
        Dismiss
      </Button>
    </div>
  );
}
