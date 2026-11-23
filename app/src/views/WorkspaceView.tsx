import {
  ParkingCircle,
  ArrowRightLeft,
  Archive,
  ArchiveRestore,
  Camera,
  FileDown,
  GitBranch,
  GitCompareArrows,
  History,
  Layers,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Play,
  Eye,
  Shield,
  MonitorUp,
  Trash2,
  LogOut,
  Clock,
} from "lucide-react";
import { useEffect, useState } from "react";

import type { Pending } from "../App";
import { CaptureReview } from "../components/CaptureReview";
import { CompareDialog } from "../components/CompareDialog";
import { ConfirmDialog, DeleteWorkspaceDialog, EditWorkspaceDialog, TrustDialog } from "../components/dialogs";
import { CleanupResults, CleanupSection, PlanItemStatus, PlanSummary } from "../components/CleanupSection";
import { IconPicker } from "../components/IconPicker";
import { CommandsSection } from "../components/CommandsSection";
import { ExportDialog } from "../components/ExportDialogs";
import {
  AppIcon,
  Badge,
  Empty,
  ErrorBanner,
  Kbd,
  Menu,
  MenuItem,
  Notice,
  SkeletonList,
  SkeletonText,
  useToast,
} from "../components/ui";
import {
  absoluteTime,
  captureStatusTone,
  friendlyAppName,
  plural,
  relativeTime,
  restoreStatusLabel,
  restoreStatusTone,
} from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type CaptureReport,
  type CleanupReport,
  type CommandError,
  type RestoreReport,
  type SaveMode,
  type SnapshotDetail,
  type SnapshotEntry,
  type WorkspaceSummary,
} from "../ipc/client";
import { RestorePanel } from "./RestorePanel";

type Props = {
  workspace: WorkspaceSummary;
  onChanged: () => void;
  /** The workspace was deleted, or archived while archived ones are hidden. */
  onGone: () => void;
  showArchived: boolean;
  /** Something the tray asked for (capture, restore), done once this page shows. */
  pending?: Pending;
  onPendingHandled: () => void;
  /** Go to another workspace (a switch), leaving `note` with this one. */
  onSwitchTo: (workspaceId: string, note?: string) => void;
  /** Workspaces you could switch to from here. */
  others: WorkspaceSummary[];
};

type Tab = "overview" | "rules" | "history";

type Dialog =
  | "rename"
  | "delete"
  | "approve"
  | "replaceIcon"
  | "leave"
  | "park"
  | "export"
  | "deleteOld"
  | { deleteVersion: number }
  | { compare: { from: number; to?: number } };

type RestoreRun = {
  dryRun: boolean;
  mode: "restore" | "switch";
  version?: number;
  only?: string[];
  key: number;
  /** Already finished elsewhere (the switcher): just show it. */
  report?: RestoreReport;
  /** A note for the workspace being left. */
  leaveNote?: string;
};

const statusLabel = { complete: "Complete", partial: "Partial", failed: "Failed" } as const;

