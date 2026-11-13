import { expect, test } from "vitest";
import { navigationReducer, type Route, sameRoute } from "./navigation";

const home: Route = { name: "home" };
const settings: Route = { name: "settings" };
const meetingA: Route = { name: "meeting", meetingId: "a" };
const meetingB: Route = { name: "meeting", meetingId: "b" };

test("meeting routes are the same only for the same meeting", () => {
  expect(sameRoute(meetingA, { name: "meeting", meetingId: "a" })).toBe(true);
  expect(sameRoute(meetingA, meetingB)).toBe(false);
  expect(sameRoute(home, { name: "home" })).toBe(true);
  expect(sameRoute(home, settings)).toBe(false);
});

test("navigate adds a screen and back removes it", () => {
  const stack = navigationReducer([home], {
    type: "navigate",
    route: meetingA,
  });
  expect(stack).toEqual([home, meetingA]);
  expect(navigationReducer(stack, { type: "back" })).toEqual([home]);
});

test("navigating to the screen already showing changes nothing", () => {
  const stack = [home, meetingA];
  expect(navigationReducer(stack, { type: "navigate", route: meetingA })).toBe(
    stack,
  );
});

test("navigating to a screen further back returns to it", () => {
  const stack = [home, meetingA, settings];
  expect(
    navigationReducer(stack, { type: "navigate", route: meetingA }),
  ).toEqual([home, meetingA]);
  expect(navigationReducer(stack, { type: "navigate", route: home })).toEqual([
    home,
  ]);
});

test("a different meeting is a new screen", () => {
  expect(
    navigationReducer([home, meetingA], { type: "navigate", route: meetingB }),
  ).toEqual([home, meetingA, meetingB]);
});

test("back on the first screen changes nothing", () => {
  const stack = [home];
  expect(navigationReducer(stack, { type: "back" })).toBe(stack);
});

test("reset forgets the way back", () => {
  expect(
    navigationReducer([{ name: "first-run" }, settings], {
      type: "reset",
      route: home,
    }),
  ).toEqual([home]);
});
