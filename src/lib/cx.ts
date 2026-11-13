/** Joins class names, skipping anything falsy: `cx(styles.row, selected && styles.selected)`. */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}
