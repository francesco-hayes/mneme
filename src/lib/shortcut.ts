// The global shortcut as a string, and as the user sees it.
//
// The saved string is what Tauri's global-shortcut plugin parses (its
// `Shortcut` is `global_hotkey::hotkey::HotKey`, read in global-hotkey 0.10.0,
// `src/hotkey.rs`): tokens joined with `+`, the modifiers first and one key
// last. This module always writes
//
//     [Control+][Alt+][Shift+][Command+]<KeyboardEvent.code>
//
// for example `Command+Shift+KeyR`, `Control+Alt+Space`, `Alt+F5`. The key is
// the W3C `code` of the physical key (`KeyR`, `Digit1`, `Comma`, `ArrowUp`),
// which the parser accepts by that exact name, and which is what a global
// hotkey is registered on. Every key offered here is one the plugin can
// register on macOS (`key_to_scancode` in its macOS backend).

export type ShortcutModifier = "Control" | "Alt" | "Shift" | "Command";

/** The order macOS writes modifiers in: ⌃⌥⇧⌘. Also the order they are saved in. */
const MODIFIERS: readonly ShortcutModifier[] = [
  "Control",
  "Alt",
  "Shift",
  "Command",
];

const MODIFIER_SYMBOLS: Record<ShortcutModifier, string> = {
  Control: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Command: "⌘",
};

const MODIFIER_NAMES: Record<ShortcutModifier, string> = {
  Control: "Control",
  Alt: "Option",
  Shift: "Shift",
  Command: "Command",
};

/** What the parser accepts for each modifier, uppercased. */
const MODIFIER_ALIASES: Record<string, ShortcutModifier> = {
  CONTROL: "Control",
  CTRL: "Control",
  ALT: "Alt",
  OPTION: "Alt",
  SHIFT: "Shift",
  COMMAND: "Command",
  CMD: "Command",
  SUPER: "Command",
  META: "Command",
  // On macOS the "command or control" spellings mean Command.
  COMMANDORCONTROL: "Command",
  COMMANDORCTRL: "Command",
  CMDORCTRL: "Command",
  CMDORCONTROL: "Command",
};

interface KeyLabel {
  /** What the key shows: `R`, `↩`, `Space`. */
  symbol: string;
  /** The key said aloud: `R`, `Return`, `Space`. */
  name: string;
}

function buildKeys(): Map<string, KeyLabel> {
  const keys = new Map<string, KeyLabel>();
  for (const letter of "ABCDEFGHIJKLMNOPQRSTUVWXYZ") {
    keys.set(`Key${letter}`, { symbol: letter, name: letter });
  }
  for (const digit of "0123456789") {
    keys.set(`Digit${digit}`, { symbol: digit, name: digit });
  }
  for (let n = 1; n <= 20; n += 1) {
    keys.set(`F${n}`, { symbol: `F${n}`, name: `F${n}` });
  }
  const others: [string, string, string][] = [
    ["Minus", "-", "Minus"],
    ["Equal", "=", "Equals"],
    ["BracketLeft", "[", "Left Bracket"],
    ["BracketRight", "]", "Right Bracket"],
    ["Backslash", "\\", "Backslash"],
    ["Semicolon", ";", "Semicolon"],
    ["Quote", "'", "Quote"],
    ["Comma", ",", "Comma"],
    ["Period", ".", "Period"],
    ["Slash", "/", "Slash"],
    ["Backquote", "`", "Backtick"],
    ["Space", "Space", "Space"],
    ["Enter", "↩", "Return"],
    ["Backspace", "⌫", "Delete"],
    ["Delete", "⌦", "Forward Delete"],
    ["ArrowUp", "↑", "Up Arrow"],
    ["ArrowDown", "↓", "Down Arrow"],
    ["ArrowLeft", "←", "Left Arrow"],
    ["ArrowRight", "→", "Right Arrow"],
    ["Home", "↖", "Home"],
    ["End", "↘", "End"],
    ["PageUp", "⇞", "Page Up"],
    ["PageDown", "⇟", "Page Down"],
  ];
  for (const [code, symbol, name] of others) keys.set(code, { symbol, name });
  return keys;
}

/** The keys a shortcut can end in, by `KeyboardEvent.code`. */
const KEYS = buildKeys();

/** Uppercased code, and the parser's one-character spellings, to the code. */
const KEY_ALIASES = new Map<string, string>();
for (const [code, label] of KEYS) {
  KEY_ALIASES.set(code.toUpperCase(), code);
  if (label.symbol.length === 1 && label.symbol.charCodeAt(0) < 128) {
    KEY_ALIASES.set(label.symbol, code);
  }
}

