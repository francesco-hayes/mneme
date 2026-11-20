import {
  type KeyboardEvent,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  Button,
  CheckIcon,
  ChevronUpDownIcon,
  DownloadProgress,
  IconButton,
  InlineError,
  TrashIcon,
  WarningIcon,
} from "../../components";
import type { ModelInfo } from "../../lib/api";
import { cx } from "../../lib/cx";
import { errorMessage } from "../../lib/errors";
import { formatBytes } from "../../lib/format";
import { type DownloadStatus, pickDefaultModel } from "../../lib/models";
import { useModelLibrary } from "../../state/modelLibrary";
import styles from "./ModelPicker.module.css";

export interface ModelPickerProps {
  /** The id of the row's label: the button is named by it and by the model it shows. */
  labelId: string;
  /** The model new meetings use, or null. */
  defaultId: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** A recording is running: nothing can be deleted. */
  locked: boolean;
  /** An installed model was picked as the default. */
  onChoose: (id: string) => void;
}

/** The first control of each row: where the arrow keys stop. */
const PRIMARY = "[data-primary]:not(:disabled)";

/**
 * The model manager in the space of one control. Closed, it is a button that
 * shows the model new meetings use. Open, it is a menu under that button:
 * the installed models, where a click picks the default and each has Delete;
 * then the models that can still be downloaded, each with Download, progress,
 * Cancel and Retry.
 *
 * It is not a `<select>` or a listbox, whose options cannot hold buttons. It
 * is a button that discloses a group of ordinary controls, so Tab reaches
 * every one of them. The arrow keys, Home and End also move between rows, and
 * Escape or a click elsewhere closes it.
 */
