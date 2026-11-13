import { useEffect, useId, useRef, useState } from "react";
import {
  Button,
  InlineError,
  ModelList,
  ModelRow,
  Section,
} from "../../components";
import type { ModelInfo } from "../../lib/api";
import { errorMessage } from "../../lib/errors";
import { pickDefaultModel } from "../../lib/models";
import { useModelLibrary } from "../../state/modelLibrary";
import { useRecording } from "../../state/recording";
import styles from "./ModelsSection.module.css";
import type { SettingSaver } from "./useSettingSaver";

export interface ModelsSectionProps {
  /** The default model setting: its value is a model id or null. */
  defaultModel: SettingSaver<string | null>;
}

/**
 * The model manager: the installed models, one of them chosen as the default
 * with a radio button and each with Delete; then the models that can still be
 * downloaded, each with Download, progress, Cancel and Retry.
 */
export function ModelsSection({ defaultModel }: ModelsSectionProps) {
  const library = useModelLibrary();
  const { isActive: recordingActive } = useRecording();
  const radioName = useId();

  const installed = library.models.filter((model) => model.installed);
  const available = library.models.filter((model) => !model.installed);
  const defaultId = defaultModel.value;

  return (
    <Section
      title="Transcription models"
      description="Models run on this Mac. Larger ones are more accurate and slower."
    >
      <h3 className={styles.groupLabel}>Installed</h3>
      {installed.length === 0 ? (
        <p className={styles.empty}>
          No model is installed. Mneme needs one to record: download one below.
        </p>
      ) : (
        <div role="radiogroup" aria-label="Default model">
          <ModelList aria-label="Installed models">
            {installed.map((model) => (
              <InstalledModelRow
                key={model.id}
                model={model}
                models={library.models}
                isDefault={model.id === defaultId}
                defaultId={defaultId}
                radioName={radioName}
                locked={recordingActive}
                onMakeDefault={() => void defaultModel.save(model.id)}
              />
            ))}
          </ModelList>
        </div>
      )}
      {installed.length > 0 && (
        <p className={styles.caption}>
          {recordingActive
            ? "New meetings use the selected model. Models cannot be deleted while a recording is running."
            : "New meetings use the selected model."}
        </p>
      )}
      {defaultModel.error !== null && (
        <InlineError
          className={styles.message}
          message={`Could not change the default model: ${defaultModel.error}`}
        />
      )}
      {library.defaultError !== null && (
        <InlineError
          className={styles.message}
          message={`Could not set the default model: ${library.defaultError}`}
          onRetry={() => void library.load()}
        />
      )}

      {available.length > 0 && (
        <>
          <h3 className={styles.groupLabel}>Available to download</h3>
          <ModelList aria-label="Models to download">
            {available.map((model) => (
              <ModelRow
                key={model.id}
                model={model}
                download={library.downloads.get(model.id)}
                onDownload={() => void library.download(model.id)}
                onCancelDownload={() => void library.cancelDownload(model.id)}
                onDismissError={() => library.dismissDownloadError(model.id)}
              />
            ))}
          </ModelList>
        </>
      )}
    </Section>
  );
}

interface InstalledModelRowProps {
  model: ModelInfo;
  models: readonly ModelInfo[];
  isDefault: boolean;
  defaultId: string | null;
  radioName: string;
  /** A recording is running: nothing can be deleted. */
  locked: boolean;
  onMakeDefault: () => void;
}

function InstalledModelRow({
  model,
  models,
  isDefault,
  defaultId,
  radioName,
  locked,
  onMakeDefault,
}: InstalledModelRowProps) {
  const { deleteModel } = useModelLibrary();
  const [asked, setAsked] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const keepButton = useRef<HTMLButtonElement>(null);
  // A recording that starts while the question is open withdraws it.
  const confirming = asked && !locked;

  // The question opens with the focus on the safe answer, and answering no
  // gives the focus back to Delete, which is only in the page again then.
  const returnFocus = useRef(false);
  useEffect(() => {
    if (confirming) {
      keepButton.current?.focus();
    } else if (returnFocus.current) {
      returnFocus.current = false;
      deleteButton.current?.focus();
    }
  }, [confirming]);

  function closeQuestion() {
    returnFocus.current = true;
    setAsked(false);
  }

  async function confirmDelete() {
    setDeleting(true);
    setError(null);
    try {
      await deleteModel(model.id);
      // The row is gone from this list now; nothing more to do here.
    } catch (failure) {
      setError(errorMessage(failure));
      setDeleting(false);
      setAsked(false);
    }
  }

  // What deleting this model does to the default, said before it is done.
  const others = models.filter((other) => other.id !== model.id);
  const nextDefaultId = pickDefaultModel(others, isDefault ? null : defaultId);
  const nextDefault = others.find((other) => other.id === nextDefaultId);
  const consequence = !isDefault
    ? ""
    : nextDefault !== undefined
      ? ` ${nextDefault.name} becomes the default.`
      : " No model will be left, and Mneme needs one to record.";

  return (
    <ModelRow
      model={model}
      badge={isDefault ? "Default" : undefined}
      leading={
        <input
          type="radio"
          className={styles.radio}
          name={radioName}
          aria-label={`Use ${model.name} for new meetings`}
          checked={isDefault}
          onChange={onMakeDefault}
        />
      }
      installedActions={
        !confirming && (
          <Button
            ref={deleteButton}
            size="small"
            aria-label={`Delete ${model.name}`}
            title={
              locked
                ? "Models cannot be deleted while a recording is running."
                : undefined
            }
            disabled={locked}
            onClick={() => {
              setError(null);
              setAsked(true);
            }}
          >
            Delete
          </Button>
        )
      }
    >
      {confirming && (
        <div
          role="group"
          aria-label={`Delete ${model.name}?`}
          className={styles.confirm}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !deleting) {
              event.stopPropagation();
              closeQuestion();
            }
          }}
        >
          <p className={styles.question}>
            {`Delete ${model.name} from this Mac?${consequence}`}
          </p>
          <Button
            ref={keepButton}
            size="small"
            disabled={deleting}
            onClick={closeQuestion}
          >
            Keep
          </Button>
          <Button
            size="small"
            variant="danger"
            disabled={deleting}
            onClick={() => void confirmDelete()}
          >
            {deleting ? "Deleting…" : "Delete"}
          </Button>
        </div>
      )}
      {error !== null && (
        <InlineError message={`Could not delete ${model.name}: ${error}`} />
      )}
    </ModelRow>
  );
}
