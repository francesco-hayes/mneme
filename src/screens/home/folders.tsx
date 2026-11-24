// Folders on Home (issue 036). The backend calls them collections: folders of
// meeting folders in the meetings root.

import { type KeyboardEvent, useId, useState } from "react";
import { ConfirmDialog, PlusIcon } from "../../components";
import { cx } from "../../lib/cx";
import { errorMessage } from "../../lib/errors";
import { ALL, collectionOf, type FolderView, sameView } from "./folderView";
import styles from "./folders.module.css";

export interface FolderBarProps {
  collections: readonly string[];
  view: FolderView;
  onSelect: (view: FolderView) => void;
  onNewFolder: () => void;
}

/**
 * The row of folders above the list: All, Unfiled (once there are folders),
 * each folder, and New folder…. The chosen one is pressed.
 */
export function FolderBar({
  collections,
  view,
  onSelect,
  onNewFolder,
}: FolderBarProps) {
  const choices: { label: string; view: FolderView }[] = [
    { label: "All", view: ALL },
    ...(collections.length > 0
      ? [{ label: "Unfiled", view: { kind: "unfiled" } as FolderView }]
      : []),
    ...collections.map((name) => ({
      label: name,
      view: { kind: "collection", name } as FolderView,
    })),
  ];
  return (
    <nav aria-label="Folders" className={styles.bar}>
      {choices.map((choice) => {
        const pressed = sameView(choice.view, view);
        return (
          <button
            key={`${choice.view.kind}:${collectionOf(choice.view) ?? ""}`}
            type="button"
            aria-pressed={pressed}
            className={cx(
              styles.chip,
              choice.view.kind === "collection" && styles.folder,
              pressed && styles.pressed,
            )}
            onClick={() => onSelect(choice.view)}
          >
            {choice.label}
          </button>
        );
      })}
      <button
        type="button"
        className={cx(styles.chip, styles.add)}
        onClick={onNewFolder}
      >
        <PlusIcon />
        New folder…
      </button>
    </nav>
  );
}

export interface FolderNameDialogProps {
  /** Set when renaming: the folder's current name, also the field's start value. */
  current?: string;
  /** Makes or renames the folder and returns its name as stored. */
  save: (name: string) => Promise<string>;
  onDone: (name: string) => void;
  onCancel: () => void;
}

/** Asks for a folder name, for New folder… and Rename…. */
export function FolderNameDialog({
  current,
  save,
  onDone,
  onCancel,
}: FolderNameDialogProps) {
  const fieldId = useId();
  const [name, setName] = useState(current ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const empty = name.trim() === "";

  async function confirm() {
    if (empty || busy) return;
    setBusy(true);
    setError(null);
    try {
      onDone(await save(name));
    } catch (failure) {
      setError(errorMessage(failure));
      setBusy(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      void confirm();
    }
  }

  return (
    <ConfirmDialog
      title={current === undefined ? "New folder" : `Rename “${current}”`}
      confirmLabel={current === undefined ? "Create" : "Rename"}
      busyLabel={current === undefined ? "Creating…" : "Renaming…"}
      busy={busy}
      error={error}
      confirmDisabled={empty}
      onConfirm={() => void confirm()}
      onCancel={onCancel}
    >
      <label htmlFor={fieldId} className={styles.label}>
        Name
      </label>
      <input
        id={fieldId}
        type="text"
        className={styles.field}
        value={name}
        disabled={busy}
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => setName(event.target.value)}
        onKeyDown={onKeyDown}
      />
    </ConfirmDialog>
  );
}

export interface MoveMeetingDialogProps {
  title: string;
  /** Where the meeting is now. */
  collection: string | null;
  collections: readonly string[];
  move: (collection: string | null) => Promise<void>;
  onDone: () => void;
  onCancel: () => void;
}

/** Move to…: a meeting's new folder, or No folder. */
export function MoveMeetingDialog({
  title,
  collection,
  collections,
  move,
  onDone,
  onCancel,
}: MoveMeetingDialogProps) {
  const group = useId();
  const [choice, setChoice] = useState<string | null>(collection);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    setBusy(true);
    setError(null);
    try {
      await move(choice);
      onDone();
    } catch (failure) {
      setError(`Could not move the meeting: ${errorMessage(failure)}`);
      setBusy(false);
    }
  }

  const options: { label: string; value: string | null }[] = [
    { label: "No folder", value: null },
    ...collections.map((name) => ({ label: name, value: name })),
  ];
  return (
    <ConfirmDialog
      title={`Move “${title}” to…`}
      message={
        collections.length === 0
          ? "There are no folders yet. Make one with New folder… on Home."
          : undefined
      }
      confirmLabel="Move"
      busyLabel="Moving…"
      busy={busy}
      error={error}
      confirmDisabled={choice === collection}
      onConfirm={() => void confirm()}
      onCancel={onCancel}
    >
      <div role="radiogroup" aria-label="Folder" className={styles.choices}>
        {options.map((option) => (
          <label key={option.value ?? ""} className={styles.choice}>
            <input
              type="radio"
              name={group}
              checked={choice === option.value}
              disabled={busy}
              onChange={() => setChoice(option.value)}
            />
            {option.label}
          </label>
        ))}
      </div>
    </ConfirmDialog>
  );
}

export interface DeleteFolderDialogProps {
  name: string;
  meetingCount: number;
  remove: () => Promise<void>;
  onDone: () => void;
  onCancel: () => void;
}

/** Delete… on a folder: it goes to the Trash with its meetings. */
export function DeleteFolderDialog({
  name,
  meetingCount,
  remove,
  onDone,
  onCancel,
}: DeleteFolderDialogProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    setBusy(true);
    setError(null);
    try {
      await remove();
      onDone();
    } catch (failure) {
      setError(`Could not delete the folder: ${errorMessage(failure)}`);
      setBusy(false);
    }
  }

  const contents =
    meetingCount === 0
      ? "It is empty."
      : meetingCount === 1
        ? "Its meeting, with notes and transcript, goes with it."
        : `Its ${meetingCount} meetings, with notes and transcripts, go with it.`;
  return (
    <ConfirmDialog
      title={`Move the folder “${name}” to the Trash?`}
      message={`${contents} Put Back in Finder restores it.`}
      confirmLabel="Move to Trash"
      busyLabel="Moving…"
      danger
      busy={busy}
      error={error}
      onConfirm={() => void confirm()}
      onCancel={onCancel}
    />
  );
}
