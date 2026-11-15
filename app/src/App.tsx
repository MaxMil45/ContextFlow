import { Camera, Layers, Plus } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { NewWorkspaceDialog } from "./components/dialogs";
import { Palette, type PaletteCommand } from "./components/Palette";
import { Empty, ErrorBanner, SkeletonText, ToastProvider, useToast } from "./components/ui";
import { useCommand } from "./hooks";
import type { CommandError } from "./bindings/CommandError";
import { api, toCommandError, type RestoreReport, type WorkspaceSummary } from "./ipc/client";
import { ShortcutsDialog } from "./components/ShortcutsDialog";
import { ResumeDialog } from "./components/ResumeDialog";
import { SettingsView } from "./views/SettingsView";
import { TimelineView } from "./views/TimelineView";
import { Sidebar } from "./views/Sidebar";
import { WorkspaceView } from "./views/WorkspaceView";

const KIND_LABEL: Record<string, string> = {
  tab: "Tab",
  folder: "Folder",
  note: "Note",
  repository: "Branch",
  window: "Window",
  app: "App",
};

/** Something the tray or the palette asked for, handled by the workspace page once it is showing. */
export type Pending =
  | { kind: "capture" | "restore"; workspaceId: string; key: number; note?: string }
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
  const [view, setView] = useState<"workspace" | "settings" | "timeline">("workspace");
  const parked = useCommand(() => api.listParked(), []);
  const [pending, setPending] = useState<Pending>();
  const [creating, setCreating] = useState(false);
  const [palette, setPalette] = useState(false);
  const [help, setHelp] = useState(false);
  /** After a restart: the workspace to offer to bring back. */
  const [recoverId, setRecoverId] = useState<string>();
  const recovering = recoverId ? workspaces.data?.find((w) => w.id === recoverId) : undefined;

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

  const ask = useCallback((kind: "capture" | "restore", workspaceId: string, note?: string) => {
    setView("workspace");
    setSelectedId(workspaceId);
    setPending({ kind, workspaceId, key: Date.now(), note });
  }, []);

  const resume = async () => {
    const next = await api.takeResume().catch(() => null);
    parked.reload();
    if (next) ask("restore", next.id);
  };

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
          } else if (a.kind === "recover") {
            setRecoverId(a.workspaceId);
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

  // First run: one click saves the desk as it is into a first workspace.
  const toast = useToast();
  const [savingDesk, setSavingDesk] = useState(false);
  const [deskError, setDeskError] = useState<CommandError>();
  const saveDesk = async () => {
    setSavingDesk(true);
    setDeskError(undefined);
    try {
      const ws = await createFirstWorkspace();
      const report = await api.capture({ workspace: ws.id });
      workspaces.reload();
      open(ws.id);
      toast.show(
        `Saved ${report.snapshot.applications.length} programs as “${ws.name}” (F2 renames it). ` +
          "Next, set your desk up for something else and save that as a second workspace. " +
          "Then Ctrl+Alt+W switches between them.",
        { ms: 15000 },
      );
    } catch (e) {
      setDeskError(toCommandError(e));
      workspaces.reload();
    } finally {
      setSavingDesk(false);
    }
  };

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
    { id: "timeline", label: "Timeline", hint: "History", run: () => setView("timeline") },
    ...(parked.data?.length
      ? [{ id: "resume", label: `Resume ${parked.data[0]!.name}`, hint: "Parked", run: () => void resume() }]
      : []),
    { id: "shortcuts", label: "Keyboard shortcuts", hint: "Ctrl+/", run: () => setHelp(true) },
  ];

  return (
    <div className="app">
      <a className="skip-link" href="#main" onClick={(e) => { e.preventDefault(); document.querySelector<HTMLElement>("main")?.focus(); }}>
        Skip to content
      </a>
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
          timelineOpen={view === "timeline"}
          onTimeline={() => setView("timeline")}
          parked={parked.data ?? []}
          onResume={() => void resume()}
          onNew={() => setCreating(true)}
          onPalette={() => setPalette(true)}
        />
        {view === "settings" ? (
          <SettingsView info={info.data} />
        ) : view === "timeline" ? (
          <TimelineView onGoTo={(id) => open(id)} />
        ) : workspaces.error ? (
          <main id="main" tabIndex={-1} className="content">
            <ErrorBanner error={workspaces.error} />
          </main>
        ) : current ? (
          <WorkspaceView
            key={current.id}
            workspace={current}
            onChanged={() => {
              workspaces.reload();
              parked.reload();
            }}
            onSwitchTo={(id, note) => ask("restore", id, note)}
            others={(workspaces.data ?? []).filter((w) => w.id !== current.id && w.latest && w.isTrusted && !w.isArchived)}
            onGone={() => {
              setSelectedId(undefined);
              workspaces.reload();
            }}
            showArchived={showArchived}
            pending={pending?.workspaceId === current.id ? pending : undefined}
            onPendingHandled={() => setPending(undefined)}
          />
        ) : (
          <main id="main" tabIndex={-1} className="content">
            {workspaces.loading && !workspaces.data ? (
              <div className="page">
                <SkeletonText lines={4} />
              </div>
            ) : (
              <Empty
                icon={Layers}
                title="Save your desk as it is"
                actions={
                  <>
                    <button className="primary" onClick={() => void saveDesk()} disabled={savingDesk}>
                      <Camera size={15} aria-hidden /> {savingDesk ? "Saving…" : "Save what's open now"}
                    </button>
                    <button onClick={() => setCreating(true)} disabled={savingDesk}>
                      <Plus size={15} aria-hidden /> New workspace
                    </button>
                  </>
                }
              >
                A workspace remembers your open apps, where their windows sit, your browser tabs and your
                project folders. Save what is open now as your first one, and you can always come back to it
                in one keystroke.
                {deskError && <ErrorBanner error={deskError} />}
              </Empty>
            )}
          </main>
        )}
      </div>
      {creating && <NewWorkspaceDialog onCreated={created} onClose={() => setCreating(false)} />}
      {palette && (
        <Palette
          commands={commands}
          onClose={() => setPalette(false)}
          onSearch={async (q) =>
            (await api.searchWorkspaces(q)).map((h, i) => ({
              id: `hit:${h.workspaceId}:${h.kind}:${i}`,
              label: `${KIND_LABEL[h.kind] ?? h.kind}: ${h.text || h.detail || ""}`,
              hint: h.workspace,
              run: () => open(h.workspaceId),
            }))
          }
        />
      )}
      {help && <ShortcutsDialog onClose={() => setHelp(false)} />}
      {recovering && (
        <ResumeDialog
          workspace={recovering}
          onResume={() => {
            setRecoverId(undefined);
            ask("restore", recovering.id);
          }}
          onClose={() => setRecoverId(undefined)}
        />
      )}
    </div>
  );
}

/** A new workspace for the first-run save, named so it can't clash with an archived one. */
async function createFirstWorkspace(): Promise<WorkspaceSummary> {
  for (let n = 1; ; n++) {
    try {
      return await api.createWorkspace(n === 1 ? "My desk" : `My desk ${n}`);
    } catch (e) {
      if ((e as { code?: string }).code !== "conflict" || n >= 20) throw e;
    }
  }
}
