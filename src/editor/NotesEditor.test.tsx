import { EditorView } from "@codemirror/view";
import { render } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import {
  blurNotes,
  notesEditor,
  notesHaveFocus,
  notesText,
  pressInNotes,
  typeInNotes,
} from "../test/editor";
import { NotesEditor } from "./NotesEditor";

function renderEditor(initialText: string, autoFocus = false) {
  const onChange = vi.fn<(text: string) => void>();
  const onBlur = vi.fn();
  const view = render(
    <NotesEditor
      initialText={initialText}
      onChange={onChange}
      onBlur={onBlur}
      autoFocus={autoFocus}
      label="Notes"
    />,
  );
  return { onChange, onBlur, ...view };
}

/** Text that a tidy-minded editor would be tempted to change. */
const AWKWARD = [
  "  leading spaces",
  "trailing spaces   ",
  "\ttab",
  "",
  "",
  "*  odd   list",
  "1) other numbering",
  "<b>html</b> &amp; entities",
  "no newline at the end",
].join("\n");

test("holds the text it was given, character for character", () => {
  const { onChange } = renderEditor(AWKWARD);

  expect(notesText()).toBe(AWKWARD);
  expect(onChange).not.toHaveBeenCalled();
});

test("reports the whole text after each change, and nothing else is touched", () => {
  const { onChange } = renderEditor(AWKWARD);

  typeInNotes("x");
  expect(onChange).toHaveBeenCalledExactlyOnceWith(`x${AWKWARD}`);
});

test("keeps CRLF line endings, in what it reads and in what is typed", () => {
  const { onChange } = renderEditor("one\r\ntwo");
  const view = notesEditor();
  // Two lines: the line ending is not part of either.
  expect(view.state.doc.lines).toBe(2);
  expect(view.state.doc.line(1).text).toBe("one");

  view.dispatch({ selection: { anchor: view.state.doc.length } });
  pressInNotes("Enter");
  typeInNotes("three");
  expect(onChange).toHaveBeenLastCalledWith("one\r\ntwo\r\nthree");
});

test("pasted text takes on the line ending of a CRLF document", () => {
  renderEditor("one\r\ntwo");
  const view = notesEditor();
  // The functions CodeMirror runs on pasted and dropped text.
  const paste = view.state.facet(EditorView.clipboardInputFilter);

  expect(paste).toHaveLength(1);
  expect(paste[0]("a\nb\r\nc", view.state)).toBe("a\r\nb\r\nc");
});

test("a text with mixed line endings comes back with \\n once it is changed", () => {
  const { onChange } = renderEditor("one\r\ntwo\nthree");

  typeInNotes("x");
  expect(onChange).toHaveBeenLastCalledWith("xone\ntwo\nthree");
});

test("later values of initialText do not replace what is in the editor", () => {
  const { rerender, onChange } = renderEditor("first");
  typeInNotes("typed ");

  rerender(
    <NotesEditor initialText="second" onChange={onChange} label="Notes" />,
  );
  expect(notesText()).toBe("typed first");
});

test("takes the focus only when asked, with the cursor after the text", () => {
  const first = renderEditor("some notes");
  expect(notesHaveFocus()).toBe(false);
  first.unmount();

  renderEditor("some notes", true);
  expect(notesHaveFocus()).toBe(true);
  typeInNotes("!");
  expect(notesText()).toBe("some notes!");
});

test("tells when it loses focus", () => {
  const { onBlur } = renderEditor("", true);

  blurNotes();
  expect(onBlur).toHaveBeenCalledOnce();
});

test("is named for screen readers and wraps long lines", () => {
  renderEditor("");
  const content = notesEditor().contentDOM;

  expect(content).toHaveAttribute("aria-label", "Notes");
  expect(content).toHaveAttribute("role", "textbox");
  expect(content).toHaveAttribute("aria-multiline", "true");
  expect(content).toHaveClass("cm-lineWrapping");
  // No line numbers or other gutters.
  expect(notesEditor().dom.querySelector(".cm-gutters")).toBeNull();
});

// The app's Content Security Policy is `style-src 'self'`: a <style> element
// in the page is blocked. CodeMirror makes one when it is mounted in the
// document, and uses a constructed stylesheet (which is allowed) when it is
// mounted in a shadow root. So the editor must stay in a shadow root.
test("lives in a shadow root and adds no style element to the page", () => {
  const { container } = renderEditor("# Heading");
  const host = container.querySelector("[data-notes-editor]")!;

  expect(notesEditor().root).toBe(host.shadowRoot);
  expect(document.querySelectorAll("style")).toHaveLength(0);
  expect(document.querySelectorAll("[style]")).toHaveLength(0);
});

test("the editor is removed when the component goes away", () => {
  const { container, unmount } = renderEditor("text");
  const host = container.querySelector("[data-notes-editor]")!;
  const root = host.shadowRoot!;
  expect(root.querySelector(".cm-editor")).not.toBeNull();

  unmount();
  expect(root.querySelector(".cm-editor")).toBeNull();
});
