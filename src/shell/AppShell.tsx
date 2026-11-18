import { FirstRunScreen } from "../screens/FirstRunScreen";
import { HomeScreen } from "../screens/HomeScreen";
import { MeetingScreen } from "../screens/MeetingScreen";
import { SettingsScreen } from "../screens/SettingsScreen";
import { type Route, useNavigation } from "../state/navigation";
import styles from "./AppShell.module.css";
import { RecordingBar } from "./RecordingBar";
import { RecordingErrorNotice } from "./RecordingErrorNotice";

function CurrentScreen({ route }: { route: Route }) {
  switch (route.name) {
    case "home":
      return <HomeScreen />;
    case "meeting":
      // Keyed so that going from one meeting to another starts from scratch.
      return (
        <MeetingScreen key={route.meetingId} meetingId={route.meetingId} />
      );
    case "settings":
      return <SettingsScreen />;
    case "first-run":
      return <FirstRunScreen />;
  }
}

/**
 * The window's layout: the current screen and, under it, the recording bar.
 * A start or stop that failed outside the window is said in one line between
 * the two, so it shows on every screen without covering any of it.
 */
export function AppShell() {
  const { route } = useNavigation();

  return (
    <div className={styles.app}>
      <div className={styles.screenArea}>
        <CurrentScreen route={route} />
      </div>
      <RecordingErrorNotice />
      <RecordingBar />
    </div>
  );
}
