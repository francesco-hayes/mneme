// In-app navigation: which screen is showing, and the way back.
//
// The app has four screens and no URL bar, so this is a small stack of routes
// held in React state instead of a router library. `NavigationProvider` owns
// the stack; screens and shared components read and change it with
// `useNavigation`.

import { createContext, useContext } from "react";

export type Route =
  | { name: "home" }
  | { name: "meeting"; meetingId: string }
  | { name: "settings" }
  | { name: "first-run" };

export interface Navigation {
  /** The screen being shown. */
  route: Route;
  /** False on the first screen of the stack, where there is nothing to go back to. */
  canGoBack: boolean;
  /**
   * Shows `route`. Going to a screen that is already further back in the stack
   * returns to it instead of stacking a second copy, so Back never loops.
   */
  navigate: (route: Route) => void;
  /** Returns to the previous screen. Does nothing when `canGoBack` is false. */
  back: () => void;
  /** Shows `route` and forgets the way back. First run uses it to finish. */
  reset: (route: Route) => void;
}

export function sameRoute(a: Route, b: Route): boolean {
  if (a.name === "meeting" && b.name === "meeting") {
    return a.meetingId === b.meetingId;
  }
  return a.name === b.name;
}

export type NavigationAction =
  | { type: "navigate"; route: Route }
  | { type: "back" }
  | { type: "reset"; route: Route };

/** The stack is never empty; its last entry is the screen being shown. */
export function navigationReducer(
  stack: Route[],
  action: NavigationAction,
): Route[] {
  switch (action.type) {
    case "navigate": {
      const existing = stack.findIndex((route) =>
        sameRoute(route, action.route),
      );
      if (existing === stack.length - 1) return stack;
      if (existing !== -1) return stack.slice(0, existing + 1);
      return [...stack, action.route];
    }
    case "back":
      return stack.length > 1 ? stack.slice(0, -1) : stack;
    case "reset":
      return [action.route];
  }
}

export const NavigationContext = createContext<Navigation | null>(null);

export function useNavigation(): Navigation {
  const navigation = useContext(NavigationContext);
  if (navigation === null) {
    throw new Error("useNavigation needs a NavigationProvider above it.");
  }
  return navigation;
}
