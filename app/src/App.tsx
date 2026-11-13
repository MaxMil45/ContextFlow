import { useState } from "react";

import { Empty, ErrorBanner } from "./components/ui";
import { useCommand } from "./hooks";
import { api } from "./ipc/client";
import { Sidebar } from "./views/Sidebar";
import { WorkspaceView } from "./views/WorkspaceView";

export function App() {
  const workspaces = useCommand(() => api.listWorkspaces(), []);
  const [selected, setSelected] = useState<string>();

  // Default to the most recently used workspace.
  const current =
    workspaces.data?.find((w) => w.name === selected) ?? (selected ? undefined : workspaces.data?.[0]);

  return (
    <div className="shell">
      <Sidebar
        workspaces={workspaces.data}
        loading={workspaces.loading}
        selected={current?.name}
        onSelect={setSelected}
        onCreated={(ws) => {
          setSelected(ws.name);
          workspaces.reload();
        }}
      />
      {workspaces.error ? (
        <main className="content">
          <ErrorBanner error={workspaces.error} />
        </main>
      ) : current ? (
        <WorkspaceView key={current.id} workspace={current} onChanged={workspaces.reload} />
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
