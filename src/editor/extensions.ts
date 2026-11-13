// What the notes editor is made of. Each piece is added by name instead of
// through CodeMirror's `basicSetup`, which is built for code (line numbers,
// bracket matching, search, completion) and would triple the size.

import {
  history,
  historyKeymap,
  indentWithTab,
  standardKeymap,
} from "@codemirror/commands";
import { markdownKeymap, markdownLanguage } from "@codemirror/lang-markdown";
import { Language } from "@codemirror/language";
import { EditorState, type Extension, Prec } from "@codemirror/state";
import { EditorView, keymap, placeholder } from "@codemirror/view";
import { styleTags } from "@lezer/highlight";
import type { MarkdownParser } from "@lezer/markdown";
import { soleLineBreak, withLineBreak } from "./lineBreaks";
import { listMarkTag, notesTheme } from "./theme";

/**
 * GitHub-flavored Markdown (lists, task lists, tables, strikethrough), parsed
 * for highlighting only. It is the package's `markdownLanguage` with one
 * change: list bullets and numbers get a tag of their own. Sharing that
 * language's `data` is what makes the package's key bindings treat this one
 * as Markdown.
 */
const notesLanguage = new Language(
  markdownLanguage.data,
  // Typed as a general parser, but always the Markdown one.
  (markdownLanguage.parser as MarkdownParser).configure({
    props: [styleTags({ ListMark: listMarkTag })],
  }),
  [],
  "markdown",
);

export interface NotesExtensionsOptions {
  /** The text the editor opens with; its line endings are kept (see lineBreaks.ts). */
  text: string;
  /** The editor's name for screen readers. */
  label: string;
  /** Shown, dimmed, while the document is empty. */
  placeholder?: string;
}

/**
 * Markdown as plain text with highlighting, and nothing that rewrites what the
 * user typed.
 *
 * The language and `markdownKeymap` are used directly instead of through the
 * package's `markdown()` helper: the helper also brings in an HTML parser,
 * HTML tag completion and paste-a-URL-as-a-link, none of which notes need.
 * The keymap continues a list, numbered list, task list or quote on Enter and
 * ends it on Enter in an empty item.
 */
export function notesExtensions(options: NotesExtensionsOptions): Extension {
  const lineBreak = soleLineBreak(options.text);
  // "\n" is what CodeMirror writes anyway, so only the others are configured.
  const keepLineBreak =
    lineBreak === null || lineBreak === "\n"
      ? []
      : [
          EditorState.lineSeparator.of(lineBreak),
          // With a separator configured CodeMirror splits on that one only,
          // so pasted or dropped text has to arrive with it.
          EditorView.clipboardInputFilter.of((text) =>
            withLineBreak(text, lineBreak),
          ),
        ];

  return [
    keepLineBreak,
    history(),
    EditorView.lineWrapping,
    notesLanguage.extension,
    Prec.high(keymap.of(markdownKeymap)),
    // Tab indents a list item and Shift-Tab outdents it. Escape and then Tab
    // moves focus out of the editor (built into CodeMirror).
    keymap.of([...standardKeymap, ...historyKeymap, indentWithTab]),
    notesTheme,
    EditorView.contentAttributes.of({ "aria-label": options.label }),
    options.placeholder ? placeholder(options.placeholder) : [],
  ];
}
