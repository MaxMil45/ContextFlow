import { useEffect, useRef, useState } from "react";

import type { Pending } from "../App";
import { CaptureReview } from "../components/CaptureReview";
import { CompareDialog } from "../components/CompareDialog";
import { ConfirmDialog, DeleteWorkspaceDialog, RenameDialog, TrustDialog } from "../components/dialogs";
import { CleanupResults, CleanupSection } from "../components/CleanupSection";
import { IconPicker } from "../components/IconPicker";
import { Badge, Empty, ErrorBanner, Spinner } from "../components/ui";
import { absoluteTime, captureStatusTone, plural, relativeTime } from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type CaptureReport,
  type CleanupReport,
  type CommandError,
  type SnapshotDetail,
  type WorkspaceSummary,
} from "../ipc/client";
import { RestorePanel } from "./RestorePanel";

type Props = {
  workspace: WorkspaceSummary;
  onChanged: () => void;
  onRenamed: (renamed: WorkspaceSummary) => void;
  /** The workspace was deleted, or archived while archived ones are hidden. */
  onGone: () => void;
  showArchived: boolean;
  /** Something the tray asked for (capture, restore), done once this page shows. */
  pending?: Pending;
  onPendingHandled: () => void;
};

type Dialog =
  | "rename"
  | "delete"
  | "approve"
  | { deleteVersion: number }
  | { compare: { from: number; to?: number } };

type RestoreRun = { dryRun: boolean; version?: number; only?: string[]; key: number };

