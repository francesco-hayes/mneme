import type { ComponentProps } from "react";
import { cx } from "../lib/cx";
import styles from "./IconButton.module.css";

export interface IconButtonProps extends Omit<
  ComponentProps<"button">,
  "aria-label" | "title"
> {
  /** What the button does. Read by screen readers and shown as the tooltip. */
  label: string;
}

/** A borderless square button holding one icon, for toolbars and rows. */
export function IconButton({
  label,
  type = "button",
  className,
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx(styles.iconButton, className)}
      {...rest}
    />
  );
}
