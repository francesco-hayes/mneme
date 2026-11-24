import {
  memo,
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Button,
  EmptyState,
  focusListRow,
  InlineError,
  List,
  ListRow,
  Screen,
} from "../components";
import { api, type MeetingSummary } from "../lib/api";
import { errorMessage } from "../lib/errors";
import {
  type DateContext,
  formatDuration,
  formatFullDate,
  formatMeetingDate,
} from "../lib/format";
import { newestFirst } from "../lib/meetings";
import { useLoad } from "../lib/useLoad";
import { useWindowEvent } from "../lib/useWindowEvent";
import { useNavigation } from "../state/navigation";
import { useRecording } from "../state/recording";
import styles from "./HomeScreen.module.css";

export interface HomeScreenProps {
  /**
   * The clock, locale and time zone that dates are shown in. The app leaves
   * all three out and gets the system's; tests pass them so that "Today" does
   * not depend on when and where they run.
   */
  now?: () => Date;
  locale?: string;
  timeZone?: string;
}

const systemNow = () => new Date();
const loadModels = () => api.listModels();

const isTextEntry = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.isContentEditable || target.matches("input, textarea, select"));

/**
 * Home: every meeting, newest first, and the button that starts a new one.
 * The list is read again each time Home is shown, when a recording starts or
 * stops, and when the window gets focus back.
 */
export function HomeScreen({
  now = systemNow,
  locale,
  timeZone,
}: HomeScreenProps) {
  const { navigate } = useNavigation();
  const { state: recording, isActive } = useRecording();

  // "Today" is judged against the moment the list was read.
  const loadMeetings = useCallback(async () => {
    const list = await api.listMeetings();
    return { list, loadedAt: now() };
  }, [now]);
  const meetings = useLoad(loadMeetings);
  const models = useLoad(loadModels);
  const reloadMeetings = meetings.reload;

  // A recording that starts adds a meeting and one that stops gives it a
  // duration, wherever it was started or stopped from.
  const recordingMeetingId = isActive ? recording.meetingId : null;
  const listedFor = useRef(recordingMeetingId);
  useEffect(() => {
    if (listedFor.current === recordingMeetingId) return;
    listedFor.current = recordingMeetingId;
    reloadMeetings();
  }, [recordingMeetingId, reloadMeetings]);

  useWindowEvent("focus", reloadMeetings);

  // Until the model list has loaded (or if it cannot be), the button stays
  // usable: a start that cannot work reports why.
  const noModel =
    models.status === "ready" && !models.data.some((model) => model.installed);
  const [starting, setStarting] = useState(false);
  const startInFlight = useRef(false);
  const [startError, setStartError] = useState<string | null>(null);
  // A start from here, or from the menu bar or the shortcut, is loading its
  // model: a second one would be refused.
  const loading = starting || recording.starting;
  const canStart = !noModel && !isActive && !loading;

  async function startMeeting() {
    if (!canStart || startInFlight.current) return;
    startInFlight.current = true;
    setStarting(true);
    setStartError(null);
    try {
      const meeting = await api.startMeeting();
      navigate({ name: "meeting", meetingId: meeting.id });
    } catch (error) {
      setStartError(errorMessage(error));
    } finally {
      startInFlight.current = false;
      setStarting(false);
    }
  }

  const openMeeting = useCallback(
    (meetingId: string) => navigate({ name: "meeting", meetingId }),
    [navigate],
  );

  const listRef = useRef<HTMLUListElement>(null);
  useWindowEvent("keydown", (event) => {
    if (event.defaultPrevented) return;
    if (event.altKey || event.ctrlKey || event.shiftKey) return;

    if (event.metaKey) {
      if (event.key.toLowerCase() === "n") {
        event.preventDefault();
        void startMeeting();
      }
      return;
    }

    // With focus anywhere but in the list or a text field, the arrow keys
    // step into the list. From there `List` moves between rows.
    const list = listRef.current;
    const into =
      event.key === "ArrowDown"
        ? "first"
        : event.key === "ArrowUp"
          ? "last"
          : null;
    if (into === null || list === null || isTextEntry(event.target)) return;
    if (event.target instanceof Node && list.contains(event.target)) return;
    if (focusListRow(list, into)) event.preventDefault();
  });

  // `loadedAt` is a new date for every read of the list, and only then.
  const loadedAt = meetings.status === "ready" ? meetings.data.loadedAt : null;
  const dates = useMemo(
    () => (loadedAt === null ? null : { now: loadedAt, locale, timeZone }),
    [loadedAt, locale, timeZone],
  );

  const disabledReason = noModel
    ? "Download a transcription model in Settings first."
    : isActive
      ? "A meeting is being recorded. Stop it to start another."
      : loading
        ? "A meeting is starting."
        : undefined;
  const newMeetingButton = (variant: "primary" | "default") => (
    <Button
      variant={variant}
      disabled={!canStart}
      title={disabledReason}
      onClick={() => void startMeeting()}
    >
      {loading ? "Starting…" : "New meeting"}
    </Button>
  );
  const modelNeeded = (
    <>
      To record a meeting, Mneme needs a transcription model.{" "}
      <Button variant="link" onClick={() => navigate({ name: "settings" })}>
        Download one in Settings
      </Button>
      .
    </>
  );

  return (
    <Screen title="Meetings" actions={newMeetingButton("primary")}>
      {startError !== null && (
        <InlineError
          className={styles.error}
          message={`Could not start the meeting: ${startError}`}
        />
      )}
      {meetings.status === "loading" && (
        <p className={`${styles.loading} appears-late`} role="status">
          Loading meetings…
        </p>
      )}
      {meetings.status === "error" && (
        <InlineError
          className={styles.error}
          message={`Could not load meetings: ${meetings.message}`}
          onRetry={meetings.reload}
        />
      )}
      {meetings.status === "ready" && meetings.data.list.length === 0 && (
        <EmptyState
          title="No meetings yet"
          description={
            noModel
              ? modelNeeded
              : "Start one, and its notes and transcript are kept here."
          }
          action={newMeetingButton("default")}
        />
      )}
      {meetings.status === "ready" &&
        meetings.data.list.length > 0 &&
        dates !== null && (
          <>
            {noModel && <p className={styles.notice}>{modelNeeded}</p>}
            <MeetingList
              meetings={meetings.data.list}
              dates={dates}
              onOpen={openMeeting}
              listRef={listRef}
            />
          </>
        )}
    </Screen>
  );
}

