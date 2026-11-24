// Which meetings Home shows (issue 036): all, the unfiled ones, or one
// folder's. The backend calls folders collections.

export type FolderView =
  { kind: "all" } | { kind: "unfiled" } | { kind: "collection"; name: string };

export const ALL: FolderView = { kind: "all" };

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
