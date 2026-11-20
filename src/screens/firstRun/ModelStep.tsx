import { useId, useState } from "react";
import {
  Button,
  InlineError,
  ModelList,
  ModelRow,
  Section,
} from "../../components";
import { isDownloadActive } from "../../lib/models";
import { useModelLibrary } from "../../state/modelLibrary";
import styles from "./FirstRun.module.css";

// A placeholder choice until the benchmark in issue 008 is finished: the
// mid-size English model, not a measured recommendation.
export const RECOMMENDED_MODEL_ID = "small.en";

/**
 * Step 1: pick a model and download it. The recommended model is selected to
 * begin with and is the only row shown until the full list is asked for. The
 * rows, their progress, Cancel and Retry are the ones Settings uses, fed by
 * the same model library.
 *
 * There is no Continue button: when the download finishes the library makes
 * the model the default, and `FirstRunScreen` moves on to the permissions.
 */
export function ModelStep() {
  const library = useModelLibrary();
  const radioName = useId();
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);

  const { models, downloads } = library;
  const recommended =
    models.find((model) => model.id === RECOMMENDED_MODEL_ID) ?? models[0];
  const selected = models.find((model) => model.id === chosenId) ?? recommended;
  // One download at a time here: while one runs, the choice is fixed.
  const busy = models.some((model) =>
    isDownloadActive(downloads.get(model.id)),
  );
  const visible = showAll
    ? models
    : models.filter(
        (model) =>
          model.id === recommended?.id ||
          model.id === selected?.id ||
          model.installed ||
          downloads.has(model.id),
      );
  // The selected row has its own controls while it downloads (Cancel), after
  // a failure (Retry) and once it is installed.
  const offerDownload =
    selected !== undefined &&
    !selected.installed &&
    !downloads.has(selected.id);

  return (
    <>
      <p className={styles.step}>Step 1 of 2</p>
      <Section
        title="Choose a transcription model"
        description="Mneme turns speech into text on this Mac, and needs a model to do it. Larger models are more accurate and slower."
      >
        <div role="radiogroup" aria-label="Model to download">
          <ModelList aria-label="Models">
            {visible.map((model) => {
              const download = downloads.get(model.id);
              return (
                <ModelRow
                  key={model.id}
                  model={model}
                  download={download}
                  badge={model.id === recommended?.id ? "Recommended" : null}
                  leading={
                    <input
                      type="radio"
                      className={styles.radio}
                      name={radioName}
                      aria-label={model.name}
                      checked={model.id === selected?.id}
                      disabled={busy}
                      onChange={() => setChosenId(model.id)}
                    />
                  }
                  // Only as Retry, next to the reason it failed: the button
                  // under the list is the one that starts a download.
                  onDownload={
                    download?.phase === "error"
                      ? () => void library.download(model.id)
                      : undefined
                  }
                  onCancelDownload={() => void library.cancelDownload(model.id)}
                  onDismissError={() => library.dismissDownloadError(model.id)}
                />
              );
            })}
          </ModelList>
        </div>

        {library.defaultError !== null && (
          <InlineError
            className={styles.message}
            message={`Could not set the default model: ${library.defaultError}`}
            onRetry={() => void library.load()}
          />
        )}

        <div className={styles.footer}>
          <Button
            variant="plain"
            aria-expanded={showAll}
            onClick={() => setShowAll(!showAll)}
          >
            {showAll ? "Show fewer models" : "Show all models"}
          </Button>
          {offerDownload && (
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void library.download(selected.id)}
            >
              {`Download ${selected.name}`}
            </Button>
          )}
        </div>
      </Section>
    </>
  );
}
