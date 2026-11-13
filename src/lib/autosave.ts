// Saving text a little after typing pauses, without ever losing it.
//
// An `Autosave` holds the latest text and the text the backend is known to
// have. It is plain TypeScript with no React in it, so it keeps working after
// the screen that created it has gone: a save that is still running, or one
// that failed and is being retried, finishes in the background.

import { errorMessage } from "./errors";

export type AutosaveStatus =
  /** The backend has the latest text. */
  | { state: "saved" }
  /** There is text the backend does not have yet; a save is waiting or running. */
  | { state: "pending" }
  /** The last save failed. The text is kept and the save is tried again. */
  | { state: "failed"; message: string };

export interface AutosaveOptions {
  /** The text the backend already has. */
  initial: string;
  /** Sends the text to the backend. A rejection counts as a failed save. */
  save: (text: string) => Promise<void>;
  /** How long typing has to pause before a save. Default 800 ms. */
  delayMs?: number;
  /** The longest that changed text waits while typing never pauses. Default 5 s. */
  maxWaitMs?: number;
  /** The wait before retrying a failed save; it doubles per failure. Default 2 s. */
  retryMs?: number;
  /** The longest wait between retries. Default 30 s. */
  maxRetryMs?: number;
}

type Timer = ReturnType<typeof setTimeout>;

const SAVED: AutosaveStatus = { state: "saved" };
const PENDING: AutosaveStatus = { state: "pending" };

/**
 * Tell it about every change with `change(text)`; it saves after a pause.
 * `flush()` saves right away (on blur, on leaving, before stop).
 *
 * Only one save runs at a time and each one sends the latest text, so saves
 * cannot reach the backend out of order. A failed save keeps the text and is
 * retried until it succeeds.
 */
export class Autosave {
  private latest: string;
  private saved: string;
  private readonly save: (text: string) => Promise<void>;
  private readonly delayMs: number;
  private readonly maxWaitMs: number;
  private readonly retryMs: number;
  private readonly maxRetryMs: number;

  private pauseTimer: Timer | null = null;
  private maxWaitTimer: Timer | null = null;
  private retryTimer: Timer | null = null;
  /** True when a save should start as soon as the running one (if any) ends. */
  private wanted = false;
  private running: Promise<void> | null = null;
  private failures = 0;
  private disposed = false;

  private current: AutosaveStatus = SAVED;
  private readonly listeners = new Set<() => void>();

  constructor(options: AutosaveOptions) {
    this.latest = options.initial;
    this.saved = options.initial;
    this.save = options.save;
    this.delayMs = options.delayMs ?? 800;
    this.maxWaitMs = options.maxWaitMs ?? 5000;
    this.retryMs = options.retryMs ?? 2000;
    this.maxRetryMs = options.maxRetryMs ?? 30_000;
  }

  /** The latest text, saved or not. */
  get text(): string {
    return this.latest;
  }

  /** True when the backend has the latest text and no save is running. */
  get isSettled(): boolean {
    return this.latest === this.saved && this.running === null;
  }

  /** Stays the same object until the status changes. */
  get status(): AutosaveStatus {
    return this.current;
  }

  /** Calls `listener` whenever `status` changes. Returns the unsubscribe function. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** Records the text as it is now and schedules a save for when typing pauses. */
  change(text: string): void {
    if (this.disposed || text === this.latest) return;
    this.latest = text;

    if (this.pauseTimer !== null) clearTimeout(this.pauseTimer);
    this.pauseTimer = null;
    if (this.latest === this.saved && this.running === null) {
      // Typed and undone: there is nothing to save.
      this.settle();
      return;
    }
    // While a save is running, `saved` is about to change, so the comparison
    // above means nothing yet: the timers below make sure it is looked at again.
    this.pauseTimer = setTimeout(this.onTimer, this.delayMs);
    this.maxWaitTimer ??= setTimeout(this.onTimer, this.maxWaitMs);
    if (this.current.state === "saved") this.setStatus(PENDING);
  }

  /**
   * Saves now instead of waiting for a pause. Resolves when the backend has
   * the latest text, or when the attempt failed (the retry then continues on
   * its own). It never rejects.
   */
  flush(): Promise<void> {
    if (this.disposed) return Promise.resolve();
    this.wanted = true;
    this.running ??= this.run().finally(() => {
      this.running = null;
    });
    return this.running;
  }

  /** Stops every timer and forgets unsaved text. For tests; the app never drops text. */
  dispose(): void {
    this.disposed = true;
    this.clearTimers();
    this.listeners.clear();
  }

  private onTimer = (): void => {
    void this.flush();
  };

  private async run(): Promise<void> {
    while (this.wanted && !this.disposed) {
      this.wanted = false;
      this.clearTimers();
      if (this.latest === this.saved) break;

      const text = this.latest;
      try {
        await this.save(text);
      } catch (error) {
        if (this.disposed) return;
        this.failures += 1;
        this.setStatus({ state: "failed", message: errorMessage(error) });
        const wait = Math.min(
          this.retryMs * 2 ** (this.failures - 1),
          this.maxRetryMs,
        );
        this.retryTimer = setTimeout(this.onTimer, wait);
        return;
      }
      this.saved = text;
      this.failures = 0;
    }
    if (this.disposed) return;

    if (this.latest === this.saved) {
      this.settle();
    } else {
      // Text typed while the save ran. Its pause timer is already counting.
      this.setStatus(PENDING);
    }
  }

  private settle(): void {
    this.clearTimers();
    this.failures = 0;
    this.setStatus(SAVED);
  }

  private clearTimers(): void {
    for (const timer of [this.pauseTimer, this.maxWaitTimer, this.retryTimer]) {
      if (timer !== null) clearTimeout(timer);
    }
    this.pauseTimer = null;
    this.maxWaitTimer = null;
    this.retryTimer = null;
  }

  private setStatus(next: AutosaveStatus): void {
    const previous = this.current;
    if (
      previous.state === next.state &&
      (previous.state !== "failed" ||
        (next.state === "failed" && previous.message === next.message))
    ) {
      return;
    }
    this.current = next;
    for (const listener of [...this.listeners]) listener();
  }
}
