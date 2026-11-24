import {
  Fragment,
  memo,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Button,
  ConfirmDialog,
  EmptyState,
  focusListRow,
  InlineError,
  List,
  ListRow,
  MenuButton,
  MoreIcon,
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
import {
  DeleteFolderDialog,
  FolderBar,
  FolderNameDialog,
  MoveMeetingDialog,
} from "./home/folders";
import {
  ALL,
  collectionOf,
  type FolderView,
  inView,
  rememberedFolder,
  rememberFolder,
} from "./home/folderView";

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
 * Home: the meetings, newest first, and the button that starts a new one.
 * The folder bar narrows the list to one folder, and New meeting starts the
 * meeting in that folder. The list is read again each time Home is shown,
 * when a recording starts or stops, and when the window gets focus back.
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
    const [list, collections] = await Promise.all([
      api.listMeetings(),
      api.listCollections(),
    ]);
    return { list, collections, loadedAt: now() };
  }, [now]);
  const meetings = useLoad(loadMeetings);
  const models = useLoad(loadModels);
  const reloadMeetings = meetings.reload;

  // The folder being shown. One that is gone (renamed or removed in Finder)
  // falls back to All.
  const [chosenView, setChosenView] = useState<FolderView>(rememberedFolder);
  const collections =
    meetings.status === "ready" ? meetings.data.collections : null;
  const view: FolderView =
    chosenView.kind === "collection" &&
    collections !== null &&
    !collections.includes(chosenView.name)
      ? ALL
      : chosenView;
  const chooseView = useCallback((next: FolderView) => {
    rememberFolder(next);
    setChosenView(next);
  }, []);
  const [folderDialog, setFolderDialog] = useState<
    "new" | "rename" | "delete" | null
  >(null);

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
      const meeting = await api.startMeeting(collectionOf(view));
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
  const shown =
    meetings.status === "ready"
      ? meetings.data.list.filter((meeting) => inView(view, meeting.collection))
      : [];
  const folderName = collectionOf(view);
  const folderMenu = folderName !== null && (
    <MenuButton
      label={`Options for the folder ${folderName}`}
      icon={<MoreIcon />}
      items={[
        { label: "Rename…", onSelect: () => setFolderDialog("rename") },
        {
          label: "Delete…",
          danger: true,
          onSelect: () => setFolderDialog("delete"),
        },
      ]}
    />
  );
  const title =
    view.kind === "collection"
      ? view.name
      : view.kind === "unfiled"
        ? "Unfiled"
        : "Meetings";

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
    <Screen
      title={title}
      actions={
        <>
          {folderMenu}
          {newMeetingButton("primary")}
        </>
      }
    >
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
      {meetings.status === "ready" && (
        <FolderBar
          collections={meetings.data.collections}
          view={view}
          onSelect={chooseView}
          onNewFolder={() => setFolderDialog("new")}
        />
      )}
      {meetings.status === "ready" &&
        view.kind !== "all" &&
        shown.length === 0 && (
          <EmptyState
            title={
              folderName !== null
                ? `No meetings in ${folderName}`
                : "No unfiled meetings"
            }
            description={
              folderName !== null
                ? "Start one here, or move meetings in with Move to… in their ⋯ menu."
                : "Every meeting is in a folder."
            }
            action={
              folderName !== null ? newMeetingButton("default") : undefined
            }
          />
        )}
      {meetings.status === "ready" &&
        view.kind === "all" &&
        meetings.data.list.length === 0 && (
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
      {meetings.status === "ready" && shown.length > 0 && dates !== null && (
        <>
          {noModel && <p className={styles.notice}>{modelNeeded}</p>}
          <MeetingList
            meetings={shown}
            collections={meetings.data.collections}
            showCollection={view.kind === "all"}
            dates={dates}
            onOpen={openMeeting}
            onChanged={reloadMeetings}
            listRef={listRef}
          />
        </>
      )}
      {folderDialog === "new" && (
        <FolderNameDialog
          save={(name) => api.createCollection(name)}
          onDone={(name) => {
            setFolderDialog(null);
            chooseView({ kind: "collection", name });
            reloadMeetings();
          }}
          onCancel={() => setFolderDialog(null)}
        />
      )}
      {folderDialog === "rename" && folderName !== null && (
        <FolderNameDialog
          current={folderName}
          save={(name) => api.renameCollection(folderName, name)}
          onDone={(name) => {
            setFolderDialog(null);
            chooseView({ kind: "collection", name });
            reloadMeetings();
          }}
          onCancel={() => setFolderDialog(null)}
        />
      )}
      {folderDialog === "delete" && folderName !== null && (
        <DeleteFolderDialog
          name={folderName}
          meetingCount={shown.length}
          remove={() => api.deleteCollection(folderName)}
          onDone={() => {
            setFolderDialog(null);
            chooseView(ALL);
            reloadMeetings();
          }}
          onCancel={() => setFolderDialog(null)}
        />
      )}
    </Screen>
  );
}

interface MeetingListProps {
  meetings: readonly MeetingSummary[];
  /** Every folder, for Move to…. */
  collections: readonly string[];
  /** Whether each row names its folder: in All, not inside a folder. */
  showCollection: boolean;
  dates: DateContext;
  onOpen: (meetingId: string) => void;
  /** A meeting was moved to the Trash or to another folder: read the list again. */
  onChanged: () => void;
  listRef: RefObject<HTMLUListElement | null>;
}

