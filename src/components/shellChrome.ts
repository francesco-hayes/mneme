// What the app shell already shows around a screen, so a screen does not
// show it twice.

import { createContext } from "react";

/**
 * True inside the shell's sidebar layout, which has its own Settings item:
 * a `Screen` then leaves the Settings button out of its top bar. A screen
 * rendered on its own (in a test, say) keeps the button.
 */
export const SettingsInSidebarContext = createContext(false);
