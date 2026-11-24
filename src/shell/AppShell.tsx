import { useEffect, useRef } from "react";
import { SettingsInSidebarContext } from "../components";
import { api } from "../lib/api";
import { FirstRunScreen } from "../screens/FirstRunScreen";
import { HomeScreen } from "../screens/HomeScreen";
import { MeetingScreen } from "../screens/MeetingScreen";
import { SettingsScreen } from "../screens/SettingsScreen";
import {
  type Navigation,
  type Route,
  useNavigation,
} from "../state/navigation";
import styles from "./AppShell.module.css";
import { RecordingBar } from "./RecordingBar";
import { RecordingErrorNotice } from "./RecordingErrorNotice";
import { Sidebar } from "./Sidebar";

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
 * Goes to the meeting the backend asks for (Open Mneme in the menu bar during
 * a recording), unless it is already on screen.
 */
function useOpenMeetingRequests(navigation: Navigation) {
  const latest = useRef(navigation);
  useEffect(() => {
    latest.current = navigation;
  });
  useEffect(
    () =>
      api.onOpenMeeting((meetingId) => {
        const { route, navigate } = latest.current;
        if (route.name === "meeting" && route.meetingId === meetingId) return;
        navigate({ name: "meeting", meetingId });
      }),
    [],
  );
}

/**
 * The window's layout: the sidebar (folders and Settings) on the left, and
 * beside it the current screen with the recording bar under it. A start or
 * stop that failed outside the window is said in one line between the two,
 * so it shows on every screen without covering any of it. First run has no
 * sidebar.
 */
export function AppShell() {
  const navigation = useNavigation();
  const { route } = navigation;
  useOpenMeetingRequests(navigation);

  const withSidebar = route.name !== "first-run";
  const main = (
    <div className={styles.app}>
      <div className={styles.screenArea}>
        <CurrentScreen route={route} />
      </div>
      <RecordingErrorNotice />
      <RecordingBar />
    </div>
  );
  if (!withSidebar) return main;
  return (
    <SettingsInSidebarContext value={true}>
      <div className={styles.window}>
        <Sidebar />
        {main}
      </div>
    </SettingsInSidebarContext>
  );
}
