// One `Autosave` per meeting whose notes have been edited in this session.
//
// The autosave outlives the meeting view on purpose. Leaving the view starts
// a last save and does not wait for it, and a save that failed keeps being
// retried in the background. If the same meeting is opened again before that
// has finished, the backend's copy of the notes is older than what the user
// typed, so the view gets the same autosave back and shows its text.

import { api } from "../../lib/api";
import { Autosave } from "../../lib/autosave";

const savers = new Map<string, Autosave>();

/**
 * The autosave for a meeting's notes. `loadedNotes` is what `api.getMeeting`
 * just returned; it is used unless an earlier autosave for the meeting still
 * holds newer text, in which case that one is returned and `text` on it is
 * what the editor should open with.
 */
export function notesAutosave(
  meetingId: string,
  loadedNotes: string,
): Autosave {
  const existing = savers.get(meetingId);
  if (existing !== undefined && !existing.isSettled) return existing;

  // Finished ones have nothing left to do.
  for (const [id, saver] of savers) {
    if (saver.isSettled) savers.delete(id);
  }
  const saver = new Autosave({
    initial: loadedNotes,
    save: (text) => api.saveNotes(meetingId, text),
  });
  savers.set(meetingId, saver);
  return saver;
}

/** Drops every autosave and its timers, unsaved text included. For tests only. */
export function resetNotesAutosave(): void {
  for (const saver of savers.values()) saver.dispose();
  savers.clear();
}
