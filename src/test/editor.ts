// Driving the notes editor in component tests.
//
// CodeMirror keeps its DOM in a shadow root, where Testing Library's queries
// do not look, and jsdom has no layout, so typing by simulated key presses is
// not reliable. The way CodeMirror's author recommends is to get the editor
// from its DOM with `EditorView.findFromDOM` and talk to it directly: these
// helpers do that. Assert on what reaches `api.saveNotes`, not on the DOM.

import { EditorView } from "@codemirror/view";
import { act, fireEvent } from "@testing-library/react";

/** The notes editor on screen, or null when there is none. */
export function queryNotesEditor(): EditorView | null {
  const host = document.querySelector("[data-notes-editor]");
  const dom = host?.shadowRoot?.querySelector<HTMLElement>(".cm-editor");
  return dom ? EditorView.findFromDOM(dom) : null;
}

/** The notes editor on screen. Throws when there is none. */
export function notesEditor(): EditorView {
  const view = queryNotesEditor();
  if (view === null) throw new Error("There is no notes editor on screen.");
  return view;
}

/** The editor's whole text, as it would be saved. */
export function notesText(): string {
  return notesEditor().state.sliceDoc();
}

/** Types `text` at the cursor (replacing the selection), as the keyboard would. */
export function typeInNotes(text: string): void {
  const view = notesEditor();
  act(() => {
    view.dispatch({
      ...view.state.replaceSelection(text),
      userEvent: "input.type",
    });
  });
}

/** Presses a key in the editor, running whatever is bound to it: `pressInNotes("Enter")`. */
export function pressInNotes(key: string, init: KeyboardEventInit = {}): void {
  fireEvent.keyDown(notesEditor().contentDOM, { key, ...init });
}

/** Takes focus away from the editor, as clicking elsewhere or switching windows does. */
export function blurNotes(): void {
  fireEvent.blur(notesEditor().contentDOM);
}

/** True when keyboard focus is in the editor. */
export function notesHaveFocus(): boolean {
  const view = queryNotesEditor();
  if (view === null) return false;
  const host = view.root instanceof ShadowRoot ? view.root.host : null;
  return (
    document.activeElement === host &&
    view.root.activeElement === view.contentDOM
  );
}
