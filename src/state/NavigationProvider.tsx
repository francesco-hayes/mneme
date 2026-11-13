import { type ReactNode, useMemo, useReducer } from "react";
import {
  type Navigation,
  NavigationContext,
  navigationReducer,
  type Route,
} from "./navigation";

interface NavigationProviderProps {
  /** The screen to start on. Read once, when the provider mounts. */
  initialRoute: Route;
  children: ReactNode;
}

export function NavigationProvider({
  initialRoute,
  children,
}: NavigationProviderProps) {
  const [stack, dispatch] = useReducer(navigationReducer, [initialRoute]);

  const navigation = useMemo<Navigation>(
    () => ({
      route: stack[stack.length - 1],
      canGoBack: stack.length > 1,
      navigate: (route) => dispatch({ type: "navigate", route }),
      back: () => dispatch({ type: "back" }),
      reset: (route) => dispatch({ type: "reset", route }),
    }),
    [stack],
  );

  return <NavigationContext value={navigation}>{children}</NavigationContext>;
}
