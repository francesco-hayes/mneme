// How the notes editor looks: styled raw Markdown.
//
// Every marker the user types stays in the text and stays visible, dimmed;
// headings, bold, italic, inline code and links are styled where they stand.
// Nothing is hidden or replaced, so what is on screen is what is in the file.
//
// These rules are CodeMirror's own (it turns them into a stylesheet, see
// NotesEditor.tsx). They use the design tokens from styles/tokens.css, which
// are CSS variables and therefore reach into the editor's shadow tree, so the
// editor follows light and dark with the rest of the app and needs no dark
// variant of its own.

import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { Tag, tags } from "@lezer/highlight";

const layout = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "transparent",
    color: "var(--color-text)",
    fontSize: "var(--text-editor)",
  },
  // The editor fills its pane, and the caret shows where focus is.
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-ui)",
    lineHeight: "var(--line-editor)",
    overflowY: "auto",
  },
  ".cm-content": {
    // Room under the last line, so typing does not happen on the bottom edge.
    padding: "var(--space-6) 0 30vh",
    caretColor: "var(--color-accent)",
    // The page turns text selection off and the arrow pointer on for the
    // whole window (styles/base.css); the editor turns them back.
    WebkitUserSelect: "text",
    userSelect: "text",
    cursor: "text",
  },
  ".cm-line": {
    // A readable measure, centered. It is padding on each line, not a maximum
    // width on the text, so that a click beside the text lands in the editor.
    padding: "0 max(var(--space-6), calc((100% - var(--content-width)) / 2))",
  },
  ".cm-placeholder": {
    color: "var(--color-text-tertiary)",
  },
});

/**
 * The tag for a list's bullets and numbers. The Markdown parser files them
 * with every other marker; `notesLanguage` in extensions.ts gives them this
 * tag instead, so they can be dimmed less than the rest.
 */
export const listMarkTag = Tag.define(tags.processingInstruction);

const heading = { fontWeight: "var(--weight-semibold)" };
const secondary = "var(--color-text-secondary)";

/**
 * A node inside a heading or bold text carries that style and its own, so the
 * `#` of a heading is heading-sized and dimmed.
 */
const markdownStyle = HighlightStyle.define([
  { tag: tags.heading1, ...heading, fontSize: "1.6em", fontWeight: "700" },
  { tag: tags.heading2, ...heading, fontSize: "1.3em", fontWeight: "700" },
  { tag: tags.heading3, ...heading, fontSize: "1.12em" },
  { tag: [tags.heading4, tags.heading5, tags.heading6], ...heading },
  { tag: tags.heading, ...heading },
  { tag: tags.strong, fontWeight: "700" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  {
    tag: tags.monospace,
    fontFamily: "var(--font-mono)",
    fontSize: "0.92em",
  },
  { tag: tags.quote, color: secondary },
  { tag: tags.link, color: "var(--color-accent)" },
  // A link's address, a code block's language, a task's checkbox.
  { tag: [tags.url, tags.labelName, tags.string, tags.atom], color: secondary },
  // Markers: `#`, `**`, `_`, backticks, `>`, link brackets, rules. Visible,
  // and out of the way of the words. After the styles above so that their
  // color wins inside a link or a quote.
  {
    tag: [tags.processingInstruction, tags.contentSeparator, tags.comment],
    color: "var(--color-text-tertiary)",
  },
  // Bullets and numbers are read along with the text: dimmed, but less.
  { tag: listMarkTag, color: secondary },
]);

export const notesTheme: Extension = [
  layout,
  syntaxHighlighting(markdownStyle),
];
