import { ArrowRightLeft, History, LogOut, ParkingCircle, Play, Undo2 } from "lucide-react";

import { Empty, ErrorBanner, SkeletonList } from "../components/ui";
import { absoluteTime } from "../format";
import { useCommand } from "../hooks";
import { api, type TimelineEntry } from "../ipc/client";

const ICONS = { switch: ArrowRightLeft, restore: Play, leave: LogOut, park: ParkingCircle, resume: Undo2 };

function describe(e: TimelineEntry): string {
  const ws = e.workspace ?? "a deleted workspace";
  switch (e.kind) {
    case "switch":
      return `Switched from ${e.from ?? "a deleted workspace"} to ${ws}`;
    case "restore":
      return `Restored ${ws}`;
    case "leave":
      return `Left ${ws}`;
    case "park":
      return `Parked ${ws} to come back to`;
    case "resume":
      return `Came back to ${ws}`;
    default:
      return ws;
  }
}

function day(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86_400_000);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
}

/** Every switch, restore, leave, park and resume: where your time went, and a way back. */
export function TimelineView({ onGoTo }: { onGoTo: (workspaceId: string) => void }) {
  const events = useCommand(() => api.listTimeline(300), []);
  const groups: Array<[string, TimelineEntry[]]> = [];
  for (const e of events.data ?? []) {
    const d = day(e.at);
    const last = groups.at(-1);
    if (last && last[0] === d) last[1].push(e);
    else groups.push([d, [e]]);
  }
  return (
    <main id="main" tabIndex={-1} className="content">
      <div className="page">
        <header className="page-head">
          <div>
            <h1>Timeline</h1>
            <p className="muted">Where you went, and the notes you left. Kept on this computer only.</p>
          </div>
        </header>
        {events.error && <ErrorBanner error={events.error} />}
        {events.loading && !events.data && <SkeletonList rows={5} />}
        {events.data?.length === 0 && (
          <Empty icon={History} title="Nothing yet">
            Each switch, park and resume appears here, so you can always find your way back.
          </Empty>
        )}
        {groups.map(([d, list]) => (
          <section className="card" key={d}>
            <header className="card-header">
              <h3>{d}</h3>
            </header>
            <ol className="timeline-events">
              {list.map((e, i) => {
                const Icon = ICONS[e.kind as keyof typeof ICONS] ?? History;
                return (
                  <li key={`${e.at}-${i}`} className="timeline-event">
                    <span className="app-icon" aria-hidden>
                      <Icon size={15} />
                    </span>
                    <span className="timeline-event-text">
                      <span>
                        {describe(e)}
                        {e.version !== null && <span className="muted"> · v{e.version}</span>}
                      </span>
                      {e.note && <span className="muted small">“{e.note}”</span>}
                    </span>
                    <span className="muted small num" title={absoluteTime(e.at)}>
                      {new Date(e.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
                    </span>
                    {e.kind === "switch" && e.fromId && e.from && (
                      <button className="ghost" onClick={() => onGoTo(e.fromId!)} data-tip={`Open ${e.from}`}>
                        Go back
                      </button>
                    )}
                  </li>
                );
              })}
            </ol>
          </section>
        ))}
      </div>
    </main>
  );
}
