// Line endings, so that the editor hands back the text it was given.
//
// CodeMirror keeps a document as lines, not as one string. By default it
// splits on any of "\n", "\r\n" and "\r" and joins with "\n", which would turn
// a CRLF file into an LF file the first time it is saved. Told one separator
// (`EditorState.lineSeparator`), it splits and joins on exactly that one. So a
// text that uses "\r\n" (or "\r") throughout is opened with that separator and
// comes back byte for byte; a text that mixes line endings cannot be kept
// as it is, and comes back with "\n" once the user changes it.

export type LineBreak = "\n" | "\r\n" | "\r";

const ANY_LINE_BREAK = /\r\n|\r|\n/g;

/** The one line break `text` uses, or null when it has none or mixes several. */
export function soleLineBreak(text: string): LineBreak | null {
  let found: LineBreak | null = null;
  for (const [match] of text.matchAll(ANY_LINE_BREAK)) {
    if (found !== null && found !== match) return null;
    found = match as LineBreak;
  }
  return found;
}

/** `text` with every line break replaced by `lineBreak`. Used on pasted text only. */
export function withLineBreak(text: string, lineBreak: LineBreak): string {
  return text.replace(ANY_LINE_BREAK, lineBreak);
}