export function ModelPicker({
  labelId,
  defaultId,
  open,
  onOpenChange,
  locked,
  onChoose,
}: ModelPickerProps) {
  const { models, downloads } = useModelLibrary();
  const triggerId = useId();
  const menuId = useId();
  const wrapper = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  // Where the focus was inside the menu, to put it back when the control
  // that held it is replaced (Download by Cancel, Delete by its question).
  const focusWasInside = useRef(false);
  const focusedModelId = useRef<string | null>(null);

  const installed = models.filter((model) => model.installed);
  const available = models.filter((model) => !model.installed);
  const current = installed.find((model) => model.id === defaultId);
  const triggerText =
    current?.name ??
    (installed.length === 0 ? "No model installed" : "Choose a model");

  function close(returnFocus: boolean) {
    focusWasInside.current = false;
    focusedModelId.current = null;
    onOpenChange(false);
    if (returnFocus) trigger.current?.focus();
  }

  // The menu opens with the focus on the model in use, or on its first row.
  useEffect(() => {
    if (!open) return;
    const root = menu.current;
    const first =
      root?.querySelector<HTMLElement>('[aria-checked="true"]') ??
      root?.querySelector<HTMLElement>(PRIMARY) ??
      root;
    first?.focus();
  }, [open]);

  // A press anywhere else closes it.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: Event) {
      const target = event.target;
      if (target instanceof Node && !wrapper.current?.contains(target)) {
        focusWasInside.current = false;
        focusedModelId.current = null;
        onOpenChange(false);
      }
    }
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open, onOpenChange]);

  // A control that is removed while it has the focus drops the focus on the
  // page, where the arrow keys and Escape no longer reach the menu. Put it
  // back on the same model's row.
  useLayoutEffect(() => {
    const root = menu.current;
    if (!open || root === null || !focusWasInside.current) return;
    if (document.activeElement !== document.body) return;
    const id = focusedModelId.current;
    const row =
      id === null
        ? null
        : Array.from(
            root.querySelectorAll<HTMLElement>("[data-model-id]"),
          ).find((item) => item.dataset.modelId === id);
    (row?.querySelector<HTMLElement>(PRIMARY) ?? root).focus();
  });

  function moveFocus(key: string) {
    const root = menu.current;
    if (root === null) return;
    const controls = Array.from(root.querySelectorAll<HTMLElement>(PRIMARY));
    if (controls.length === 0) return;
    const active = document.activeElement;
    const row = active instanceof Element ? active.closest("li") : null;
    const index =
      row === null
        ? -1
        : controls.findIndex((control) => control.closest("li") === row);
    const last = controls.length - 1;
    let next: number;
    if (key === "Home") next = 0;
    else if (key === "End") next = last;
    else if (key === "ArrowDown")
      next = index < 0 ? 0 : Math.min(index + 1, last);
    else next = index < 0 ? last : Math.max(index - 1, 0);
    controls[next]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    switch (event.key) {
      case "Escape":
        if (open) {
          event.preventDefault();
          event.stopPropagation();
          close(true);
        }
        break;
      case "ArrowDown":
      case "ArrowUp":
        event.preventDefault();
        if (open) moveFocus(event.key);
        else onOpenChange(true);
        break;
      case "Home":
      case "End":
        if (open) {
          event.preventDefault();
          moveFocus(event.key);
        }
        break;
    }
  }

  return (
    <div
      ref={wrapper}
      className={styles.picker}
      onKeyDown={onKeyDown}
      onBlur={(event) => {
        // Tab took the focus out of the menu: it does not stay open behind.
        const next = event.relatedTarget;
        if (open && next instanceof Node && !wrapper.current?.contains(next)) {
          close(false);
        }
      }}
    >
      <button
        ref={trigger}
        id={triggerId}
        type="button"
        className={styles.trigger}
        aria-haspopup="true"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-labelledby={`${labelId} ${triggerId}`}
        onClick={() => (open ? close(false) : onOpenChange(true))}
      >
        <span className={cx(styles.value, current === undefined && styles.dim)}>
          {triggerText}
        </span>
        <ChevronUpDownIcon />
      </button>

      {open && (
        <div
          ref={menu}
          id={menuId}
          role="group"
          aria-label="Transcription models"
          tabIndex={-1}
          className={styles.menu}
          onFocus={(event) => {
            focusWasInside.current = true;
            const row = event.target.closest<HTMLElement>("[data-model-id]");
            focusedModelId.current = row?.dataset.modelId ?? null;
          }}
        >
          {installed.length > 0 && (
            <>
              <div className={styles.groupLabel}>Installed</div>
              <div role="radiogroup" aria-label="Default model">
                <ul className={styles.list} aria-label="Installed models">
                  {installed.map((model) => (
                    <InstalledRow
                      key={model.id}
                      model={model}
                      models={models}
                      isDefault={model.id === defaultId}
                      defaultId={defaultId}
                      locked={locked}
                      onChoose={() => {
                        onChoose(model.id);
                        close(true);
                      }}
                    />
                  ))}
                </ul>
              </div>
            </>
          )}
          {installed.length > 0 && available.length > 0 && (
            <div className={styles.divider} />
          )}
          {available.length > 0 && (
            <>
              <div className={styles.groupLabel}>Not downloaded</div>
              <ul className={styles.list} aria-label="Models to download">
                {available.map((model) => (
                  <AvailableRow
                    key={model.id}
                    model={model}
                    download={downloads.get(model.id)}
                  />
                ))}
              </ul>
            </>
          )}
          {locked && installed.length > 0 && (
            <p className={styles.caption}>
              Models cannot be deleted while a recording is running.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

interface InstalledRowProps {
  model: ModelInfo;
  models: readonly ModelInfo[];
  isDefault: boolean;
  defaultId: string | null;
  locked: boolean;
  onChoose: () => void;
}

function InstalledRow({
  model,
  models,
  isDefault,
  defaultId,
  locked,
  onChoose,
}: InstalledRowProps) {
  const { deleteModel } = useModelLibrary();
  const nameId = useId();
  const [asked, setAsked] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const keepButton = useRef<HTMLButtonElement>(null);
  // A recording that starts while the question is open withdraws it.
  const confirming = asked && !locked;

  // The question opens with the focus on the safe answer, and answering no
  // gives the focus back to Delete, which is only in the menu again then.
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
    <li
      className={styles.row}
      aria-labelledby={nameId}
      data-model-id={model.id}
      data-state="installed"
    >
      <div className={styles.line}>
        <button
          type="button"
          role="radio"
          aria-checked={isDefault}
          aria-label={`Use ${model.name} for new meetings`}
          className={styles.choice}
          data-primary
          onClick={onChoose}
        >
          <span className={styles.check}>{isDefault && <CheckIcon />}</span>
          <span id={nameId} className={styles.name}>
            {model.name}
          </span>
          <span className={styles.size}>{formatBytes(model.sizeBytes)}</span>
        </button>
        {!confirming && (
          <IconButton
            ref={deleteButton}
            label={`Delete ${model.name}`}
            className={styles.delete}
            disabled={locked}
            onClick={() => {
              setError(null);
              setAsked(true);
            }}
          >
            <TrashIcon />
          </IconButton>
        )}
      </div>
      {confirming && (
        <div
          role="group"
          aria-label={`Delete ${model.name}?`}
          className={cx(styles.below, styles.confirm)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && !deleting) {
              // Answers the question; the menu stays open.
              event.stopPropagation();
              closeQuestion();
            }
          }}
        >
          <p className={styles.question}>
            {`Delete ${model.name} from this Mac?${consequence}`}
          </p>
          <div className={styles.answers}>
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
        </div>
      )}
      {error !== null && (
        <InlineError
          className={styles.below}
          message={`Could not delete ${model.name}: ${error}`}
        />
      )}
    </li>
  );
}

function AvailableRow({
  model,
  download,
}: {
  model: ModelInfo;
  download: DownloadStatus | undefined;
}) {
  const nameId = useId();
  const failed = download?.phase === "error";
  const active = download !== undefined && !failed;

  return (
    <li
      className={styles.row}
      aria-labelledby={nameId}
      data-model-id={model.id}
      data-state={download?.phase ?? "available"}
    >
      <div className={styles.line}>
        <span className={styles.check} />
        <span id={nameId} className={cx(styles.name, styles.dim)}>
          {model.name}
        </span>
        {/* While a download runs, its own line gives received and total. */}
        {!active && (
          <span className={styles.size}>{formatBytes(model.sizeBytes)}</span>
        )}
        <DownloadActions model={model} download={download} />
      </div>
      {active && (
        <DownloadProgress
          className={styles.below}
          name={model.name}
          status={download}
        />
      )}
      <DownloadMessage
        className={styles.below}
        model={model}
        download={download}
      />
    </li>
  );
}

interface DownloadPartProps {
  model: ModelInfo;
  download: DownloadStatus | undefined;
  className?: string;
}

/** The buttons a model that is not installed has: Download, Cancel, or Retry and Dismiss. */
function DownloadActions({ model, download }: DownloadPartProps) {
  const library = useModelLibrary();

  if (download === undefined) {
    return (
      <Button
        size="small"
        aria-label={`Download ${model.name}`}
        data-primary
        onClick={() => void library.download(model.id)}
      >
        Download
      </Button>
    );
  }
  if (download.phase === "error") {
    return (
      <span className={styles.actions}>
        <Button
          size="small"
          variant="plain"
          aria-label={`Dismiss the error for ${model.name}`}
          onClick={() => library.dismissDownloadError(model.id)}
        >
          Dismiss
        </Button>
        <Button
          size="small"
          aria-label={`Retry downloading ${model.name}`}
          data-primary
          onClick={() => void library.download(model.id)}
        >
          Retry
        </Button>
      </span>
    );
  }
  return (
    <Button
      size="small"
      aria-label={`Cancel downloading ${model.name}`}
      data-primary
      // Nothing to cancel until the backend has the request, and nothing
      // more to ask for once Cancel has been pressed.
      disabled={
        download.phase === "starting" || download.phase === "cancelling"
      }
      onClick={() => void library.cancelDownload(model.id)}
    >
      Cancel
    </Button>
  );
}

/** Why a download failed, or why its Cancel could not be carried out. */
function DownloadMessage({ model, download, className }: DownloadPartProps) {
  if (download === undefined || download.error === null) return null;
  return (
    <p role="alert" className={cx(styles.error, className)}>
      <WarningIcon />
      <span className="selectable">
        {download.phase === "error"
          ? `Could not download ${model.name}: ${download.error}`
          : download.error}
      </span>
    </p>
  );
}

/**
 * A download as the Settings row shows it while the menu is closed: one line
 * with the model's name, the bar, how far it is, and Cancel; or, after a
 * failure, the reason with Retry and Dismiss.
 */
export function DownloadStrip({
  model,
  download,
}: {
  model: ModelInfo;
  download: DownloadStatus;
}) {
  const failed = download.phase === "error";
  return (
    <div className={styles.strip} data-state={download.phase}>
      <div className={styles.stripLine}>
        {failed ? (
          <DownloadMessage
            className={styles.stripText}
            model={model}
            download={download}
          />
        ) : (
          <>
            {/* The bar carries the same words as its name. */}
            <span className={styles.stripLabel} aria-hidden="true">
              {`Downloading ${model.name}`}
            </span>
            <DownloadProgress
              className={styles.stripProgress}
              name={model.name}
              status={download}
            />
          </>
        )}
        <DownloadActions model={model} download={download} />
      </div>
      {!failed && <DownloadMessage model={model} download={download} />}
    </div>
  );
}
