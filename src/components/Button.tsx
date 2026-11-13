import type { ComponentProps } from "react";
import { cx } from "../lib/cx";
import styles from "./Button.module.css";

export interface ButtonProps extends ComponentProps<"button"> {
  /**
   * `default` is the standard push button, `primary` the one main action of a
   * screen (accent colored), `danger` a destructive action, and `plain` a
   * borderless button for quiet actions inside rows and bars.
   */
  variant?: "default" | "primary" | "danger" | "plain";
  size?: "regular" | "small";
}

/** A push button. Takes every `<button>` attribute; `type` defaults to `button`. */
export function Button({
  variant = "default",
  size = "regular",
  type = "button",
  className,
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(styles.button, styles[variant], styles[size], className)}
      {...rest}
    />
  );
}
