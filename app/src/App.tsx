import { useEffect, useRef, useState } from "react";

import { Empty, ErrorBanner } from "./components/ui";
import { useCommand } from "./hooks";
import { api } from "./ipc/client";
import { SettingsView } from "./views/SettingsView";
import { Sidebar } from "./views/Sidebar";
import { WorkspaceView } from "./views/WorkspaceView";

/** Something the tray asked for, handled by the workspace page once it is showing. */
export type Pending = { kind: "capture" | "restore"; workspace: string; key: number };

export function App() {
  const [showArchived, setShowArchived] = useState(false);
  const workspaces = useCommand(() => api.listWorkspaces(showArchived), [showArchived]);
  const info = useCommand(() => api.appInfo(), []);
  const [selected, setSelected] = useState<string>();
  const [view, setView] = useState<"workspace" | "settings">("workspace");
  const [pending, setPending] = useState<Pending>();

  // Default to the most recently used workspace.
  const current =
    workspaces.data?.find((w) => w.name === selected) ?? (selected ? undefined : workspaces.data?.[0]);

  // The tray's handlers outlive renders; give them the latest list.
  const latest = useRef({ list: workspaces.data, current: current?.name });
  latest.current = { list: workspaces.data, current: current?.name };

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const keep = (u: () => void) => (disposed ? u() : unlisteners.push(u));

    // Tray: "Capture current…" works on the workspace you're in.
    api
      .onTrayCapture(() => {
        const { list = [], current: name } = latest.current;
        const target = list.find((w) => w.isActive) ?? list.find((w) => w.name === name) ?? list[0];
        if (!target) return;
        setView("workspace");
        setSelected(target.name);
        setPending({ kind: "capture", workspace: target.name, key: Date.now() });
      })
      .then(keep);
    // Tray: "Restore recent ▸ <workspace>".
    api
      .onTrayRestore((workspace) => {
        setView("workspace");
        setSelected(workspace);
        setPending({ kind: "restore", workspace, key: Date.now() });
      })
      .then(keep);
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, []);

  // Keep the tray's "Restore recent" list in step with the workspaces.
  useEffect(() => {
    if (workspaces.data) void api.refreshTrayMenu().catch(() => {});
  }, [workspaces.data]);

  return (
    <div className="shell">
      <Sidebar
        workspaces={workspaces.data}
        loading={workspaces.loading}
        selected={view === "workspace" ? current?.name : undefined}
        onSelect={(name) => {
          setSelected(name);
          setView("workspace");
        }}
        showArchived={showArchived}
        onShowArchived={setShowArchived}
        settingsOpen={view === "settings"}
        onSettings={() => setView("settings")}
        onCreated={(ws) => {
          setSelected(ws.name);
          setView("workspace");
          workspaces.reload();
        }}
      />
      {view === "settings" ? (
        <SettingsView info={info.data} />
      ) : workspaces.error ? (
        <main className="content">
          <ErrorBanner error={workspaces.error} />
        </main>
      ) : current ? (
        <WorkspaceView
          key={current.id}
          workspace={current}
          onChanged={workspaces.reload}
          onRenamed={(ws) => {
            setSelected(ws.name);
            workspaces.reload();
          }}
          // After archiving (when archived are hidden) or deleting: pick the next one.
          onGone={() => {
            setSelected(undefined);
            workspaces.reload();
          }}
          showArchived={showArchived}
          pending={pending?.workspace === current.name ? pending : undefined}
          onPendingHandled={() => setPending(undefined)}
        />
      ) : (
        <main className="content">
          {!workspaces.loading && (
            <Empty title="Create your first workspace">
              <p className="muted">
                Name it on the left, arrange your apps the way you like, then capture.
              </p>
            </Empty>
          )}
        </main>
      )}
    </div>
  );
}
