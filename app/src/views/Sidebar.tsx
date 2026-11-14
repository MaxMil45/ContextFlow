import { ChevronDown, ChevronRight, History, Keyboard, Lock, Plus, Search, Settings, Undo2 } from "lucide-react";

import { Kbd, SkeletonList, WorkspaceDot } from "../components/ui";
import { relativeTime } from "../format";
import type { WorkspaceSummary } from "../ipc/client";

type Props = {
  workspaces: WorkspaceSummary[] | undefined;
  loading: boolean;
  selected: string | undefined;
  onSelect: (id: string) => void;
  onNew: () => void;
  onPalette: () => void;
  showArchived: boolean;
  onShowArchived: (show: boolean) => void;
  settingsOpen: boolean;
  onSettings: () => void;
  onHelp: () => void;
  timelineOpen: boolean;
  onTimeline: () => void;
  /** Parked workspaces, the one Resume returns to first. */
  parked: Array<{ id: string; name: string }>;
  onResume: () => void;
};

export function Sidebar({
  workspaces,
  loading,
  selected,
  onSelect,
  onNew,
  onPalette,
  showArchived,
  onShowArchived,
  settingsOpen,
  onSettings,
  onHelp,
  timelineOpen,
  onTimeline,
  parked,
  onResume,
}: Props) {
  const live = workspaces?.filter((w) => !w.isArchived) ?? [];
  const archived = workspaces?.filter((w) => w.isArchived) ?? [];

  const row = (ws: WorkspaceSummary, n?: number) => (
    <button
      key={ws.id}
      className={`ws-item${ws.id === selected ? " active" : ""}${ws.isArchived ? " archived" : ""}`}
      aria-current={ws.id === selected ? "page" : undefined}
      onClick={() => onSelect(ws.id)}
      title={n ? `Ctrl+${n}` : undefined}
    >
      <WorkspaceDot id={ws.id} name={ws.name} />
      <span className="ws-text">
        <span className="ws-name">
          {ws.name}
          {ws.isActive && <span className="live-dot" role="img" aria-label="You are in this workspace" />}
          {ws.saveMode === "fixed" && (
            <Lock size={11} className="faint" aria-label="Always restores as captured" />
          )}
        </span>
        <span className="ws-meta">
          {ws.latest ? `Saved ${relativeTime(ws.latest.createdAt)}` : "Nothing saved yet"}
        </span>
      </span>
    </button>
  );

  return (
    <aside className="sidebar" aria-label="Sidebar">
      <div className="sidebar-head">
        <span className="sidebar-label">Workspaces</span>
        <button className="ghost icon" onClick={onNew} aria-label="New workspace" data-tip="New workspace (Ctrl+N)">
          <Plus size={16} />
        </button>
      </div>
      <button className="search-hint" onClick={onPalette}>
        <Search size={14} aria-hidden /> Search <Kbd>Ctrl K</Kbd>
      </button>
      {parked.length > 0 && (
        <button className="resume-pill" onClick={onResume} data-tip="Come back to what you parked">
          <Undo2 size={14} aria-hidden /> Resume {parked[0]!.name}
          {parked.length > 1 && <span className="tab-count">+{parked.length - 1}</span>}
        </button>
      )}
      <nav className="ws-list" aria-label="Workspaces">
        {loading && !workspaces && <SkeletonList rows={3} />}
        {workspaces && live.length === 0 && archived.length === 0 && (
          <p className="muted pad">No workspaces yet. Press the + above.</p>
        )}
        {live.map((ws, i) => row(ws, i < 9 ? i + 1 : undefined))}
        <button
          className="group-toggle"
          aria-expanded={showArchived}
          onClick={() => onShowArchived(!showArchived)}
        >
          {showArchived ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          Archived{showArchived && archived.length > 0 ? ` · ${archived.length}` : ""}
        </button>
        {showArchived && archived.map((ws) => row(ws))}
        {showArchived && archived.length === 0 && <p className="muted small pad">Nothing archived.</p>}
      </nav>
      <div className="sidebar-footer">
        <button
          className={`nav-item${timelineOpen ? " active" : ""}`}
          aria-current={timelineOpen ? "page" : undefined}
          onClick={onTimeline}
        >
          <History size={15} aria-hidden /> Timeline
        </button>
        <button
          className={`nav-item${settingsOpen ? " active" : ""}`}
          aria-current={settingsOpen ? "page" : undefined}
          onClick={onSettings}
          title="Ctrl+,"
        >
          <Settings size={15} aria-hidden /> Settings
        </button>
        <button className="nav-item" onClick={onHelp} title="Ctrl+/">
          <Keyboard size={15} aria-hidden /> Keyboard shortcuts
        </button>
      </div>
    </aside>
  );
}
