import {
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
import { ConfirmDialog, DeleteWorkspaceDialog, RenameDialog, TrustDialog } from "../components/dialogs";
import { CleanupResults, CleanupSection } from "../components/CleanupSection";
import { IconPicker } from "../components/IconPicker";
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
import { absoluteTime, plural, relativeTime } from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type CaptureReport,
  type CleanupReport,
  type CommandError,
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
};

type Tab = "overview" | "rules" | "history";

type Dialog =
  | "rename"
  | "delete"
  | "approve"
  | "replaceIcon"
  | { deleteVersion: number }
  | { compare: { from: number; to?: number } };

type RestoreRun = { dryRun: boolean; version?: number; only?: string[]; key: number };

const statusTone = { complete: "ok", partial: "warn", failed: "bad" } as const;
const statusLabel = { complete: "Complete", partial: "Partial", failed: "Failed" } as const;

export function WorkspaceView({ workspace, onChanged, onGone, showArchived, pending, onPendingHandled }: Props) {
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
  const startRestore = (dryRun: boolean) => {
    if (canRestore) setRestore({ dryRun, version, key: Date.now() });
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
    if (pending.kind === "capture") {
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
      setRestore({ dryRun: false, version, key: pending.key });
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
      ? `Restore v${version}`
      : "Restore";

  return (
    <main className="content">
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
                {workspace.pinnedVersion !== null && ` · Locked to v${workspace.pinnedVersion}`}
              </p>
            </div>
          </div>
          <div className="actions">
            {workspace.isActive && (
              <button
                className="ghost"
                data-tip="Run this workspace's “when I leave” rules and step out of it"
                onClick={guard(async () => {
                  setLeaveReport(await api.leaveWorkspace());
                  onChanged();
                })}
              >
                <LogOut size={15} aria-hidden /> Leave
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
              data-tip={workspace.isTrusted ? "Reopen everything (Ctrl+Enter)" : "Imported workspaces must be approved first"}
            >
              <Play size={14} aria-hidden /> {restoreLabel} <Kbd>Ctrl ↵</Kbd>
            </button>
            <Menu label="More actions" trigger={<MoreHorizontal size={16} />}>
              {(close) => (
                <>
                  <MenuItem icon={MonitorUp} close={close} onSelect={() => void createShortcut(false)}>
                    Create desktop icon
                  </MenuItem>
                  <MenuItem icon={Pencil} hint="F2" close={close} onSelect={() => setDialog("rename")}>
                    Rename…
                  </MenuItem>
                  <MenuItem
                    icon={FileDown}
                    close={close}
                    onSelect={guard(async () => {
                      const r = await api.exportWorkspaces([id]);
                      if (r) toast.show(`${r.message} ${r.path}`);
                    })}
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
            only={restore.only}
            onStart={() => setRestore({ dryRun: false, version: restore.version, key: Date.now() })}
            onRetry={(ids) => setRestore({ dryRun: false, version: restore.version, only: ids, key: Date.now() })}
            onClose={() => setRestore(undefined)}
            onFinished={onChanged}
          />
        )}

        <div className="tabs" role="tablist" aria-label="Workspace sections">
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
              {snapshot.error && <ErrorBanner error={snapshot.error} />}
              {snapshot.loading && !snapshot.data && <SkeletonText lines={4} />}
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
            </>
          )}

          {tab === "rules" && <CleanupSection workspace={id} onActivity={onChanged} />}

          {tab === "history" && (
            <div className="columns">
              <section className="card">
                <header className="card-header">
                  <h3>
                    <History size={15} aria-hidden /> Saved versions
                  </h3>
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
            </div>
          )}
        </div>
      </div>

      {dialog === "rename" && (
        <RenameDialog
          workspace={workspace}
          onRenamed={() => onChanged()}
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
        <span className={`status-dot ${statusTone[entry.captureStatus]}`} role="img" aria-label={statusLabel[entry.captureStatus]} />
        <span className="timeline-main">
          <span className="timeline-when">{relativeTime(entry.createdAt)}</span>
          <span className="timeline-sub">
            Version {entry.version}
            {entry.captureStatus !== "complete" && ` · ${statusLabel[entry.captureStatus].toLowerCase()}`}
          </span>
        </span>
        {pinned && <Pin size={13} aria-label="Locked" style={{ marginLeft: "auto", color: "var(--accent)" }} />}
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
            <Badge tone={statusTone[snapshot.captureStatus]}>{statusLabel[snapshot.captureStatus]} save</Badge>
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
              <span className="app-name">{a.applicationId}</span>
              <span className="muted small">
                {[
                  plural(a.windowCount, "window"),
                  a.tabCount > 0 && plural(a.tabCount, "tab"),
                  a.folderCount > 0 && plural(a.folderCount, "folder"),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
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
        <h3>
          Version {snapshot.version}
          <Badge tone={statusTone[snapshot.captureStatus]}>{statusLabel[snapshot.captureStatus]}</Badge>
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
            {pinned ? <PinOff size={15} aria-hidden /> : <Pin size={15} aria-hidden />} {pinned ? "Unlock" : "Lock"}
          </button>
          <button
            className="ghost danger"
            onClick={onDelete}
            disabled={pinned}
            data-tip={pinned ? "Unlock it before deleting it" : "Permanently delete this version"}
          >
            <Trash2 size={15} aria-hidden /> Delete
          </button>
        </div>
      </header>
      <p className="muted">
        Saved {absoluteTime(snapshot.createdAt)} · {plural(snapshot.applications.length, "app")},{" "}
        {plural(windows, "window")}, {plural(snapshot.monitorCount, "monitor")}
      </p>
      {snapshot.notes.length > 0 && <p style={{ marginTop: 12 }}>“{snapshot.notes.at(-1)}”</p>}
      <ul className="app-list" style={{ marginTop: 12 }}>
        {snapshot.applications.map((a) => (
          <li className="app-row" key={a.applicationId}>
            <AppIcon applicationId={a.applicationId} />
            <span className="app-name">{a.applicationId}</span>
            <span className="muted small">{plural(a.windowCount, "window")}</span>
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
        <div style={{ marginTop: 12 }}>
          <Notice onDismiss={() => setReport(undefined)}>
            Saved v{report.snapshot.version}
            {report.identicalToPrevious && " (identical to the previous version)"}.
            {report.pinnedVersion !== null && ` Restore still uses locked v${report.pinnedVersion}.`}
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