interface MeetingListProps {
  meetings: readonly MeetingSummary[];
  dates: DateContext;
  onOpen: (meetingId: string) => void;
  listRef: RefObject<HTMLUListElement | null>;
}

/**
 * The rows. Memoized because Home renders again on every second of a
 * recording, and nothing in the list changes with the timer.
 */
const MeetingList = memo(function MeetingList({
  meetings,
  dates,
  onOpen,
  listRef,
}: MeetingListProps) {
  return (
    <List aria-label="Meetings" ref={listRef}>
      {newestFirst(meetings).map((meeting) => (
        <ListRow
          key={meeting.id}
          title={meeting.title}
          detail={<MeetingDetail meeting={meeting} dates={dates} />}
          meta={
            meeting.status === "recording" ? (
              <span className={styles.live}>
                <span className={styles.liveDot} aria-hidden="true" />
                Recording
              </span>
            ) : (
              (formatDuration(meeting.durationSecs, dates.locale) ?? undefined)
            )
          }
          onActivate={() => onOpen(meeting.id)}
        />
      ))}
    </List>
  );
});

/** The second line of a row: when it started, and whether it was cut short. */
function MeetingDetail({
  meeting,
  dates,
}: {
  meeting: MeetingSummary;
  dates: DateContext;
}) {
  const date = formatMeetingDate(meeting.startedAt, dates);
  return (
    <>
      {date !== "" && (
        <time
          dateTime={meeting.startedAt}
          title={formatFullDate(meeting.startedAt, dates)}
        >
          {date}
        </time>
      )}
      {meeting.status === "interrupted" && (
        <>
          {date !== "" && (
            <>
              {" "}
              <span aria-hidden="true">·</span>{" "}
            </>
          )}
          <span
            className={styles.interrupted}
            title="The recording ended without being stopped. The notes and the transcript up to that point were kept."
          >
            Interrupted
          </span>
        </>
      )}
    </>
  );
}
