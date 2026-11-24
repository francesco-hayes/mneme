import { type ReactNode, useMemo, useState } from "react";
import {
  FolderIcon,
  GearIcon,
  IconButton,
  InboxIcon,
  PlusIcon,
  StackIcon,
} from "../components";
import { api } from "../lib/api";
import { cx } from "../lib/cx";
import { FolderNameDialog } from "../screens/home/folders";
import { ALL, type FolderView, sameView } from "../screens/home/folderView";
import { useLibrary } from "../state/library";
import { useNavigation } from "../state/navigation";
import styles from "./Sidebar.module.css";

interface ItemProps {
  icon: ReactNode;
  label: string;
  count?: number;
  current: boolean;
  onClick: () => void;
}

function Item({ icon, label, count, current, onClick }: ItemProps) {
  return (
    <button
      type="button"
      className={cx(styles.item, current && styles.current)}
      aria-current={current ? "page" : undefined}
      title={label}
      onClick={onClick}
    >
      <span className={styles.icon}>{icon}</span>
      <span className={styles.label}>{label}</span>
      {count !== undefined && (
        <span className={styles.count} aria-hidden="true">
          {count}
        </span>
      )}
    </button>
  );
}

/**
 * The window's left column: All meetings, Unfiled, every folder with its
 * meeting count, New folder, and Settings at the bottom. Choosing a folder
 * shows it on Home, from any screen. It reads the same list as Home
 * (`useLibrary`), so the counts and the list agree.
 */
export function Sidebar() {
  const { data, view, setView } = useLibrary();
  const { route, navigate } = useNavigation();
  const [creating, setCreating] = useState(false);

  const library = data.status === "ready" ? data.data : null;
  const counts = useMemo(() => {
    const byCollection = new Map<string, number>();
    let unfiled = 0;
    for (const meeting of library?.meetings ?? []) {
      if (meeting.collection === null) unfiled += 1;
      else {
        byCollection.set(
          meeting.collection,
          (byCollection.get(meeting.collection) ?? 0) + 1,
        );
      }
    }
    return { all: library?.meetings.length, unfiled, byCollection };
  }, [library]);

  const onHome = route.name === "home";
  const isCurrent = (target: FolderView) => onHome && sameView(view, target);
  function show(target: FolderView) {
    setView(target);
    navigate({ name: "home" });
  }

  const collections = library?.collections ?? [];
  return (
    <nav aria-label="Sidebar" className={styles.sidebar}>
      <div className={styles.scroll}>
        <Item
          icon={<StackIcon />}
          label="All meetings"
          count={counts.all}
          current={isCurrent(ALL)}
          onClick={() => show(ALL)}
        />
        <Item
          icon={<InboxIcon />}
          label="Unfiled"
          count={library === null ? undefined : counts.unfiled}
          current={isCurrent({ kind: "unfiled" })}
          onClick={() => show({ kind: "unfiled" })}
        />

        <div className={styles.section}>
          <h2 className={styles.sectionTitle} id="sidebar-folders">
            Folders
          </h2>
          <IconButton
            label="New folder"
            className={styles.add}
            onClick={() => setCreating(true)}
          >
            <PlusIcon />
          </IconButton>
        </div>
        {collections.length > 0 ? (
          <ul aria-labelledby="sidebar-folders" className={styles.folders}>
            {collections.map((name) => {
              const target: FolderView = { kind: "collection", name };
              return (
                <li key={name}>
                  <Item
                    icon={<FolderIcon />}
                    label={name}
                    count={counts.byCollection.get(name) ?? 0}
                    current={isCurrent(target)}
                    onClick={() => show(target)}
                  />
                </li>
              );
            })}
          </ul>
        ) : (
          library !== null && (
            <p className={styles.empty}>Group meetings by client or project.</p>
          )
        )}
      </div>

      <div className={styles.footer}>
        <Item
          icon={<GearIcon />}
          label="Settings"
          current={route.name === "settings"}
          onClick={() => navigate({ name: "settings" })}
        />
      </div>

      {creating && (
        <FolderNameDialog
          save={(name) => api.createCollection(name)}
          onDone={(name) => {
            setCreating(false);
            data.reload();
            show({ kind: "collection", name });
          }}
          onCancel={() => setCreating(false)}
        />
      )}
    </nav>
  );
}
