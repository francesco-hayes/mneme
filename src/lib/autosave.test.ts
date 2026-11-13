import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { Autosave, type AutosaveOptions } from "./autosave";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const advance = (ms = 0) => vi.advanceTimersByTimeAsync(ms);

/** A save that is settled by hand. */
function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** An autosave over a fake backend that records what it was sent. */
function setup(options: Partial<AutosaveOptions> = {}) {
  const sent: string[] = [];
  const save = vi.fn(async (text: string) => {
    sent.push(text);
  });
  const autosave = new Autosave({ initial: "", save, ...options });
  const statuses: string[] = [];
  autosave.subscribe(() => statuses.push(autosave.status.state));
  return { autosave, save, sent, statuses };
}

describe("when typing pauses", () => {
  test("saves the latest text once, after the delay", async () => {
    const { autosave, sent, statuses } = setup();

    autosave.change("a");
    autosave.change("ab");
    expect(autosave.status.state).toBe("pending");
    await advance(799);
    expect(sent).toEqual([]);

    await advance(1);
    expect(sent).toEqual(["ab"]);
    expect(autosave.status.state).toBe("saved");
    expect(autosave.isSettled).toBe(true);
    expect(statuses).toEqual(["pending", "saved"]);
  });

  test("each change starts the wait again", async () => {
    const { autosave, sent } = setup();

    for (const text of ["a", "ab", "abc"]) {
      autosave.change(text);
      await advance(700);
    }
    expect(sent).toEqual([]);
    await advance(100);
    expect(sent).toEqual(["abc"]);
  });

  test("typing that never pauses is still saved every few seconds", async () => {
    const { autosave, sent } = setup();

    let text = "";
    for (let i = 0; i < 24; i++) {
      text += "x";
      autosave.change(text);
      await advance(500);
    }
    // Twelve seconds of typing with no pause as long as the delay.
    expect(sent).toEqual(["x".repeat(10), "x".repeat(20)]);
  });

  test("text that is typed and then undone is not saved", async () => {
    const { autosave, sent } = setup({ initial: "same" });

    autosave.change("same!");
    autosave.change("same");
    expect(autosave.status.state).toBe("saved");
    await advance(60_000);
    expect(sent).toEqual([]);
  });

  test("the delays can be set", async () => {
    const { autosave, sent } = setup({ delayMs: 50 });

    autosave.change("a");
    await advance(50);
    expect(sent).toEqual(["a"]);
  });
});

describe("flush", () => {
  test("saves at once and resolves when the backend has the text", async () => {
    const pending = deferred();
    const { autosave, save } = setup();
    save.mockImplementationOnce(() => pending.promise);
    let done = false;

    autosave.change("a");
    void autosave.flush().then(() => {
      done = true;
    });
    await advance();
    expect(save).toHaveBeenCalledExactlyOnceWith("a");
    expect(done).toBe(false);
    expect(autosave.isSettled).toBe(false);

    pending.resolve();
    await advance();
    expect(done).toBe(true);
    expect(autosave.isSettled).toBe(true);

    // The timer that was waiting does not save a second time.
    await advance(60_000);
    expect(save).toHaveBeenCalledTimes(1);
  });

  test("does nothing when there is nothing to save", async () => {
    const { autosave, save } = setup();

    await autosave.flush();
    expect(save).not.toHaveBeenCalled();
    expect(autosave.isSettled).toBe(true);
  });
});

