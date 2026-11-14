import { useState, type FormEvent } from "react";

import { Badge, ErrorBanner, Spinner } from "../components/ui";
import { captureStatusTone, relativeTime } from "../format";
import { api, toCommandError, type CommandError, type WorkspaceSummary } from "../ipc/client";

type Props = {
  workspaces: WorkspaceSummary[] | undefined;
  loading: boolean;
  selected: string | undefined;
  onSelect: (name: string) => void;
  onCreated: (ws: WorkspaceSummary) => void;
  showArchived: boolean;
  onShowArchived: (show: boolean) => void;
  settingsOpen: boolean;
  onSettings: () => void;
};

export function Sidebar({
  workspaces,
  loading,
  selected,
  onSelect,
  onCreated,
  showArchived,
  onShowArchived,
  settingsOpen,
  onSettings,
}: Props) {
  return (
    <aside className="sidebar">
      <header className="sidebar-header">
        <span className="brand">ContextFlow</span>
      </header>
      <NewWorkspace onCreated={onCreated} />
      <nav className="ws-list" aria-label="Workspaces">
        {loading && !workspaces && <Spinner label="Loading…" />}
        {workspaces?.length === 0 && <p className="muted pad">No workspaces yet.</p>}
        {workspaces?.map((ws) => (
          <button
            key={ws.id}
            className={`ws-item${ws.name === selected ? " active" : ""}${ws.isArchived ? " archived" : ""}`}
            onClick={() => onSelect(ws.name)}
          >
            <span className="ws-name">
              {ws.name}
              {ws.isActive && <Badge tone="ok">active</Badge>}
              {ws.isArchived && <Badge tone="muted">archived</Badge>}
            </span>
            <span className="ws-meta">
              {ws.latest ? (
                <>
                  <Badge tone={captureStatusTone[ws.latest.captureStatus]}>v{ws.latest.version}</Badge>
                  <span>{relativeTime(ws.latest.createdAt)}</span>
                </>
              ) : (
                <span className="muted">Not captured yet</span>
              )}
            </span>
          </button>
        ))}
      </nav>
      <div className="sidebar-footer">
        <label className="check">
          <input type="checkbox" checked={showArchived} onChange={(e) => onShowArchived(e.target.checked)} />
          Show archived
        </label>
        <button className={`settings-link${settingsOpen ? " active" : ""}`} onClick={onSettings}>
          Settings
        </button>
      </div>
    </aside>
  );
}

function NewWorkspace({ onCreated }: { onCreated: (ws: WorkspaceSummary) => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError>();

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    setError(undefined);
    try {
      onCreated(await api.createWorkspace(name.trim()));
      setName("");
    } catch (err) {
      setError(toCommandError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="new-ws" onSubmit={submit}>
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="New workspace…"
        aria-label="New workspace name"
        disabled={busy}
      />
      <button type="submit" disabled={busy || !name.trim()}>
        Add
      </button>
      {error && <ErrorBanner error={error} onDismiss={() => setError(undefined)} />}
    </form>
  );
}
