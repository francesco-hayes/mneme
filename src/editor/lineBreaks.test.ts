import { expect, test } from "vitest";
import { soleLineBreak, withLineBreak } from "./lineBreaks";

test("finds the line break a text uses throughout", () => {
  expect(soleLineBreak("a\nb\n")).toBe("\n");
  expect(soleLineBreak("a\r\nb\r\n")).toBe("\r\n");
  expect(soleLineBreak("a\rb")).toBe("\r");
});

test("finds none in one line, or when line breaks are mixed", () => {
  expect(soleLineBreak("")).toBeNull();
  expect(soleLineBreak("one line")).toBeNull();
  expect(soleLineBreak("a\r\nb\nc")).toBeNull();
  expect(soleLineBreak("a\nb\r\nc")).toBeNull();
  expect(soleLineBreak("a\rb\r\nc")).toBeNull();
});

test("an empty line between CRLF lines is still CRLF", () => {
  expect(soleLineBreak("a\r\n\r\nb")).toBe("\r\n");
});

test("gives every line break of a text the same form", () => {
  expect(withLineBreak("a\nb\r\nc\rd", "\r\n")).toBe("a\r\nb\r\nc\r\nd");
  expect(withLineBreak("a\r\n\r\nb", "\n")).toBe("a\n\nb");
});
