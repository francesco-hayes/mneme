/**
 * The keys that open and close the transcript pane: Shift-Command-T.
 *
 * Chosen because nothing here has it. The editor's key bindings (CodeMirror's
 * standard, history and Markdown keymaps, and Tab) do not use it, and it is
 * not one of the shortcuts macOS gives every app: Command-T alone is Show
 * Fonts and Option-Command-T is Show Toolbar, while Shift-Command-T belongs
 * to single apps (Finder's tab bar, a browser's closed tab). `key` is
 * compared, not `code`, so it is the key labeled T on any keyboard layout.
 */
export function isTranscriptShortcut(event: KeyboardEvent): boolean {
  return (
    event.metaKey &&
    event.shiftKey &&
    !event.altKey &&
    !event.ctrlKey &&
    !event.repeat &&
    event.key.toLowerCase() === "t"
  );
}