export function WorkspaceView({
  workspace,
  onChanged,
  onRenamed,
  onGone,
  showArchived,
  pending,
  onPendingHandled,
}: Props) {
  const name = workspace.name;
  const history = useCommand(() => api.listSnapshots(name), [name]);
  const [selectedVersion, setSelectedVersion] = useState<number>();
  const version = selectedVersion ?? workspace.pinnedVersion ?? workspace.latest?.version;
  const snapshot = useCommand(
    () => (version === undefined ? Promise.resolve(undefined) : api.getSnapshot(name, version)),
    [name, version],
  );
  const [restore, setRestore] = useState<RestoreRun>();
  const [dialog, setDialog] = useState<Dialog>();
  const [actionError, setActionError] = useState<CommandError>();
  const [leaveReport, setLeaveReport] = useState<CleanupReport | null>();
  const [compareSel, setCompareSel] = useState<number[]>([]);
  const [notice, setNotice] = useState<string>();
  /** Changes to open the capture review (from the tray). */
  const [captureKey, setCaptureKey] = useState<number>();

  const refresh = () => {
    history.reload();
    snapshot.reload();
    onChanged();
  };

  const canRestore = workspace.latest !== null && workspace.isTrusted && !restore;

  // The tray asked for a capture or a restore of this workspace.
  useEffect(() => {
    if (!pending) return;
    if (pending.kind === "capture") {
      if (workspace.isArchived) setActionError({ code: "conflict", message: "This workspace is archived; unarchive it to capture." });
      else setCaptureKey(pending.key);
    } else if (workspace.latest === null) {
      setActionError({ code: "conflict", message: "This workspace has no snapshot to restore yet." });
    } else if (!workspace.isTrusted) {
      setActionError({ code: "conflict", message: "This workspace was imported and must be approved before it can be restored." });
    } else {
      setRestore({ dryRun: false, version, key: pending.key });
    }
    onPendingHandled();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending?.key]);

  return (
    <main className="content">
      <header className="content-header">
        <div className="title">
          <IconPicker workspace={name} />
          <div>
          <h1>
            {name} {workspace.isActive && <Badge tone="ok">active</Badge>}
          </h1>
          {workspace.description && <p className="muted">{workspace.description}</p>}
          {workspace.pinnedVersion !== null && (
            <p className="muted">Restore uses pinned v{workspace.pinnedVersion}.</p>
          )}
          </div>
        </div>
        <div className="actions">
          {workspace.isActive && (
            <button
              title="Run this workspace's 'when I leave' cleanup and stop being in it"
              onClick={async () => {
                setActionError(undefined);
                try {
                  setLeaveReport(await api.leaveWorkspace());
                  onChanged();
                } catch (e) {
                  setActionError(toCommandError(e));
                }
              }}
            >
              Leave workspace
            </button>
          )}
          <ShortcutButton workspace={name} disabled={workspace.latest === null} />
          <WorkspaceMenu
            archived={workspace.isArchived}
            onRename={() => setDialog("rename")}
            onDelete={() => setDialog("delete")}
            onExport={async () => {
              setActionError(undefined);
              try {
                const r = await api.exportWorkspaces([name]);
                if (r) setNotice(`${r.message} ${r.path}`);
              } catch (e) {
                setActionError(toCommandError(e));
              }
            }}
            onArchive={async () => {
              setActionError(undefined);
              try {
                await api.setWorkspaceArchived(name, !workspace.isArchived);
                // Archived workspaces are hidden unless "Show archived" is on.
                if (!workspace.isArchived && !showArchived) onGone();
                else onChanged();
              } catch (e) {
                setActionError(toCommandError(e));
              }
            }}
          />
          <button
            disabled={!canRestore}
            onClick={() => setRestore({ dryRun: true, version, key: Date.now() })}
          >
            Preview restore
          </button>
          <button
            className="primary"
            disabled={!canRestore}
            onClick={() => setRestore({ dryRun: false, version, key: Date.now() })}
            title={workspace.isTrusted ? undefined : "Imported workspaces must be approved first"}
          >
            Restore{version !== undefined ? ` v${version}` : ""}
          </button>
        </div>
      </header>

      {!workspace.isTrusted && (
        <div className="banner banner-warn">
          <span>This workspace was imported and has not been approved, so it can't be restored yet.</span>
          <button onClick={() => setDialog("approve")}>Review and approve…</button>
        </div>
      )}

      {workspace.isArchived && (
        <div className="banner banner-muted">
          This workspace is archived. Unarchive it from the ⋯ menu to capture new snapshots.
        </div>
      )}

      {leaveReport && leaveReport.results.length > 0 && (
        <section className="card">
          <CleanupResults report={leaveReport} />
          <button className="link" onClick={() => setLeaveReport(undefined)}>
            Dismiss
          </button>
        </section>
      )}

      {actionError && <ErrorBanner error={actionError} onDismiss={() => setActionError(undefined)} />}
      {notice && (
        <div className="banner banner-ok">
          <span>{notice}</span>
          <button className="link" onClick={() => setNotice(undefined)}>
            Dismiss
          </button>
        </div>
      )}

      {restore && (
        <RestorePanel
          key={restore.key}
          workspace={name}
          version={restore.version}
          dryRun={restore.dryRun}
          only={restore.only}
          onStart={() => setRestore({ dryRun: false, version: restore.version, key: Date.now() })}
          onRetry={(ids) => setRestore({ dryRun: false, version: restore.version, only: ids, key: Date.now() })}
          onClose={() => setRestore(undefined)}
          onFinished={onChanged}
        />
      )}

      <CaptureCard
        workspace={name}
        disabled={workspace.isArchived}
        openKey={captureKey}
        onCaptured={() => {
          setSelectedVersion(undefined);
          refresh();
        }}
      />

      <CleanupSection workspace={name} onActivity={onChanged} />

      <div className="columns">
        <section className="card">
          <header className="card-header">
            <h3>History</h3>
            {compareSel.length === 2 ? (
              <button
                onClick={() => {
                  const [from, to] = [...compareSel].sort((a, b) => a - b);
                  setDialog({ compare: { from: from!, to } });
                }}
              >
                Compare selected
              </button>
            ) : (
              (history.data?.length ?? 0) > 1 && <span className="muted small">Tick two to compare</span>
            )}
          </header>
          {history.error && <ErrorBanner error={history.error} />}
          {history.loading && !history.data && <Spinner />}
          {history.data?.length === 0 && <p className="muted">No snapshots yet. Capture one above.</p>}
          <ol className="timeline">
            {history.data?.map((s) => (
              <li key={s.version} className="timeline-row">
                <input
                  type="checkbox"
                  aria-label={`Select v${s.version} to compare`}
                  checked={compareSel.includes(s.version)}
                  onChange={() =>
                    setCompareSel((sel) =>
                      sel.includes(s.version)
                        ? sel.filter((v) => v !== s.version)
                        : [...sel, s.version].slice(-2),
                    )
                  }
                />
                <button
                  className={`timeline-item${s.version === version ? " active" : ""}`}
                  onClick={() => setSelectedVersion(s.version)}
                >
                  <Badge tone={captureStatusTone[s.captureStatus]}>v{s.version}</Badge>
                  <span title={absoluteTime(s.createdAt)}>{relativeTime(s.createdAt)}</span>
                  {s.version === workspace.pinnedVersion && <Badge tone="muted">pinned</Badge>}
                  <span className="mono muted">{s.shortHash}</span>
                </button>
              </li>
            ))}
          </ol>
        </section>

        <section className="card grow">
          {snapshot.error && <ErrorBanner error={snapshot.error} />}
          {snapshot.data ? (
            <SnapshotDetails
              snapshot={snapshot.data}
              pinned={snapshot.data.version === workspace.pinnedVersion}
              onPin={async (pin) => {
                setActionError(undefined);
                try {
                  await api.setPinnedVersion(name, pin ? snapshot.data!.version : null);
                  refresh();
                } catch (e) {
                  setActionError(toCommandError(e));
                }
              }}
              onDelete={() => setDialog({ deleteVersion: snapshot.data!.version })}
              onCompareNow={() => setDialog({ compare: { from: snapshot.data!.version } })}
            />
          ) : (
            !snapshot.loading && <Empty title="Nothing captured yet">Open your apps, then capture.</Empty>
          )}
        </section>
      </div>

      {dialog === "rename" && (
        <RenameDialog workspace={workspace} onRenamed={onRenamed} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "approve" && (
        <TrustDialog workspace={name} onApproved={onChanged} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "delete" && (
        <DeleteWorkspaceDialog
          workspace={name}
          snapshotCount={history.data?.length ?? 0}
          onDeleted={(outcome) => {
            // The workspace is gone even if its icon couldn't be removed; say so on the next screen.
            if (outcome.shortcutError) window.alert(`The workspace was deleted, but its desktop icon could not be removed: ${outcome.shortcutError}`);
            onGone();
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {typeof dialog === "object" && "compare" in dialog && (
        <CompareDialog
          workspace={name}
          from={dialog.compare.from}
          to={dialog.compare.to}
          onClose={() => setDialog(undefined)}
        />
      )}
      {typeof dialog === "object" && "deleteVersion" in dialog && (
        <ConfirmDialog
          title={`Delete v${dialog.deleteVersion}?`}
          confirmLabel="Delete version"
          danger
          action={async () => {
            await api.deleteSnapshot(name, dialog.deleteVersion);
            setSelectedVersion(undefined);
            refresh();
          }}
          onClose={() => setDialog(undefined)}
        >
          <p>
            This permanently deletes version {dialog.deleteVersion} of <strong>{name}</strong>. Other
            versions are kept, and the number is not reused.
          </p>
        </ConfirmDialog>
      )}
    </main>
  );
}

/** The ⋯ menu: rename, archive, delete. */
function WorkspaceMenu({
  archived,
  onRename,
  onArchive,
  onDelete,
  onExport,
}: {
  archived: boolean;
  onRename: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onExport: () => void;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const item = (label: string, run: () => void, danger = false) => (
    <button
      role="menuitem"
      className={danger ? "danger" : undefined}
      onClick={() => {
        setOpen(false);
        run();
      }}
    >
      {label}
    </button>
  );

  return (
    <div className="shortcut" ref={root}>
      <button aria-haspopup="menu" aria-expanded={open} aria-label="More actions" onClick={() => setOpen((o) => !o)}>
        ⋯
      </button>
      {open && (
        <div className="menu menu-right" role="menu">
          {item("Rename…", onRename)}
          {item("Export…", onExport)}
          {item(archived ? "Unarchive" : "Archive", onArchive)}
          {item("Delete…", onDelete, true)}
        </div>
      )}
    </div>
  );
}

function SnapshotDetails({
  snapshot,
  pinned,
  onPin,
  onDelete,
  onCompareNow,
}: {
  snapshot: SnapshotDetail;
  pinned: boolean;
  onPin: (pin: boolean) => void;
  onDelete: () => void;
  onCompareNow: () => void;
}) {
  const windows = snapshot.applications.reduce((n, a) => n + a.windowCount, 0);
  return (
    <>
      <header className="card-header">
        <h3>
          v{snapshot.version} · {absoluteTime(snapshot.createdAt)}
        </h3>
        <div className="actions">
          <Badge tone={captureStatusTone[snapshot.captureStatus]}>{snapshot.captureStatus}</Badge>
          <button onClick={onCompareNow} title="What has changed on your screen since this was saved?">
            Compare with now
          </button>
          <button
            onClick={() => onPin(!pinned)}
            title={
              pinned
                ? "Restore the latest snapshot again"
                : "Make restore and the desktop icon always use this version"
            }
          >
            {pinned ? "Unpin" : "Pin this version"}
          </button>
          <button
            className="danger"
            onClick={onDelete}
            disabled={pinned}
            title={pinned ? "Unpin it before deleting it" : "Permanently delete this version"}
          >
            Delete…
          </button>
        </div>
      </header>
      <p className="muted">
        {plural(snapshot.applications.length, "application")}, {plural(windows, "window")},{" "}
        {plural(snapshot.monitorCount, "monitor")}
      </p>
      {snapshot.notes.length > 0 && (
        <blockquote className="resume-card">{snapshot.notes.at(-1)}</blockquote>
      )}
      <table className="apps">
        <tbody>
          {snapshot.applications.map((a) => (
            <tr key={a.applicationId}>
              <td className="mono">{a.applicationId}</td>
              <td>{plural(a.windowCount, "window")}</td>
              <td className="muted">
                {[
                  a.tabCount > 0 && plural(a.tabCount, "tab"),
                  a.folderCount > 0 && plural(a.folderCount, "folder"),
                ]
                  .filter(Boolean)
                  .join(", ")}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
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
    </>
  );
}

function CaptureCard({
  workspace,
  disabled,
  openKey,
  onCaptured,
}: {
  workspace: string;
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
        <span className="muted">
          {hotkey
            ? `Press ${hotkey} from anywhere to save instantly, or review what goes in:`
            : "Save what's open now as a new snapshot. You choose what goes in."}
        </span>
        <button className="primary" disabled={disabled} onClick={() => setOpen(true)}>
          Capture…
        </button>
      </div>
      {report && (
        <div className="banner banner-ok">
          <span>
            Saved v{report.snapshot.version}
            {report.identicalToPrevious && " (identical to the previous version)"}.
            {report.pinnedVersion !== null && ` Restore still uses pinned v${report.pinnedVersion}.`}
            {report.autoPruned.length > 0 && ` Auto-pruned ${plural(report.autoPruned.length, "old snapshot")}.`}
          </span>
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

/** "Desktop icon": the same shortcut `contextflow shortcut` makes. */
function ShortcutButton({ workspace, disabled }: { workspace: string; disabled: boolean }) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string }>();

  async function create(overwrite: boolean) {
    setBusy(true);
    setResult(undefined);
    try {
      const r = await api.createShortcut(workspace, overwrite);
      setResult({ ok: true, text: `Icon created: ${r.path}` });
    } catch (e) {
      const err = toCommandError(e);
      if (err.code === "already_exists" && !overwrite) {
        if (window.confirm(`A desktop icon for "${workspace}" already exists. Replace it?`)) {
          return create(true);
        }
      } else {
        setResult({ ok: false, text: err.message });
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <span className="shortcut">
      <button
        disabled={disabled || busy}
        onClick={() => create(false)}
        title={disabled ? "Capture the workspace first" : "Double-click the icon to restore this workspace"}
      >
        {busy ? "Creating…" : "Desktop icon"}
      </button>
      {result && (
        <span className={`shortcut-result ${result.ok ? "ok" : "bad"}`} role="status">
          {result.text}
        </span>
      )}
    </span>
  );
}
