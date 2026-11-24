// Which folder Home shows (issue 036), and the helpers around it.

/** Which meetings Home lists. */
export type FolderView =
  { kind: "all" } | { kind: "unfiled" } | { kind: "collection"; name: string };

export const ALL: FolderView = { kind: "all" };

// The folder Home showed last, kept while the app runs so that opening a
// meeting and coming back lands in the same folder.
let remembered: FolderView = ALL;

export function rememberedFolder(): FolderView {
  return remembered;
}

export function rememberFolder(view: FolderView) {
  remembered = view;
}

/** Forgets the remembered folder. For tests. */
export function resetRememberedFolder() {
  remembered = ALL;
}

/** The collection a view is in, for a start or a filter. */
export function collectionOf(view: FolderView): string | null {
  return view.kind === "collection" ? view.name : null;
}

export function sameView(a: FolderView, b: FolderView): boolean {
  return a.kind === b.kind && collectionOf(a) === collectionOf(b);
}

/** Whether a meeting in `collection` belongs in `view`. */
export function inView(view: FolderView, collection: string | null): boolean {
  switch (view.kind) {
    case "all":
      return true;
    case "unfiled":
      return collection === null;
    case "collection":
      return collection === view.name;
  }
}
