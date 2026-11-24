import { Button, InlineError, StopIcon, WarningIcon } from "../components";
import { formatElapsed } from "../lib/format";
import { useNavigation } from "../state/navigation";
import { useRecording } from "../state/recording";
import styles from "./RecordingBar.module.css";

/**
 * The one piece of recording UI, at the bottom of every screen while a meeting
 * is being recorded or is stopping, and absent when idle. It shows that the
 * app is listening, for how long, whether a channel is missing, a way to the
 * live meeting, and Stop.
 */
export function RecordingBar() {
  const { state, isStopping, stop, stopError } = useRecording();
  const { route, navigate } = useNavigation();

  const meetingId = state.meetingId;
  if (state.status === "idle" || meetingId === null) return null;

  const onLiveMeeting =
    route.name === "meeting" && route.meetingId === meetingId;

  return (
    <section
      className={styles.bar}
      aria-label="Recording"
      data-stopping={isStopping || undefined}
    >
      <span className={styles.dot} aria-hidden="true" />
      <span className={styles.label} role="status">
        {isStopping ? "Stopping…" : "Recording"}
      </span>
      <span className={styles.timer} role="timer" aria-label="Recording time">
        {formatElapsed(state.elapsedSecs)}
      </span>
      {/* The live meeting shows the whole sentence above the notes. */}
      {state.warning !== null && !onLiveMeeting && (
        <span className={styles.warning} title={state.warning}>
          <WarningIcon />
          <span aria-hidden="true">{shortWarning(state.warning)}</span>
          <span className="visually-hidden">{state.warning}</span>
        </span>
      )}
      {stopError !== null && (
        <InlineError
          className={styles.error}
          message={`Could not stop: ${stopError}`}
        />
      )}
      <span className={styles.spacer} />
      {!onLiveMeeting && (
        <Button
          variant="plain"
          size="small"
          onClick={() => navigate({ name: "meeting", meetingId })}
        >
          Open meeting
        </Button>
      )}
      <Button size="small" disabled={isStopping} onClick={() => void stop()}>
        <span className={styles.stopIcon}>
          <StopIcon />
        </span>
        Stop
      </Button>
    </section>
  );
}

/**
 * A few words for the bar. The backend's sentences name the channel first
 * ("System audio is not being recorded…", "The microphone stopped…").
 */
function shortWarning(warning: string): string {
  return warning.startsWith("System audio")
    ? "Mic only"
    : "Not all audio recorded";
}