export function WorkspaceView({ workspace, onChanged, onGone, showArchived, pending, onPendingHandled, onSwitchTo, others }: Props) {
  const { id, name } = workspace;
  const toast = useToast();
  const history = useCommand(() => api.listSnapshots(id), [id]);
  const [tab, setTab] = useState<Tab>("overview");
  const [selectedVersion, setSelectedVersion] = useState<number>();
  const version = selectedVersion ?? workspace.pinnedVersion ?? workspace.latest?.version;
  const snapshot = useCommand(
    () => (version === undefined ? Promise.resolve(undefined) : api.getSnapshot(id, version)),
    [id, version],
  );
  const [restore, setRestore] = useState<RestoreRun>();
  const [dialog, setDialog] = useState<Dialog>();
  const [actionError, setActionError] = useState<CommandError>();
  const [leaveReport, setLeaveReport] = useState<CleanupReport | null>();
  /** Changes to open the capture review (from the tray or the palette). */
  const [captureKey, setCaptureKey] = useState<number>();

  const refresh = () => {
    history.reload();
    snapshot.reload();
    onChanged();
  };

  const canRestore = workspace.latest !== null && workspace.isTrusted && !restore && !workspace.isArchived;
  // Coming from another workspace, the main action is a switch (save where you are,
  // then bring this one back). In the workspace you're already in, it's a plain restore.
  const mode: "restore" | "switch" = workspace.isActive ? "restore" : "switch";
  const startRestore = (dryRun: boolean) => {
    if (canRestore) setRestore({ dryRun, mode, version, key: Date.now() });
  };

  // Keyboard: Ctrl+Enter restores, F2 renames.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector('[aria-modal="true"]')) return;
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        startRestore(false);
      } else if (e.key === "F2") {
        e.preventDefault();
        setDialog("rename");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // The tray or palette asked for a capture or a restore of this workspace.
  useEffect(() => {
    if (!pending) return;
    if (pending.kind === "report") {
      setRestore({ dryRun: false, mode: "switch", key: pending.key, report: pending.report });
    } else if (pending.kind === "capture") {
      if (workspace.isArchived) setActionError({ code: "conflict", message: "This workspace is archived; unarchive it to capture." });
      else {
        setTab("overview");
        setCaptureKey(pending.key);
      }
    } else if (workspace.latest === null) {
      setActionError({ code: "conflict", message: "Nothing has been saved in this workspace yet, so there is nothing to restore." });
    } else if (!workspace.isTrusted) {
      setActionError({ code: "conflict", message: "This workspace was imported and must be approved before it can be restored." });
    } else {
      setRestore({ dryRun: false, mode, version, key: pending.key, leaveNote: pending.note });
    }
    onPendingHandled();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending?.key]);

  const guard = (run: () => Promise<void>) => async () => {
    setActionError(undefined);
    try {
      await run();
    } catch (e) {
      setActionError(toCommandError(e));
    }
  };

  async function createShortcut(overwrite: boolean) {
    try {
      const r = await api.createShortcut(id, overwrite);
      toast.show(`Desktop icon created: ${r.path}`);
    } catch (e) {
      const err = toCommandError(e);
      if (err.code === "already_exists" && !overwrite) setDialog("replaceIcon");
      else toast.show(err.message, { tone: "bad" });
    }
  }

  const restoreLabel =
    version !== undefined && version !== (workspace.pinnedVersion ?? workspace.latest?.version)
      ? `${mode === "switch" ? "Switch to" : "Restore"} v${version}`
      : mode === "switch"
        ? "Switch to this"
        : "Restore";

  return (
    <main id="main" tabIndex={-1} className="content">
      <div className="page">
        <header className="page-head">
          <div className="title">
            <IconPicker workspace={id} name={name} />
            <div className="title-text">
              <h1>
                {name}
                {workspace.isActive && (
                  <Badge tone="ok" icon={Layers}>
                    You're here
                  </Badge>
                )}
              </h1>
              <p className="muted">
                {workspace.description ??
                  (workspace.latest
                    ? `Last saved ${relativeTime(workspace.latest.createdAt)}`
                    : "Nothing saved yet")}
                {workspace.pinnedVersion !== null && ` · Pinned to v${workspace.pinnedVersion}`}
              </p>
            </div>
          </div>
          <div className="actions">
            {workspace.isActive && (
              <button
                className="ghost"
                data-tip="Run this workspace's “when I leave” rules and step out of it"
                onClick={() => setDialog("leave")}
              >
                <LogOut size={15} aria-hidden /> Leave
              </button>
            )}
            {workspace.isActive && (
              <button
                className="ghost"
                data-tip="Put it aside for something else, and come back with Resume"
                onClick={() => setDialog("park")}
              >
                <ParkingCircle size={15} aria-hidden /> Park
              </button>
            )}
            <button
              className="ghost"
              disabled={!canRestore}
              data-tip="See what a restore would do, without doing it"
              onClick={() => startRestore(true)}
            >
              <Eye size={15} aria-hidden /> Preview
            </button>
            <button
              className="primary"
              disabled={!canRestore}
              onClick={() => startRestore(false)}
              data-tip={
                !workspace.isTrusted
                  ? "Imported workspaces must be approved first"
                  : mode === "switch"
                    ? "Save where you are, then bring this workspace back"
                    : "Reopen everything (Ctrl+Enter)"
              }
            >
              {mode === "switch" ? <ArrowRightLeft size={14} aria-hidden /> : <Play size={14} aria-hidden />} {restoreLabel} <Kbd>Ctrl ↵</Kbd>
            </button>
            <Menu label="More actions" trigger={<MoreHorizontal size={16} />}>
              {(close) => (
                <>
                  <MenuItem icon={MonitorUp} close={close} onSelect={() => void createShortcut(false)}>
                    Create desktop icon
                  </MenuItem>
                  <MenuItem icon={Pencil} hint="F2" close={close} onSelect={() => setDialog("rename")}>
                    Edit name and description…
                  </MenuItem>
                  <MenuItem
                    icon={FileDown}
                    close={close}
                    onSelect={() => setDialog("export")}
                  >
                    Export…
                  </MenuItem>
                  <hr />
                  <MenuItem
                    icon={workspace.isArchived ? ArchiveRestore : Archive}
                    close={close}
                    onSelect={guard(async () => {
                      const archiving = !workspace.isArchived;
                      await api.setWorkspaceArchived(id, archiving);
                      toast.show(archiving ? `Archived “${name}”` : `Restored “${name}” from the archive`, {
                        action: {
                          label: "Undo",
                          run: () => void api.setWorkspaceArchived(id, !archiving).then(onChanged),
                        },
                      });
                      // Archived workspaces are hidden unless the list shows them.
                      if (archiving && !showArchived) onGone();
                      else onChanged();
                    })}
                  >
                    {workspace.isArchived ? "Unarchive" : "Archive"}
                  </MenuItem>
                  <MenuItem icon={Trash2} danger close={close} onSelect={() => setDialog("delete")}>
                    Delete…
                  </MenuItem>
                </>
              )}
            </Menu>
          </div>
        </header>

        {!workspace.isTrusted && (
          <div className="banner banner-warn">
            <span>
              <Shield size={16} aria-hidden />
              This workspace was imported and hasn't been approved, so it can't be restored yet.
            </span>
            <button onClick={() => setDialog("approve")}>Review and approve…</button>
          </div>
        )}
        {workspace.isArchived && (
          <Notice tone="muted">This workspace is archived. Unarchive it from the ⋯ menu to save new snapshots.</Notice>
        )}
        {actionError && <ErrorBanner error={actionError} onDismiss={() => setActionError(undefined)} />}

        {leaveReport && leaveReport.results.length > 0 && (
          <section className="card">
            <CleanupResults report={leaveReport} />
            <button className="link" onClick={() => setLeaveReport(undefined)}>
              Dismiss
            </button>
          </section>
        )}

        {restore && (
          <RestorePanel
            key={restore.key}
            workspace={id}
            label={name}
            version={restore.version}
            dryRun={restore.dryRun}
            mode={restore.mode}
            initialReport={restore.report}
            leaveNote={restore.leaveNote}
            only={restore.only}
            onStart={() => setRestore({ dryRun: false, mode: restore.mode, version: restore.version, key: Date.now() })}
            onRetry={(ids) => setRestore({ dryRun: false, mode: "restore", version: restore.version, only: ids, key: Date.now() })}
            onClose={() => setRestore(undefined)}
            onFinished={onChanged}
          />
        )}

        <div
          className="tabs"
          role="tablist"
          aria-label="Workspace sections"
          onKeyDown={(e) => {
            const order: Tab[] = ["overview", "rules", "history"];
            const at = order.indexOf(tab);
            const next =
              e.key === "ArrowRight" ? order[(at + 1) % order.length]
              : e.key === "ArrowLeft" ? order[(at + order.length - 1) % order.length]
              : e.key === "Home" ? order[0]
              : e.key === "End" ? order[order.length - 1]
              : undefined;
            if (next) {
              e.preventDefault();
              setTab(next);
              document.getElementById(`tab-${next}`)?.focus();
            }
          }}
        >
          {(
            [
              ["overview", "Overview", null],
              ["rules", "Focus rules", null],
              ["history", "History", history.data?.length ?? null],
            ] as const
          ).map(([key, label, count]) => (
            <button
              key={key}
              role="tab"
              id={`tab-${key}`}
              aria-selected={tab === key}
              tabIndex={tab === key ? 0 : -1}
              aria-controls={`panel-${key}`}
              className="tab"
              onClick={() => setTab(key)}
            >
              {label}
              {count !== null && <span className="tab-count">{count}</span>}
            </button>
          ))}
        </div>

        <div className="tab-panel" role="tabpanel" id={`panel-${tab}`} aria-labelledby={`tab-${tab}`} key={tab}>
          {tab === "overview" && (
            <>
              {/* What you came for first: where you left off and what comes
                  back. Saving and the save setting follow. */}
              {snapshot.error && <ErrorBanner error={snapshot.error} />}
              {snapshot.loading && !snapshot.data && <SkeletonText lines={4} />}
              {snapshot.data && <BrowserNotice snapshot={snapshot.data} />}
              {snapshot.data ? (
                <Overview snapshot={snapshot.data} />
              ) : (
                !snapshot.loading && (
                  <Empty icon={Camera} title="Nothing saved yet">
                    Open the apps you work in, arrange them how you like, then choose Capture. Next time, one
                    click brings it all back.
                  </Empty>
                )
              )}
              <CaptureCard
                workspace={id}
                label={name}
                disabled={workspace.isArchived}
                openKey={captureKey}
                onCaptured={() => {
                  setSelectedVersion(undefined);
                  refresh();
                }}
              />
              {/* Only meaningful once something is saved. */}
              {snapshot.data && <SaveModeCard workspace={workspace} onChanged={onChanged} />}
              <SwitcherTip />
            </>
          )}

          {tab === "rules" && (
            <>
              <CleanupSection workspace={id} onActivity={onChanged} />
              <CommandsSection workspace={id} trusted={workspace.isTrusted} />
            </>
          )}

          {tab === "history" && (
            <div className="columns">
              <section className="card">
                <header className="card-header">
                  <h3>
                    <History size={15} aria-hidden /> Saved versions
                  </h3>
                  {(history.data?.length ?? 0) > 1 && (
                    <button
                      className="ghost small"
                      data-tip="Keep only the newest version (and a pinned one)"
                      onClick={() => setDialog("deleteOld")}
                    >
                      Delete older…
                    </button>
                  )}
                </header>
                {history.error && <ErrorBanner error={history.error} />}
                {history.loading && !history.data && <SkeletonList rows={3} />}
                {history.data?.length === 0 && <p className="muted">No snapshots yet. Capture one from Overview.</p>}
                <ol className="timeline">
                  {history.data?.map((s, i) => (
                    <HistoryRow
                      key={s.version}
                      entry={s}
                      active={s.version === version}
                      pinned={s.version === workspace.pinnedVersion}
                      older={history.data?.[i + 1]?.version}
                      onSelect={() => setSelectedVersion(s.version)}
                      onCompare={(from) => setDialog({ compare: { from, to: s.version } })}
                    />
                  ))}
                </ol>
              </section>
              <section className="card grow">
                {snapshot.error && <ErrorBanner error={snapshot.error} />}
                {snapshot.loading && !snapshot.data && <SkeletonText lines={5} />}
                {!snapshot.loading && !snapshot.data && !snapshot.error && (
                  <Empty icon={History} title="No versions yet">
                    Each capture, and each switch away from this workspace, adds a version here.
                  </Empty>
                )}
                {snapshot.data && (
                  <VersionDetails
                    snapshot={snapshot.data}
                    pinned={snapshot.data.version === workspace.pinnedVersion}
                    onPin={guard(async () => {
                      const pin = snapshot.data!.version !== workspace.pinnedVersion;
                      await api.setPinnedVersion(id, pin ? snapshot.data!.version : null);
                      refresh();
                    })}
                    onDelete={() => setDialog({ deleteVersion: snapshot.data!.version })}
                    onCompareNow={() => setDialog({ compare: { from: snapshot.data!.version } })}
                  />
                )}
              </section>
              <RecentRestores workspace={id} refreshKey={restore?.key} />
            </div>
          )}
        </div>
      </div>

      {dialog === "rename" && (
        <EditWorkspaceDialog
          workspace={workspace}
          onSaved={(u) => {
            if (u.iconError) toast.show(`Saved, but the desktop icon kept its old name: ${u.iconError}`, { tone: "bad", ms: 8000 });
            onChanged();
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "leave" && (
        <LeaveDialog
          workspace={id}
          name={name}
          onLeft={(r) => {
            if (r.savedVersion !== null) toast.show(`Saved “${name}” as v${r.savedVersion} and left it`);
            else toast.show(`Left “${name}”${r.saveNote ? `. ${r.saveNote}` : ""}`);
            const failed = r.commands.filter((c) => !c.ok);
            if (failed.length) toast.show(`A stop command didn't work: ${failed[0]!.command} (${failed[0]!.message})`, { tone: "bad", ms: 9000 });
            setLeaveReport(r.cleanup);
            history.reload();
            onChanged();
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "export" && (
        <ExportDialog
          workspaces={[id]}
          title={`Export “${name}”`}
          onDone={(r) => toast.show(`${r.message} ${r.path}`, { ms: 8000 })}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "park" && (
        <ParkDialog
          name={name}
          others={others}
          onParked={(target, note) => {
            toast.show(`Parked “${name}”. Resume brings you back.`);
            onChanged();
            if (target) onSwitchTo(target, note);
          }}
          onLeave={(note) =>
            void api.leaveWorkspace(note).then(() => {
              history.reload();
              onChanged();
            })
          }
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "approve" && (
        <TrustDialog workspace={id} name={name} onApproved={onChanged} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "delete" && (
        <DeleteWorkspaceDialog
          workspace={id}
          name={name}
          snapshotCount={history.data?.length ?? 0}
          onDeleted={(outcome) => {
            if (outcome.shortcutError) {
              toast.show(`Deleted, but its desktop icon could not be removed: ${outcome.shortcutError}`, { tone: "bad", ms: 8000 });
            } else {
              toast.show(`Deleted “${name}”`);
            }
            onGone();
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "replaceIcon" && (
        <ConfirmDialog
          title="Replace the desktop icon?"
          confirmLabel="Replace"
          action={() => createShortcut(true)}
          onClose={() => setDialog(undefined)}
        >
          <p>
            A desktop icon for <strong>{name}</strong> already exists. Replacing it keeps the same name and points it at
            this workspace.
          </p>
        </ConfirmDialog>
      )}
      {typeof dialog === "object" && "compare" in dialog && (
        <CompareDialog
          workspace={id}
          from={dialog.compare.from}
          to={dialog.compare.to}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "deleteOld" && (
        <ConfirmDialog
          title="Delete older versions?"
          confirmLabel="Delete older versions"
          danger
          action={async () => {
            const deleted = await api.deleteOldSnapshots(id);
            setSelectedVersion(undefined);
            refresh();
            toast.show(deleted.length ? `Deleted ${deleted.length} older version(s) of “${name}”` : "Nothing older to delete");
          }}
          onClose={() => setDialog(undefined)}
        >
          <p>
            This permanently deletes every saved version of <strong>{name}</strong> except the newest
            {workspace.pinnedVersion ? ` and the pinned version (v${workspace.pinnedVersion})` : ""}. Version numbers are
            not reused. Use it to clear out test saves.
          </p>
        </ConfirmDialog>
      )}
      {typeof dialog === "object" && "deleteVersion" in dialog && (
        <ConfirmDialog
          title={`Delete version ${dialog.deleteVersion}?`}
          confirmLabel="Delete version"
          danger
          action={async () => {
            await api.deleteSnapshot(id, dialog.deleteVersion);
            setSelectedVersion(undefined);
            refresh();
          }}
          onClose={() => setDialog(undefined)}
        >
          <p>
            This permanently deletes version {dialog.deleteVersion} of <strong>{name}</strong>. Other versions are
            kept, and the number is not reused.
          </p>
        </ConfirmDialog>
      )}
    </main>
  );
}

function HistoryRow({
  entry,
  active,
  pinned,
  older,
  onSelect,
  onCompare,
}: {
  entry: SnapshotEntry;
  active: boolean;
  pinned: boolean;
  older: number | undefined;
  onSelect: () => void;
  onCompare: (from: number) => void;
}) {
  return (
    <li className="timeline-row">
      <button
        className={`timeline-item${active ? " active" : ""}`}
        onClick={onSelect}
        title={`Version ${entry.version} · ${absoluteTime(entry.createdAt)} · ${entry.shortHash}`}
      >
        <span className={`status-dot ${captureStatusTone[entry.captureStatus]}`} role="img" aria-label={statusLabel[entry.captureStatus]} />
        <span className="timeline-main">
          <span className="timeline-when">{relativeTime(entry.createdAt)}</span>
          <span className="timeline-sub">
            Version {entry.version}
            {entry.captureStatus !== "complete" && ` · ${statusLabel[entry.captureStatus].toLowerCase()}`}
          </span>
        </span>
        {entry.auto && <span className="tag">Auto</span>}
        {pinned && <Pin size={13} aria-label="Pinned" className="pin-mark" />}
      </button>
      {older !== undefined && (
        <button
          className="ghost icon"
          aria-label={`Compare v${entry.version} with v${older}`}
          data-tip={`What changed since v${older}?`}
          onClick={() => onCompare(older)}
        >
          <GitCompareArrows size={15} />
        </button>
      )}
    </li>
  );
}

/** "Where you left off": the first thing you see when you open a workspace. */
function Overview({ snapshot }: { snapshot: SnapshotDetail }) {
  const windows = snapshot.applications.reduce((n, a) => n + a.windowCount, 0);
  const note = snapshot.notes.at(-1);
  const repo = snapshot.repositories[0];
  const dirty = repo ? repo.staged + repo.modified + repo.untracked : 0;

  return (
    <>
      <section className="resume-card" aria-label="Where you left off">
        <span className="eyebrow">Where you left off</span>
        <p className="resume-note">{note ?? "No note on this save. Add one when you capture to remember what you were doing."}</p>
        <div className="resume-meta">
          <span title={absoluteTime(snapshot.createdAt)}>
            <Clock size={13} aria-hidden /> Saved {relativeTime(snapshot.createdAt)}
          </span>
          {repo && (
            <span>
              <GitBranch size={13} aria-hidden /> {repo.branch ?? "detached"}
              {dirty > 0 ? ` · ${plural(dirty, "changed file")}` : " · clean"}
            </span>
          )}
          {snapshot.captureStatus !== "complete" && (
            <Badge tone={captureStatusTone[snapshot.captureStatus]}>{statusLabel[snapshot.captureStatus]} save</Badge>
          )}
        </div>
      </section>

      <div className="stat-row">
        <div className="stat">
          <div className="stat-value">{snapshot.applications.length}</div>
          <div className="stat-label">{snapshot.applications.length === 1 ? "App" : "Apps"}</div>
        </div>
        <div className="stat">
          <div className="stat-value">{windows}</div>
          <div className="stat-label">{windows === 1 ? "Window" : "Windows"}</div>
        </div>
        <div className="stat">
          <div className="stat-value">{snapshot.monitorCount}</div>
          <div className="stat-label">{snapshot.monitorCount === 1 ? "Monitor" : "Monitors"}</div>
        </div>
      </div>

      <section className="card">
        <header className="card-header">
          <h3>What comes back</h3>
        </header>
        <ul className="app-list">
          {snapshot.applications.map((a) => (
            <li className="app-row" key={a.applicationId}>
              <AppIcon applicationId={a.applicationId} />
              <span className="app-name" title={a.applicationId}>{friendlyAppName(a.applicationId)}</span>
              <span className="muted small">{appSummary(a)}</span>
            </li>
          ))}
        </ul>
        {snapshot.repositories.length > 0 && (
          <>
            <h4>Repositories</h4>
            <ul className="repos">
              {snapshot.repositories.map((r) => (
                <li key={r.path}>
                  <span className="mono">{r.path}</span> <Badge tone="muted">{r.branch ?? "detached"}</Badge>{" "}
                  <span className="muted">
                    {r.staged} staged, {r.modified} modified, {r.untracked} untracked
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
      </section>
    </>
  );
}

/** "2 windows · 7 tabs · 2 folders": what comes back of one application. */
function appSummary(a: SnapshotDetail["applications"][number]): string {
  return [
    plural(a.windowCount, "window"),
    a.tabCount > 0 && plural(a.tabCount, "tab"),
    a.folderCount > 0 && plural(a.folderCount, "folder"),
  ]
    .filter(Boolean)
    .join(" · ");
}

function VersionDetails({
  snapshot,
  pinned,
  onPin,
  onDelete,
  onCompareNow,
}: {
  snapshot: SnapshotDetail;
  pinned: boolean;
  onPin: () => void;
  onDelete: () => void;
  onCompareNow: () => void;
}) {
  const windows = snapshot.applications.reduce((n, a) => n + a.windowCount, 0);
  return (
    <>
      <header className="card-header">
        <h3 className="nowrap">
          Version {snapshot.version}
          <Badge tone={captureStatusTone[snapshot.captureStatus]}>{statusLabel[snapshot.captureStatus]}</Badge>
        </h3>
        <div className="actions">
          <button className="ghost" onClick={onCompareNow} data-tip="What changed on your screen since this was saved?">
            <GitCompareArrows size={15} aria-hidden /> Compare with now
          </button>
          <button
            className="ghost"
            onClick={onPin}
            data-tip={pinned ? "Restore the newest save again" : "Always restore this version, even after new saves"}
          >
            {pinned ? <PinOff size={15} aria-hidden /> : <Pin size={15} aria-hidden />} {pinned ? "Unpin" : "Pin"}
          </button>
          <button
            className="ghost danger"
            onClick={onDelete}
            disabled={pinned}
            data-tip={pinned ? "Unpin it before deleting it" : "Permanently delete this version"}
          >
            <Trash2 size={15} aria-hidden /> Delete
          </button>
        </div>
      </header>
      <p className="muted">
        Saved {absoluteTime(snapshot.createdAt)} · {plural(snapshot.applications.length, "app")},{" "}
        {plural(windows, "window")}, {plural(snapshot.monitorCount, "monitor")}
      </p>
      {snapshot.notes.length > 0 && <p className="gap-top">“{snapshot.notes.at(-1)}”</p>}
      <ul className="app-list gap-top">
        {snapshot.applications.map((a) => (
          <li className="app-row" key={a.applicationId}>
            <AppIcon applicationId={a.applicationId} />
            <span className="app-name" title={a.applicationId}>{friendlyAppName(a.applicationId)}</span>
            <span className="muted small">{appSummary(a)}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

function CaptureCard({
  workspace,
  label,
  disabled,
  openKey,
  onCaptured,
}: {
  workspace: string;
  label: string;
  disabled: boolean;
  /** Changes to open the review (the tray's "Capture current…"). */
  openKey?: number;
  onCaptured: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [report, setReport] = useState<CaptureReport>();
  const hotkey = useCommand(() => api.getSettings(), []).data?.app.captureHotkey;

  useEffect(() => {
    if (openKey !== undefined) setOpen(true);
  }, [openKey]);

  return (
    <section className="card">
      <div className="capture">
        <div>
          <h3>Save what's open now</h3>
          <p className="muted small">
            {hotkey ? (
              <>
                You choose what goes in. Or press <Kbd>{hotkey}</Kbd> anywhere to save instantly.
              </>
            ) : (
              "You choose what goes in."
            )}
          </p>
        </div>
        <button className="primary" disabled={disabled} onClick={() => setOpen(true)}>
          <Camera size={15} aria-hidden /> Capture…
        </button>
      </div>
      {report && (
        <div className="gap-top">
          <Notice onDismiss={() => setReport(undefined)}>
            Saved v{report.snapshot.version}
            {report.identicalToPrevious && " (identical to the previous version)"}.
            {report.pinnedVersion !== null && ` Restore still uses pinned v${report.pinnedVersion}.`}
            {report.autoPruned.length > 0 && ` Removed ${plural(report.autoPruned.length, "old snapshot")}.`}
          </Notice>
          {report.warnings.length > 0 && (
            <ul className="warnings">
              {report.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      {open && (
        <CaptureReview
          workspace={workspace}
          label={label}
          onSaved={(r) => {
            setReport(r);
            onCaptured();
          }}
          onClose={() => setOpen(false)}
        />
      )}
    </section>
  );
}

/** Leave the workspace you're in: it is saved first, then its leave rules run. */
function LeaveDialog({
  workspace,
  name,
  onLeft,
  onClose,
}: {
  workspace: string;
  name: string;
  onLeft: (r: import("../ipc/client").LeaveReport) => void;
  onClose: () => void;
}) {
  const plan = useCommand(() => api.previewCleanup(workspace, "leaving"), [workspace]);
  const [note, setNote] = useState("");
  const closing = plan.data?.items.filter((i) => i.action.kind === "close") ?? [];
  return (
    <ConfirmDialog
      title={`Leave “${name}”?`}
      confirmLabel={closing.length ? `Leave and close ${closing.length === 1 ? "1 program" : `${closing.length} programs`}` : "Leave"}
      danger={closing.some((i) => i.mode === "force")}
      action={async () => onLeft(await api.leaveWorkspace(note))}
      onClose={onClose}
    >
      <p>Its layout is saved first (unless nothing has changed), so you can come back to it later.</p>
      <label className="field">
        <span className="field-label">Note for when you come back (optional)</span>
        <input
          className="wide"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="What were you doing?"
          maxLength={500}
        />
      </label>
      {plan.loading && !plan.data && <SkeletonText lines={2} />}
      {closing.length > 0 && plan.data && (
        <>
          <p className="muted small">Its “when I leave” rules will close:</p>
          <ul className="cleanup-confirm">
            {closing.map((i) => (
              <li key={i.process}>
                {friendlyAppName(i.process)} · {i.mode === "force" ? "force close" : "close nicely"} ·{" "}
                <PlanItemStatus item={i} />
              </li>
            ))}
          </ul>
          <p className="small">
            <PlanSummary plan={plan.data} />
          </p>
        </>
      )}
      {plan.data && closing.length === 0 && <p className="muted small">Nothing will be closed.</p>}
    </ConfirmDialog>
  );
}

/** The last few restores of this workspace and what went wrong in them. */
function RecentRestores({ workspace, refreshKey }: { workspace: string; refreshKey?: number }) {
  const runs = useCommand(() => api.listRestoreRuns(workspace), [workspace, refreshKey]);
  return (
    <section className="card span-2">
      <header className="card-header">
        <h3>
          <Clock size={15} aria-hidden /> Recent restores
        </h3>
      </header>
      {runs.error && <ErrorBanner error={runs.error} />}
      {runs.loading && !runs.data && <SkeletonList rows={2} />}
      {runs.data?.length === 0 && <p className="muted">This workspace hasn't been restored yet.</p>}
      <ul className="app-list">
        {runs.data?.map((r) => (
          <li className="app-row run-row" key={r.startedAt}>
            <Badge tone={restoreStatusTone[r.overall]}>{restoreStatusLabel[r.overall]}</Badge>
            <span className="app-name" title={absoluteTime(r.startedAt)}>
              {relativeTime(r.startedAt)} · v{r.version}
            </span>
            <span className="muted small">
              {r.problems.length === 0
                ? plural(r.steps, "program")
                : r.problems
                    .map((p) => `${friendlyAppName(p.applicationId)}${p.message ? `: ${p.message}` : ""}`)
                    .join(" · ")}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Workspaces with browsers only get their tabs back once the extension is connected. */
function BrowserNotice({ snapshot }: { snapshot: SnapshotDetail }) {
  const hasBrowser = snapshot.applications.some((a) => /^(chrome|msedge)$/i.test(a.applicationId));
  const status = useCommand(
    () => (hasBrowser ? api.getBrowserStatus() : Promise.resolve(undefined)),
    [hasBrowser],
  );
  const connected = status.data?.browsers.some((b) => b.connected);
  if (!hasBrowser || !status.data || connected) return null;
  return (
    <Notice tone="warn">
      Browser tabs aren't being saved yet: the ContextFlow browser extension isn't connected. Set it up in
      Settings → Capture → Browser tabs. Windows still come back without it.
    </Notice>
  );
}

const TIP_KEY = "contextflow.tip.switcher";

/** A one-time pointer to the switcher, the fastest way to use ContextFlow. */
function SwitcherTip() {
  const [hidden, setHidden] = useState(() => localStorage.getItem(TIP_KEY) === "1");
  const hotkey = useCommand(() => api.getSettings(), []).data?.app.switcherHotkey;
  if (hidden || !hotkey) return null;
  return (
    <Notice
      tone="muted"
      onDismiss={() => {
        localStorage.setItem(TIP_KEY, "1");
        setHidden(true);
      }}
    >
      Tip: press <Kbd>{hotkey}</Kbd> anywhere in Windows to switch workspaces without opening this window.
    </Notice>
  );
}

const SAVE_MODES: Array<{ mode: SaveMode; title: string; hint: string }> = [
  {
    mode: "notify",
    title: "Save my changes and tell me",
    hint: "For work in progress: your layout is saved as you leave, and a notification says which version.",
  },
  {
    mode: "auto",
    title: "Save my changes quietly",
    hint: "The same, without the notification.",
  },
  {
    mode: "fixed",
    title: "Keep it as I captured it",
    hint: "One and done: never saved automatically, so it always comes back exactly the same. Capture again to change it.",
  },
];

/** What happens to this workspace when you switch away from it or leave it. */
function SaveModeCard({ workspace, onChanged }: { workspace: WorkspaceSummary; onChanged: () => void }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const choose = async (mode: SaveMode) => {
    if (mode === workspace.saveMode) return;
    setBusy(true);
    try {
      await api.setSaveMode(workspace.id, mode);
      onChanged();
    } catch (e) {
      toast.show(toCommandError(e).message, { tone: "bad" });
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="card">
      <header className="card-header">
        <h3>When you leave this workspace</h3>
      </header>
      <div className="choice-list" role="radiogroup" aria-label="When you leave this workspace">
        {SAVE_MODES.map((o) => (
          <label key={o.mode} className={`choice${workspace.saveMode === o.mode ? " selected" : ""}`}>
            <input
              type="radio"
              name={`save-mode-${workspace.id}`}
              checked={workspace.saveMode === o.mode}
              disabled={busy}
              onChange={() => void choose(o.mode)}
            />
            <span>
              <span className="choice-title">{o.title}</span>
              <span className="choice-hint">{o.hint}</span>
            </span>
          </label>
        ))}
      </div>
    </section>
  );
}

/** Park the workspace you're in: note where you were, then go somewhere else. */
function ParkDialog({
  name,
  others,
  onParked,
  onLeave,
  onClose,
}: {
  name: string;
  others: WorkspaceSummary[];
  onParked: (target: string | undefined, note: string) => void;
  onLeave: (note: string) => void;
  onClose: () => void;
}) {
  const [note, setNote] = useState("");
  const [target, setTarget] = useState<string>("");
  return (
    <ConfirmDialog
      title={`Park “${name}”?`}
      confirmLabel={target ? "Park and switch" : "Park and leave"}
      action={async () => {
        await api.parkCurrent(note);
        if (target) onParked(target, note);
        else {
          onParked(undefined, note);
          onLeave(note);
        }
      }}
      onClose={onClose}
    >
      <p>
        It's saved as it is now and put aside. Resume (in the sidebar, the tray or Ctrl+K) brings you straight back,
        even after several interruptions.
      </p>
      <label className="field">
        <span className="field-label">Where you were (optional)</span>
        <input
          className="wide"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="e.g. halfway through the migration test"
          maxLength={500}
        />
      </label>
      <label className="field">
        <span className="field-label">Then</span>
        <select value={target} onChange={(e) => setTarget(e.target.value)}>
          <option value="">Just leave it (no other workspace)</option>
          {others.map((w) => (
            <option key={w.id} value={w.id}>
              Switch to {w.name}
            </option>
          ))}
        </select>
      </label>
    </ConfirmDialog>
  );
}
