import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import { useLoad } from "./useLoad";

/** A load function whose calls are settled by hand, in order. */
function manualLoad() {
  const calls: {
    resolve: (value: string) => void;
    reject: (error: Error) => void;
  }[] = [];
  const load = () =>
    new Promise<string>((resolve, reject) => {
      calls.push({ resolve, reject });
    });
  return { load, calls };
}

test("goes from loading to ready", async () => {
  const { load, calls } = manualLoad();
  const { result } = renderHook(() => useLoad(load));
  expect(result.current.status).toBe("loading");

  act(() => calls[0].resolve("first"));
  await waitFor(() =>
    expect(result.current).toMatchObject({ status: "ready", data: "first" }),
  );
});

test("reports a failure, and reload shows loading and then the new result", async () => {
  const { load, calls } = manualLoad();
  const { result } = renderHook(() => useLoad(load));

  act(() => calls[0].reject(new Error("No such folder.")));
  await waitFor(() =>
    expect(result.current).toMatchObject({
      status: "error",
      message: "No such folder.",
    }),
  );

  act(() => result.current.reload());
  expect(result.current.status).toBe("loading");
  act(() => calls[1].resolve("second"));
  await waitFor(() =>
    expect(result.current).toMatchObject({ status: "ready", data: "second" }),
  );
});

test("reload keeps showing the data it has until the new result arrives", async () => {
  const { load, calls } = manualLoad();
  const { result } = renderHook(() => useLoad(load));
  act(() => calls[0].resolve("first"));
  await waitFor(() => expect(result.current.status).toBe("ready"));

  act(() => result.current.reload());
  expect(result.current).toMatchObject({ status: "ready", data: "first" });
  act(() => calls[1].resolve("second"));
  await waitFor(() =>
    expect(result.current).toMatchObject({ status: "ready", data: "second" }),
  );
});

test("a new load function starts over and ignores the old one's result", async () => {
  const a = manualLoad();
  const b = manualLoad();
  const { result, rerender } = renderHook(({ load }) => useLoad(load), {
    initialProps: { load: a.load },
  });

  rerender({ load: b.load });
  expect(result.current.status).toBe("loading");
  act(() => a.calls[0].resolve("from a"));
  act(() => b.calls[0].resolve("from b"));
  await waitFor(() =>
    expect(result.current).toMatchObject({ status: "ready", data: "from b" }),
  );
});