/**
 * The rows. Memoized because Home renders again on every second of a
 * recording, and nothing in the list changes with the timer.
 */
const MeetingList = memo(function MeetingList({
  meetings,
  collections,
  showCollection,
  dates,
  onOpen,
  onChanged,
  listRef,
}: MeetingListProps) {
  return (
    <List aria-label="Meetings" ref={listRef}>
      {newestFirst(meetings).map((meeting) => (
        <MeetingRow
          key={meeting.id}
          meeting={meeting}
          collections={collections}
          showCollection={showCollection}
          dates={dates}
          onOpen={onOpen}
          onChanged={onChanged}
        />
      ))}
    </List>
  );
});

interface MeetingRowProps {
  meeting: MeetingSummary;
  collections: readonly string[];
  showCollection: boolean;
  dates: DateContext;
  onOpen: (meetingId: string) => void;
  onChanged: () => void;
}

/**
 * One meeting: it opens on a click, and its ⋯ menu has View, Move to… and
 * Delete. Move to… picks a folder in a dialog. Delete asks in a dialog
 * first, then moves the meeting to the Trash.
 */
function MeetingRow({
  meeting,
  collections,
  showCollection,
  dates,
  onOpen,
  onChanged,
}: MeetingRowProps) {
  const [moving, setMoving] = useState(false);
  const [asking, setAsking] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const recording = meeting.status === "recording";

  function closeQuestion() {
    setAsking(false);
    setError(null);
  }

  async function confirmDelete() {
    setDeleting(true);
    setError(null);
    try {
      await api.deleteMeeting(meeting.id);
      setAsking(false);
      onChanged();
    } catch (failure) {
      setError(`Could not delete the meeting: ${errorMessage(failure)}`);
    } finally {
      setDeleting(false);
    }
  }

  return (
    <>
      <ListRow
        title={meeting.title}
        detail={
          <MeetingDetail
            meeting={meeting}
            dates={dates}
            showCollection={showCollection}
          />
        }
        meta={
          recording ? (
            <span className={styles.live}>
              <span className={styles.liveDot} aria-hidden="true" />
              Recording
            </span>
          ) : (
            (formatDuration(meeting.durationSecs, dates.locale) ?? undefined)
          )
        }
        onActivate={() => onOpen(meeting.id)}
        trailing={
          <MenuButton
            label={`Options for ${meeting.title}`}
            icon={<MoreIcon />}
            items={[
              { label: "View", onSelect: () => onOpen(meeting.id) },
              {
                label: "Move to…",
                disabled: recording,
                title: recording
                  ? "Stop the recording before moving this meeting."
                  : undefined,
                onSelect: () => setMoving(true),
              },
              {
                label: "Delete…",
                danger: true,
                disabled: recording,
                title: recording
                  ? "Stop the recording before deleting this meeting."
                  : undefined,
                onSelect: () => setAsking(true),
              },
            ]}
          />
        }
      />
      {moving && (
        <MoveMeetingDialog
          title={meeting.title}
          collection={meeting.collection}
          collections={collections}
          move={async (collection) => {
            await api.moveMeeting(meeting.id, collection);
          }}
          onDone={() => {
            setMoving(false);
            onChanged();
          }}
          onCancel={() => setMoving(false)}
        />
      )}
      {asking && (
        <ConfirmDialog
          title={`Move “${meeting.title}” to the Trash?`}
          message="Its notes and transcript go with it. Put Back in Finder restores it."
          confirmLabel="Move to Trash"
          busyLabel="Moving…"
          danger
          busy={deleting}
          error={error}
          onConfirm={() => void confirmDelete()}
          onCancel={closeQuestion}
        />
      )}
    </>
  );
}

/**
 * The second line of a row: when it started, whether it was cut short, and
 * (in All) its folder.
 */
function MeetingDetail({
  meeting,
  dates,
  showCollection,
}: {
  meeting: MeetingSummary;
  dates: DateContext;
  showCollection: boolean;
}) {
  const date = formatMeetingDate(meeting.startedAt, dates);
  const parts: ReactNode[] = [];
  if (date !== "") {
    parts.push(
      <time
        key="date"
        dateTime={meeting.startedAt}
        title={formatFullDate(meeting.startedAt, dates)}
      >
        {date}
      </time>,
    );
  }
  if (meeting.status === "interrupted") {
    parts.push(
      <span
        key="interrupted"
        className={styles.interrupted}
        title="The recording ended without being stopped. The notes and the transcript up to that point were kept."
      >
        Interrupted
      </span>,
    );
  }
  if (showCollection && meeting.collection !== null) {
    parts.push(
      <span key="collection" className={styles.collection}>
        {meeting.collection}
      </span>,
    );
  }
  return (
    <>
      {parts.map((part, index) => (
        <Fragment key={index}>
          {index > 0 && (
            <>
              {" "}
              <span aria-hidden="true">·</span>{" "}
            </>
          )}
          {part}
        </Fragment>
      ))}
    </>
  );
}
