// Keyboard focus for the rows of a `List`. Only rows that open something (a
// `ListRow` with `onActivate`) take part; controls at the end of a row do not.

const rowButtons = (list: HTMLElement) =>
  Array.from(list.querySelectorAll<HTMLButtonElement>(":scope > li > button"));

/**
 * Puts keyboard focus on the first or last row of the list. Returns false when
 * the list has no row to focus.
 */
export function focusListRow(
  list: HTMLElement,
  which: "first" | "last",
): boolean {
  const row = rowButtons(list).at(which === "first" ? 0 : -1);
  row?.focus();
  return row !== undefined;
}

/**
 * Moves focus from the row holding it to the next (`1`) or previous (`-1`)
 * one, stopping at the ends. Returns false when focus is not on a row.
 */
export function moveListFocus(list: HTMLElement, by: 1 | -1): boolean {
  const rows = rowButtons(list);
  const current = rows.findIndex((row) => row === document.activeElement);
  if (current === -1) return false;
  rows[Math.min(rows.length - 1, Math.max(0, current + by))].focus();
  return true;
}
