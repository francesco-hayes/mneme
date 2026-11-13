import { fireEvent, renderHook } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { useWindowEvent } from "./useWindowEvent";

test("calls the latest handler while mounted, and stops after", () => {
  const first = vi.fn();
  const second = vi.fn();
  const { rerender, unmount } = renderHook(
    ({ handler }) => useWindowEvent("keydown", handler),
    { initialProps: { handler: first } },
  );

  fireEvent.keyDown(document.body, { key: "n" });
  expect(first).toHaveBeenCalledOnce();
  expect(first.mock.calls[0][0].key).toBe("n");

  rerender({ handler: second });
  fireEvent.keyDown(window, { key: "ArrowDown" });
  expect(first).toHaveBeenCalledOnce();
  expect(second).toHaveBeenCalledOnce();

  unmount();
  fireEvent.keyDown(window, { key: "n" });
  expect(second).toHaveBeenCalledOnce();
});
