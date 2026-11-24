import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { cx } from "../lib/cx";
import { IconButton } from "./IconButton";
import styles from "./MenuButton.module.css";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  /** Drawn in the danger color: Delete. */
  danger?: boolean;
  disabled?: boolean;
  /** Tooltip, for example why the item is disabled. */
  title?: string;
}

export interface MenuButtonProps {
  /** What the button is for, such as "Options for Standup". Read by screen readers and shown as the tooltip. */
  label: string;
  /** The icon on the button. */
  icon: ReactNode;
  items: readonly MenuItem[];
  className?: string;
}

const ITEM = '[role="menuitem"]:not(:disabled)';

/**
 * An icon button that opens a short menu of actions under it, right edges
 * lined up. The arrow keys, Home and End move between the items, Escape and
 * Tab close it, and so does a press anywhere else. Choosing an item closes the
 * menu, puts the focus back on the button, then runs the item.
 *
 * Keys handled here are marked handled, so a `List` around it does not also
 * move between its rows.
 */
export function MenuButton({ label, icon, items, className }: MenuButtonProps) {
  const [open, setOpen] = useState(false);
  const wrapper = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();

  function close(refocus: boolean) {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  }

  // Opening puts the focus on the first item that can be chosen.
  useEffect(() => {
    if (open) menu.current?.querySelector<HTMLElement>(ITEM)?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: Event) {
      const target = event.target;
      if (target instanceof Node && !wrapper.current?.contains(target)) {
        setOpen(false);
      }
    }
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  function moveFocus(key: string) {
    const root = menu.current;
    if (root === null) return;
    const enabled = Array.from(root.querySelectorAll<HTMLElement>(ITEM));
    if (enabled.length === 0) return;
    const index = enabled.findIndex((item) => item === document.activeElement);
    const last = enabled.length - 1;
    const next =
      key === "Home"
        ? 0
        : key === "End"
          ? last
          : key === "ArrowDown"
            ? index < 0
              ? 0
              : Math.min(index + 1, last)
            : index < 0
              ? last
              : Math.max(index - 1, 0);
    enabled[next].focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    switch (event.key) {
      case "Escape":
        if (open) {
          event.preventDefault();
          event.stopPropagation();
          close(true);
        }
        break;
      case "ArrowDown":
      case "ArrowUp":
        event.preventDefault();
        if (open) moveFocus(event.key);
        else setOpen(true);
        break;
      case "Home":
      case "End":
        if (open) {
          event.preventDefault();
          moveFocus(event.key);
        }
        break;
      case "Tab":
        if (open) close(false);
        break;
    }
  }

  return (
    <div
      ref={wrapper}
      className={cx(styles.menuButton, className)}
      onKeyDown={onKeyDown}
    >
      <IconButton
        ref={trigger}
        label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        className={cx(open && styles.pressed)}
        onClick={() => (open ? close(false) : setOpen(true))}
      >
        {icon}
      </IconButton>
      {open && (
        <div
          ref={menu}
          id={menuId}
          role="menu"
          aria-label={label}
          className={styles.menu}
        >
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              className={cx(styles.item, item.danger && styles.danger)}
              disabled={item.disabled}
              title={item.title}
              onClick={() => {
                close(true);
                item.onSelect();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