describe("order", () => {
  test("only one save runs at a time, and the next one sends the latest text", async () => {
    const first = deferred();
    const { autosave, save, sent } = setup();
    save.mockImplementationOnce((text) => {
      sent.push(text);
      return first.promise;
    });

    autosave.change("a");
    await advance(800);
    autosave.change("ab");
    await advance(800);
    autosave.change("abc");
    void autosave.flush();
    await advance(10_000);
    expect(sent).toEqual(["a"]);

    first.resolve();
    await advance();
    expect(sent).toEqual(["a", "abc"]);
    expect(autosave.isSettled).toBe(true);
  });

  test("text typed while a save runs waits for its own pause", async () => {
    const first = deferred();
    const { autosave, save, sent } = setup();
    save.mockImplementationOnce((text) => {
      sent.push(text);
      return first.promise;
    });

    autosave.change("a");
    await advance(800);
    autosave.change("ab");
    await advance(100);
    first.resolve();
    await advance();
    // The first save is back; "ab" was typed 100 ms ago.
    expect(sent).toEqual(["a"]);
    expect(autosave.status.state).toBe("pending");

    await advance(700);
    expect(sent).toEqual(["a", "ab"]);
  });

  test("going back to the old text while newer text is being saved saves the old text again", async () => {
    const first = deferred();
    const { autosave, save, sent } = setup({ initial: "a" });
    save.mockImplementationOnce((text) => {
      sent.push(text);
      return first.promise;
    });

    autosave.change("ab");
    await advance(800);
    // The backend is receiving "ab"; the user deletes the "b".
    autosave.change("a");
    first.resolve();
    await advance(800);

    expect(sent).toEqual(["ab", "a"]);
    expect(autosave.isSettled).toBe(true);
  });
});

describe("when a save fails", () => {
  test("keeps the text, reports it, and retries with longer and longer waits", async () => {
    const { autosave, save, sent, statuses } = setup();
    save.mockRejectedValue(new Error("The disk is full."));

    autosave.change("keep me");
    await advance(800);
    expect(save).toHaveBeenCalledTimes(1);
    expect(autosave.status).toEqual({
      state: "failed",
      message: "The disk is full.",
    });
    expect(autosave.text).toBe("keep me");
    expect(autosave.isSettled).toBe(false);

    // Retries after 2 s, then 4 s, then 8 s.
    await advance(1999);
    expect(save).toHaveBeenCalledTimes(1);
    await advance(1);
    expect(save).toHaveBeenCalledTimes(2);
    await advance(4000);
    expect(save).toHaveBeenCalledTimes(3);
    await advance(7999);
    expect(save).toHaveBeenCalledTimes(3);
    await advance(1);
    expect(save).toHaveBeenCalledTimes(4);

    save.mockImplementation(async (text) => {
      sent.push(text);
    });
    await advance(16_000);
    expect(sent).toEqual(["keep me"]);
    expect(autosave.status.state).toBe("saved");
    expect(autosave.isSettled).toBe(true);
    // One report of the failure, however many retries it took.
    expect(statuses).toEqual(["pending", "failed", "saved"]);
  });

  test("never waits longer than the longest retry wait", async () => {
    const { autosave, save } = setup({ retryMs: 1000, maxRetryMs: 3000 });
    save.mockRejectedValue(new Error("no"));

    autosave.change("a");
    await advance(800); // fails
    await advance(1000); // fails: retry 1
    await advance(2000); // fails: retry 2
    expect(save).toHaveBeenCalledTimes(3);
    await advance(3000); // capped at 3 s, not 4
    expect(save).toHaveBeenCalledTimes(4);
    await advance(3000);
    expect(save).toHaveBeenCalledTimes(5);
  });

  test("flush does not reject, and typing on saves the newer text", async () => {
    const { autosave, save, sent } = setup();
    save.mockRejectedValueOnce("offline");

    autosave.change("a");
    await expect(autosave.flush()).resolves.toBeUndefined();
    expect(autosave.status).toEqual({ state: "failed", message: "offline" });

    autosave.change("ab");
    // Still failed until a save works: typing does not hide the warning.
    expect(autosave.status.state).toBe("failed");
    await advance(800);
    expect(sent).toEqual(["ab"]);
    expect(autosave.status.state).toBe("saved");
  });
});

test("dispose stops the timers", async () => {
  const { autosave, save } = setup();

  autosave.change("a");
  autosave.dispose();
  await advance(60_000);
  expect(save).not.toHaveBeenCalled();
});
