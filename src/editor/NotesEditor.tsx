import { Compartment, EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { type Ref, useEffect, useImperativeHandle, useRef } from "react";
import { cx } from "../lib/cx";
import { notesExtensions, spellcheckAttributes } from "./extensions";
import styles from "./NotesEditor.module.css";

export interface NotesEditorProps {
  /**
   * The text to open with. Read once, when the editor mounts: later values
   * are ignored, because from then on the editor holds the text.
   */
  initialText: string;
  /**
   * Called after every change with the whole document, which is exactly the
   * string to save: nothing is trimmed, wrapped or normalized.
   */
  onChange: (text: string) => void;
  /** Called when the editor loses focus, including when the window does. */
  onBlur?: () => void;
  /** Puts the cursor in the editor, at the end of the text, when it mounts. */
  autoFocus?: boolean;
  /** The editor's name for screen readers. */
  label: string;
  /** Shown, dimmed, while there is no text. */
  placeholder?: string;
  /** Marks misspelled words. Can change while the editor is open. Default off. */
  spellcheck?: boolean;
  className?: string;
  /** Gives the parent a way to put the cursor in the editor. */
  ref?: Ref<NotesEditorHandle>;
}

export interface NotesEditorHandle {
  /** Moves keyboard focus into the editor; the cursor stays where it was. */
  focus: () => void;
}

/**
 * The Markdown notes editor (CodeMirror 6).
 *
 * The editor lives in a shadow root, and the reason is the app's Content
 * Security Policy, `style-src 'self'`. CodeMirror builds its stylesheet at
 * runtime. Mounted in the document it does that with a `<style>` element,
 * which the policy blocks; mounted in a shadow root it uses a constructed
 * stylesheet (`new CSSStyleSheet()`, filled rule by rule and adopted by the
 * root), which the policy does not cover: it governs style elements and
 * style attributes, not stylesheets built from script. Do not move the
 * editor out of the shadow root without changing how its styles are loaded.
 * The design tokens are CSS variables, so they still reach the editor through
 * the shadow boundary.
 *
 * The component is uncontrolled: it starts from `initialText` and reports
 * changes. Give it a `key` to start over with another document.
 */
export function NotesEditor({
  initialText,
  onChange,
  onBlur,
  autoFocus = false,
  label,
  placeholder,
  spellcheck = false,
  className,
  ref,
}: NotesEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const spellcheckSlot = useRef(new Compartment());
  useImperativeHandle(
    ref,
    () => ({ focus: () => viewRef.current?.focus() }),
    [],
  );

  // The view is created once; it always calls the latest handlers.
  const handlers = useRef({ onChange, onBlur });
  useEffect(() => {
    handlers.current = { onChange, onBlur };
  });
  const initial = useRef({
    initialText,
    autoFocus,
    label,
    placeholder,
    spellcheck,
  });

  useEffect(() => {
    const host = hostRef.current;
    if (host === null) return;
    const { initialText, autoFocus, label, placeholder } = initial.current;
    const spellcheckOn = initial.current.spellcheck;
    // An element can have only one shadow root, and React's development mode
    // runs this effect twice.
    const root = host.shadowRoot ?? host.attachShadow({ mode: "open" });

    const view = new EditorView({
      parent: root,
      root,
      state: EditorState.create({
        doc: initialText,
        extensions: [
          notesExtensions({ text: initialText, label, placeholder }),
          spellcheckSlot.current.of(spellcheckAttributes(spellcheckOn)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              // `sliceDoc` joins the lines with the document's own line break.
              handlers.current.onChange(update.state.sliceDoc());
            }
          }),
          EditorView.domEventHandlers({
            blur: () => {
              handlers.current.onBlur?.();
            },
          }),
        ],
      }),
    });
    if (autoFocus) {
      // The end of the text is the place to carry on writing.
      view.dispatch({ selection: { anchor: view.state.doc.length } });
      view.focus();
    }

    viewRef.current = view;

    return () => {
      viewRef.current = null;
      view.destroy();
    };
  }, []);

  // The setting can change while the editor is open: follow it without
  // starting a new editor.
  useEffect(() => {
    viewRef.current?.dispatch({
      effects: spellcheckSlot.current.reconfigure(
        spellcheckAttributes(spellcheck),
      ),
    });
  }, [spellcheck]);

  return (
    // The attribute is how tests find the element that holds the shadow root.
    <div
      ref={hostRef}
      className={cx(styles.host, className)}
      data-notes-editor=""
    />
  );
}
