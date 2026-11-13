// Text formats shared by more than one screen.

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Recording time for the timer: `0:07`, `12:34`, and `1:02:03` from an hour on. */
export function formatElapsed(totalSecs: number): string {
  const secs = Math.max(0, Math.floor(totalSecs));
  const hours = Math.floor(secs / 3600);
  const minutes = Math.floor(secs / 60) % 60;
  const seconds = pad2(secs % 60);
  return hours > 0
    ? `${hours}:${pad2(minutes)}:${seconds}`
    : `${minutes}:${seconds}`;
}
