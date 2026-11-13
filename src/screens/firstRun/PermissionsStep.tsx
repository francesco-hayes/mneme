import { type ReactNode, useEffect, useState } from "react";
import {
  Button,
  CheckIcon,
  Group,
  GroupRow,
  InlineError,
  Section,
} from "../../components";
import { api, type ModelInfo, type Permissions } from "../../lib/api";
import { cx } from "../../lib/cx";
import { errorMessage } from "../../lib/errors";
import { useLoad } from "../../lib/useLoad";
import styles from "./FirstRun.module.css";

type PermissionKind = keyof Permissions;

interface PermissionText {
  label: string;
  /** How the buttons name it: "Allow the microphone". */
  spoken: string;
  why: string;
  /** What to do in System Settings once it has been denied. */
  fix: string;
}

const TEXT: Record<PermissionKind, PermissionText> = {
  microphone: {
    label: "Microphone",
    spoken: "the microphone",
    why: "Records your side of the meeting.",
    fix: "Mneme is not allowed to use the microphone. In System Settings, open Privacy & Security > Microphone and turn on Mneme.",
  },
  systemAudio: {
    label: "System audio",
    spoken: "system audio",
    why: "Records the other side of the call: the sound your Mac plays.",
    fix: "Mneme is not allowed to record system audio. In System Settings, open Privacy & Security > Screen & System Audio Recording and turn on Mneme under System Audio Recording Only. If it still shows as not allowed, quit Mneme and open it again.",
  },
};

const readPermissions = () => api.getPermissions();

export interface PermissionsStepProps {
  /** The model that was just installed and is now the default. */
  model: ModelInfo;
  /** Leaves First run. */
  onFinish: () => void;
}

/**
 * Step 2: the two permissions a recording uses. The status shown is always
 * what `api.getPermissions` says: it is read when the step opens, after every
 * request, and when the window gets the focus back (the user returning from
 * System Settings).
 *
 * The microphone is required to finish. System audio is not: without it a
 * recording has only the user's side, and the Finish button says so.
 */
export function PermissionsStep({ model, onFinish }: PermissionsStepProps) {
  const permissions = useLoad(readPermissions);
  const { reload } = permissions;
  const [requesting, setRequesting] = useState<PermissionKind | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);

  useEffect(() => {
    window.addEventListener("focus", reload);
    return () => window.removeEventListener("focus", reload);
  }, [reload]);

  // Shows the system's prompt, or System Settings when the answer was no.
  const request = async (kind: PermissionKind) => {
    setRequesting(kind);
    setRequestError(null);
    try {
      await api.requestPermission(kind);
    } catch (error) {
      setRequestError(
        `Could not ask for ${TEXT[kind].spoken}: ${errorMessage(error)}`,
      );
    } finally {
      setRequesting(null);
      reload();
    }
  };

  let content: ReactNode;
  if (permissions.status === "error") {
    content = (
      <InlineError
        className={styles.message}
        message={`Could not read the permissions: ${permissions.message}`}
        onRetry={reload}
      />
    );
  } else if (permissions.status === "loading") {
    content = (
      <p className={cx(styles.loading, "appears-late")}>
        Checking permissions…
      </p>
    );
  } else {
    const { microphone, systemAudio } = permissions.data;
    const micGranted = microphone === "granted";
    const kinds: PermissionKind[] = ["microphone", "systemAudio"];
    content = (
      <>
        <Group>
          {kinds.map((kind) => (
            <PermissionRow
              key={kind}
              kind={kind}
              status={permissions.data[kind]}
              busy={requesting !== null}
              onRequest={() => void request(kind)}
            />
          ))}
        </Group>
        {requestError !== null && (
          <InlineError className={styles.message} message={requestError} />
        )}
        <p className={styles.advice}>
          Use headphones: without them the microphone also hears the other side
          of the call.
        </p>
        <div className={styles.footer}>
          <p className={styles.footerNote}>
            {!micGranted
              ? "Mneme cannot record without the microphone. Allow it to finish."
              : systemAudio !== "granted"
                ? "Without system audio, a recording has only your side."
                : null}
          </p>
          <Button variant="primary" disabled={!micGranted} onClick={onFinish}>
            {micGranted && systemAudio !== "granted"
              ? "Finish without system audio"
              : "Finish"}
          </Button>
        </div>
      </>
    );
  }

  return (
    <>
      <p className={styles.step}>Step 2 of 2</p>
      <Section
        title="Allow recording"
        description={`${model.name} is installed. Mneme records two things, and macOS asks you about each.`}
      >
        {content}
      </Section>
    </>
  );
}

interface PermissionRowProps {
  kind: PermissionKind;
  status: Permissions[PermissionKind];
  /** A request is open: no second one can be made. */
  busy: boolean;
  onRequest: () => void;
}

function PermissionRow({ kind, status, busy, onRequest }: PermissionRowProps) {
  const text = TEXT[kind];
  return (
    <GroupRow
      label={text.label}
      description={text.why}
      below={status === "denied" && <p className={styles.denied}>{text.fix}</p>}
    >
      <span
        data-permission={kind}
        data-status={status}
        className={styles.status}
      >
        {status === "granted" ? (
          <>
            <CheckIcon />
            Allowed
          </>
        ) : status === "denied" ? (
          <>
            <span className={styles.notAllowed}>Not allowed</span>
            <Button
              size="small"
              aria-label={`Open System Settings for ${text.spoken}`}
              disabled={busy}
              onClick={onRequest}
            >
              Open System Settings
            </Button>
          </>
        ) : (
          <Button
            size="small"
            aria-label={`Allow ${text.spoken}`}
            disabled={busy}
            onClick={onRequest}
          >
            Allow…
          </Button>
        )}
      </span>
    </GroupRow>
  );
}
