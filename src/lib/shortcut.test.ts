import { describe, expect, test } from "vitest";
import {
  captureShortcut,
  describeShortcut,
  heldModifiers,
  KEY_NOT_USABLE,
  modifierSymbols,
  NEEDS_MODIFIER,
  SHIFT_IS_NOT_ENOUGH,
  type ShortcutKeyEvent,
} from "./shortcut";

/** A key event: `press("KeyR", "r", { metaKey: true })`. */
const press = (
  code: string,
  key: string,
  modifiers: Partial<ShortcutKeyEvent> = {},
): ShortcutKeyEvent => ({
  code,
  key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...modifiers,
});

describe("captureShortcut", () => {
  test("modifiers and a key make a shortcut, written modifiers first", () => {
    expect(
      captureShortcut(press("KeyR", "r", { metaKey: true, shiftKey: true })),
    ).toEqual({ kind: "shortcut", value: "Shift+Command+KeyR" });
    expect(
      captureShortcut(press("Space", " ", { ctrlKey: true, altKey: true })),
    ).toEqual({ kind: "shortcut", value: "Control+Alt+Space" });
    expect(captureShortcut(press("F5", "F5", { altKey: true }))).toEqual({
      kind: "shortcut",
      value: "Alt+F5",
    });
  });

  test("the modifiers are always in the order Control, Alt, Shift, Command", () => {
    expect(
      captureShortcut(
        press("Digit1", "!", {
          metaKey: true,
          shiftKey: true,
          altKey: true,
          ctrlKey: true,
        }),
      ),
    ).toEqual({ kind: "shortcut", value: "Control+Alt+Shift+Command+Digit1" });
  });

  test("the key is the physical key, whatever character Option makes of it", () => {
    // Option-E is a dead key on a US keyboard.
    expect(captureShortcut(press("KeyE", "Dead", { altKey: true }))).toEqual({
      kind: "shortcut",
      value: "Alt+KeyE",
    });
    expect(
      captureShortcut(press("Comma", "≤", { altKey: true, metaKey: true })),
    ).toEqual({ kind: "shortcut", value: "Alt+Command+Comma" });
  });

  test("a key with no modifier is refused", () => {
    expect(captureShortcut(press("KeyR", "r"))).toEqual({
      kind: "rejected",
      reason: NEEDS_MODIFIER,
    });
    expect(captureShortcut(press("F5", "F5"))).toEqual({
      kind: "rejected",
      reason: NEEDS_MODIFIER,
    });
    expect(captureShortcut(press("Space", " "))).toEqual({
      kind: "rejected",
      reason: NEEDS_MODIFIER,
    });
  });

  test("Shift alone is not enough for a key that types, but is for a function key", () => {
    expect(captureShortcut(press("KeyR", "R", { shiftKey: true }))).toEqual({
      kind: "rejected",
      reason: SHIFT_IS_NOT_ENOUGH,
    });
    expect(captureShortcut(press("F5", "F5", { shiftKey: true }))).toEqual({
      kind: "shortcut",
      value: "Shift+F5",
    });
  });

  test("modifier keys by themselves are not a shortcut yet", () => {
    expect(
      captureShortcut(press("MetaLeft", "Meta", { metaKey: true })),
    ).toEqual({ kind: "modifiers", modifiers: ["Command"] });
    expect(
      captureShortcut(
        press("ShiftLeft", "Shift", { metaKey: true, shiftKey: true }),
      ),
    ).toEqual({ kind: "modifiers", modifiers: ["Shift", "Command"] });
  });

  test("Escape cancels, with or without modifiers", () => {
    expect(captureShortcut(press("Escape", "Escape"))).toEqual({
      kind: "cancel",
    });
    expect(
      captureShortcut(press("Escape", "Escape", { metaKey: true })),
    ).toEqual({ kind: "cancel" });
  });

  test("Tab is never taken, so the focus can always move on", () => {
    expect(captureShortcut(press("Tab", "Tab"))).toEqual({ kind: "pass" });
    expect(captureShortcut(press("Tab", "Tab", { shiftKey: true }))).toEqual({
      kind: "pass",
    });
    expect(captureShortcut(press("Tab", "Tab", { ctrlKey: true }))).toEqual({
      kind: "pass",
    });
  });

  test("a key the plugin cannot register is refused", () => {
    expect(
      captureShortcut(press("CapsLock", "CapsLock", { metaKey: true })),
    ).toEqual({ kind: "rejected", reason: KEY_NOT_USABLE });
    expect(
      captureShortcut(press("", "Unidentified", { metaKey: true })),
    ).toEqual({ kind: "rejected", reason: KEY_NOT_USABLE });
  });
});

test("heldModifiers and modifierSymbols use the Mac order", () => {
  const all = { ctrlKey: true, altKey: true, shiftKey: true, metaKey: true };
  expect(heldModifiers(all)).toEqual(["Control", "Alt", "Shift", "Command"]);
  expect(modifierSymbols(["Command", "Control"])).toBe("⌃⌘");
  expect(modifierSymbols([])).toBe("");
});

describe("describeShortcut", () => {
  test("shows what the recorder saved, in Mac symbols", () => {
    expect(describeShortcut("Shift+Command+KeyR")).toEqual({
      symbols: "⇧⌘R",
      spoken: "Shift Command R",
    });
    expect(describeShortcut("Control+Alt+Space")).toEqual({
      symbols: "⌃⌥Space",
      spoken: "Control Option Space",
    });
    expect(describeShortcut("Alt+F5")?.symbols).toBe("⌥F5");
    expect(describeShortcut("Command+ArrowUp")?.symbols).toBe("⌘↑");
    expect(describeShortcut("Command+Enter")).toEqual({
      symbols: "⌘↩",
      spoken: "Command Return",
    });
    expect(describeShortcut("Control+Comma")?.symbols).toBe("⌃,");
  });

  test("every shortcut the recorder can produce can be shown", () => {
    const codes = [
      "KeyA",
      "KeyZ",
      "Digit0",
      "Digit9",
      "F1",
      "F20",
      "Minus",
      "Backquote",
      "Space",
      "Enter",
      "Backspace",
      "Delete",
      "ArrowLeft",
      "PageDown",
    ];
    for (const code of codes) {
      const captured = captureShortcut(press(code, "x", { metaKey: true }));
      expect(captured).toEqual({ kind: "shortcut", value: `Command+${code}` });
      expect(describeShortcut(`Command+${code}`)?.symbols).toMatch(/^⌘.+/);
    }
  });

  test("reads the other spellings the plugin's parser accepts", () => {
    expect(describeShortcut("CommandOrControl+Shift+C")?.symbols).toBe("⇧⌘C");
    expect(describeShortcut("cmd+shift+keyr")?.symbols).toBe("⇧⌘R");
    expect(describeShortcut("Ctrl + Option + 1")?.symbols).toBe("⌃⌥1");
    expect(describeShortcut("Super+Meta+/")?.symbols).toBe("⌘/");
    // Whatever order they were written in, they are shown in the Mac order.
    expect(describeShortcut("Command+Shift+Alt+Control+KeyR")?.symbols).toBe(
      "⌃⌥⇧⌘R",
    );
  });

  test("gives up on a string it cannot read", () => {
    expect(describeShortcut("")).toBeNull();
    expect(describeShortcut("Command+")).toBeNull();
    expect(describeShortcut("Hyper+KeyR")).toBeNull();
    expect(describeShortcut("Command+MediaPlayPause")).toBeNull();
  });
});