const isFunctionKey = (code: string) => /^F\d+$/.test(code);

/** The parts of a keyboard event this module reads. */
export interface ShortcutKeyEvent {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** The modifiers held during `event`, in the order they are written. */
export function heldModifiers(
  event: Pick<ShortcutKeyEvent, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">,
): ShortcutModifier[] {
  const held: Record<ShortcutModifier, boolean> = {
    Control: event.ctrlKey,
    Alt: event.altKey,
    Shift: event.shiftKey,
    Command: event.metaKey,
  };
  return MODIFIERS.filter((modifier) => held[modifier]);
}

/** `⌃⌥` for the modifiers alone, as shown while a combination is being pressed. */
export function modifierSymbols(
  modifiers: readonly ShortcutModifier[],
): string {
  return MODIFIERS.filter((modifier) => modifiers.includes(modifier))
    .map((modifier) => MODIFIER_SYMBOLS[modifier])
    .join("");
}

export type ShortcutCapture =
  /** Not for the recorder: Tab, which must keep moving the keyboard focus. */
  | { kind: "pass" }
  /** Escape: stop recording and keep what was set before. */
  | { kind: "cancel" }
  /** Only modifiers are down so far. */
  | { kind: "modifiers"; modifiers: ShortcutModifier[] }
  /** A combination that cannot be a global shortcut; `reason` says why. */
  | { kind: "rejected"; reason: string }
  /** A complete shortcut, as the string to save. */
  | { kind: "shortcut"; value: string };

export const NEEDS_MODIFIER = "Hold ⌘, ⌃ or ⌥ with the key.";
export const SHIFT_IS_NOT_ENOUGH = "Shift alone is not enough. Add ⌘, ⌃ or ⌥.";
export const KEY_NOT_USABLE = "That key cannot be used in a shortcut.";

/**
 * What a key press means while the recorder is listening.
 *
 * A shortcut is at least one modifier and one key. Shift alone only counts
 * with a function key: Shift with a letter is how capitals are typed, and a
 * global shortcut on it would take that key away from every app.
 */
export function captureShortcut(event: ShortcutKeyEvent): ShortcutCapture {
  if (event.key === "Tab" || event.code === "Tab") return { kind: "pass" };
  if (event.key === "Escape" || event.code === "Escape") {
    return { kind: "cancel" };
  }

  const modifiers = heldModifiers(event);
  if (["Control", "Alt", "Shift", "Meta", "OS"].includes(event.key)) {
    return { kind: "modifiers", modifiers };
  }
  if (!KEYS.has(event.code)) {
    return { kind: "rejected", reason: KEY_NOT_USABLE };
  }
  if (modifiers.length === 0) {
    return { kind: "rejected", reason: NEEDS_MODIFIER };
  }
  if (
    modifiers.length === 1 &&
    modifiers[0] === "Shift" &&
    !isFunctionKey(event.code)
  ) {
    return { kind: "rejected", reason: SHIFT_IS_NOT_ENOUGH };
  }
  return { kind: "shortcut", value: [...modifiers, event.code].join("+") };
}

export interface ShortcutLabel {
  /** Mac symbols: `⇧⌘R`. */
  symbols: string;
  /** The same, said aloud: `Shift Command R`. */
  spoken: string;
}

/**
 * How to show a saved shortcut string, or null for one this module cannot
 * read (the caller then shows the string as it is). Reads everything the
 * plugin's parser reads for the keys above: any case, spaces around tokens,
 * `Cmd`, `Ctrl`, `Option`, `Super`, `CommandOrControl`, and `R` for `KeyR`.
 */
export function describeShortcut(value: string): ShortcutLabel | null {
  const tokens = value.split("+").map((token) => token.trim());
  const keyToken = tokens.pop();
  if (keyToken === undefined || keyToken === "") return null;

  const code = KEY_ALIASES.get(keyToken.toUpperCase());
  const key = code === undefined ? undefined : KEYS.get(code);
  if (key === undefined) return null;

  const held = new Set<ShortcutModifier>();
  for (const token of tokens) {
    const modifier = MODIFIER_ALIASES[token.toUpperCase()];
    if (modifier === undefined) return null;
    held.add(modifier);
  }
  const modifiers = MODIFIERS.filter((modifier) => held.has(modifier));
  return {
    symbols: modifierSymbols(modifiers) + key.symbol,
    spoken: [...modifiers.map((m) => MODIFIER_NAMES[m]), key.name].join(" "),
  };
}
