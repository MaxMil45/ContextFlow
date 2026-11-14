import { Layers, Plus } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { NewWorkspaceDialog } from "./components/dialogs";
import { Palette, type PaletteCommand } from "./components/Palette";
import { Empty, ErrorBanner, SkeletonText, ToastProvider } from "./components/ui";
import { useCommand } from "./hooks";
import { api, type RestoreReport, type WorkspaceSummary } from "./ipc/client";
import { ShortcutsDialog } from "./components/ShortcutsDialog";
import { SettingsView } from "./views/SettingsView";
import { Sidebar } from "./views/Sidebar";
import { WorkspaceView } from "./views/WorkspaceView";

/** Something the tray or the palette asked for, handled by the workspace page once it is showing. */
export type Pending =
  | { kind: "capture" | "restore"; workspaceId: string; key: number }
  | { kind: "report"; workspaceId: string; key: number; report: RestoreReport };

export function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

function Shell() {
  const [showArchived, setShowArchived] = useState(false);
  const workspaces = useCommand(() => api.listWorkspaces(showArchived), [showArchived]);
  const info = useCommand(() => api.appInfo(), []);
  // Workspaces are tracked by id everywhere, so renaming one mid-action can't redirect it.
  const [selectedId, setSelectedId] = useState<string>();
  const [view, setView] = useState<"workspace" | "settings">("workspace");
  const [pending, setPending] = useState<Pending>();
  const [creating, setCreating] = useState(false);
  const [palette, setPalette] = useState(false);
  const [help, setHelp] = useState(false);

  // Default to the most recently used workspace.
  const current =
    workspaces.data?.find((w) => w.id === selectedId) ?? (selectedId ? undefined : workspaces.data?.[0]);

  // The tray's handlers outlive renders; give them the latest list.
  const latest = useRef({ list: workspaces.data, currentId: current?.id, reload: workspaces.reload });
  latest.current = { list: workspaces.data, currentId: current?.id, reload: workspaces.reload };

  const open = useCallback((id: string) => {
    setSelectedId(id);
    setView("workspace");
  }, []);

  const ask = useCallback((kind: "capture" | "restore", workspaceId: string) => {
    setView("workspace");
    setSelectedId(workspaceId);
    setPending({ kind, workspaceId, key: Date.now() });
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const keep = (u: () => void) => (disposed ? u() : unlisteners.push(u));

    // The tray and the switcher leave an action for the window (which may have
    // only just been opened for it): take it now, and whenever another arrives.
    const take = () =>
      void api
        .takeUiAction()
        .then((a) => {
          if (!a) return;
          if (a.kind === "report" && a.report) {
            setView("workspace");
            setSelectedId(a.workspaceId);
            setPending({ kind: "report", workspaceId: a.workspaceId, key: Date.now(), report: a.report });
            latest.current.reload();
          } else if (a.kind === "capture" || a.kind === "restore") {
            ask(a.kind, a.workspaceId);
          }
        })
        .catch(() => {});
    take();
    api.onUiPending(take).then(keep);
    // A quick capture (shortcut or tray) saved a snapshot: show the new version.
    api.onCaptureSaved(() => latest.current.reload()).then(keep);
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [ask]);

  // Keep the tray's "Restore recent" list in step with the workspaces.
  useEffect(() => {
    if (workspaces.data) void api.refreshTrayMenu().catch(() => {});
  }, [workspaces.data]);

  // App-wide keyboard shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = /^(INPUT|TEXTAREA|SELECT)$/.test((e.target as HTMLElement)?.tagName ?? "");
      if (e.key === "?" && !typing && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        setHelp(true);
        return;
      }
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      if (e.key === "/") {
        e.preventDefault();
        setHelp(true);
        return;
      }
      if (e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      } else if (e.key.toLowerCase() === "n" && !typing) {
        e.preventDefault();
        setCreating(true);
      } else if (e.key === ",") {
        e.preventDefault();
        setView("settings");
      } else if (/^[1-9]$/.test(e.key) && !typing) {
        const ws = latest.current.list?.[Number(e.key) - 1];
        if (ws) {
          e.preventDefault();
          open(ws.id);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const created = (ws: WorkspaceSummary, captureNow: boolean) => {
    workspaces.reload();
    if (captureNow) ask("capture", ws.id);
    else open(ws.id);
  };

  const commands: PaletteCommand[] = [
    ...(workspaces.data ?? []).flatMap<PaletteCommand>((w) => [
      { id: `open:${w.id}`, label: w.name, hint: "Open", run: () => open(w.id) },
      ...(w.latest && w.isTrusted && !w.isArchived
        ? [{
              id: `restore:${w.id}`,
              label: w.isActive ? `Restore ${w.name}` : `Switch to ${w.name}`,
              hint: w.isActive ? "Restore" : "Switch",
              run: () => ask("restore", w.id),
            }]
        : []),
    ]),
    { id: "new", label: "New workspace…", hint: "Ctrl+N", run: () => setCreating(true) },
    { id: "settings", label: "Settings", hint: "Ctrl+,", run: () => setView("settings") },
    { id: "shortcuts", label: "Keyboard shortcuts", hint: "Ctrl+/", run: () => setHelp(true) },
  ];

  return (
    <div className="app">
      <div className="shell">
        <Sidebar
          workspaces={workspaces.data}
          loading={workspaces.loading}
          selected={view === "workspace" ? current?.id : undefined}
          onSelect={open}
          showArchived={showArchived}
          onShowArchived={setShowArchived}
          settingsOpen={view === "settings"}
          onSettings={() => setView("settings")}
          onHelp={() => setHelp(true)}
          onNew={() => setCreating(true)}
          onPalette={() => setPalette(true)}
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
            onGone={() => {
              setSelectedId(undefined);
              workspaces.reload();
            }}
            showArchived={showArchived}
            pending={pending?.workspaceId === current.id ? pending : undefined}
            onPendingHandled={() => setPending(undefined)}
          />
        ) : (
          <main className="content">
            {workspaces.loading && !workspaces.data ? (
              <div className="page">
                <SkeletonText lines={4} />
              </div>
            ) : (
              <Empty
                icon={Layers}
                title="Make your first workspace"
                actions={
                  <button className="primary" onClick={() => setCreating(true)}>
                    <Plus size={15} aria-hidden /> New workspace
                  </button>
                }
              >
                A workspace remembers your open apps, where their windows sit, your browser tabs and your
                VS Code folders, so you can come back to them in one click. Open what you need first, then
                create one and save it.
              </Empty>
            )}
          </main>
        )}
      </div>
      {creating && <NewWorkspaceDialog onCreated={created} onClose={() => setCreating(false)} />}
      {palette && <Palette commands={commands} onClose={() => setPalette(false)} />}
      {help && <ShortcutsDialog onClose={() => setHelp(false)} />}
    </div>
  );
}
