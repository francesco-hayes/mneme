import { useEffect, useState } from "react";
import "./App.css";
import { api, type MeetingSummary } from "./lib/api";

type Load =
  | { state: "loading" }
  | { state: "ready"; meetings: MeetingSummary[] }
  | { state: "error"; message: string };

function describeDuration(durationSecs: number | null): string {
  if (durationSecs === null) return "";
  return `${Math.max(1, Math.round(durationSecs / 60))} min`;
}

/** Placeholder screen that proves the API wiring. Issue 015 replaces it with the real shell. */
export default function App() {
  const [load, setLoad] = useState<Load>({ state: "loading" });

  useEffect(() => {
    let cancelled = false;
    api.listMeetings().then(
      (meetings) => {
        if (!cancelled) setLoad({ state: "ready", meetings });
      },
      (error: unknown) => {
        const message = error instanceof Error ? error.message : String(error);
        if (!cancelled) setLoad({ state: "error", message });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="placeholder">
      <h1>Mneme</h1>
      <p className="hint">Placeholder screen: meetings from the API module.</p>

      {load.state === "loading" && <p>Loading meetings…</p>}
      {load.state === "error" && (
        <p role="alert">Could not load meetings: {load.message}</p>
      )}
      {load.state === "ready" && load.meetings.length === 0 && (
        <p>No meetings yet.</p>
      )}
      {load.state === "ready" && load.meetings.length > 0 && (
        <ul className="meetings">
          {load.meetings.map((meeting) => (
            <li key={meeting.id}>
              <span className="title">{meeting.title}</span>
              {meeting.status !== "complete" && (
                <span className="status">{meeting.status}</span>
              )}
              <span className="meta">
                {new Date(meeting.startedAt).toLocaleDateString(undefined, {
                  month: "short",
                  day: "numeric",
                })}
              </span>
              <span className="meta">
                {describeDuration(meeting.durationSecs)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
