import { expect, test } from "vitest";
import { formatElapsed } from "./format";

test("formats recording time as m:ss, and h:mm:ss from an hour on", () => {
  expect(formatElapsed(0)).toBe("0:00");
  expect(formatElapsed(7)).toBe("0:07");
  expect(formatElapsed(754)).toBe("12:34");
  expect(formatElapsed(3599)).toBe("59:59");
  expect(formatElapsed(3723)).toBe("1:02:03");
});

test("treats negative and fractional seconds as whole seconds from zero", () => {
  expect(formatElapsed(-4)).toBe("0:00");
  expect(formatElapsed(61.9)).toBe("1:01");
});
