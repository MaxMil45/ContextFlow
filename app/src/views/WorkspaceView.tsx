import { useState, type FormEvent } from "react";

import { IconPicker } from "../components/IconPicker";
import { Badge, Empty, ErrorBanner, Spinner } from "../components/ui";
import { absoluteTime, captureStatusTone, plural, relativeTime } from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type CaptureReport,
  type CommandError,
  type SnapshotDetail,
  type WorkspaceSummary,
} from "../ipc/client";
import { RestorePanel } from "./RestorePanel";

type Props = {
  workspace: WorkspaceSummary;
  onChanged: () => void;
};

type RestoreRun = { dryRun: boolean; version?: number; key: number };

export function WorkspaceView({ workspace, onChanged }: Props) {
  const name = workspace.name;
  const history = useCommand(() => api.listSnapshots(name), [name]);
  const [selectedVersion, setSelectedVersion] = useState<number>();
  const version = selectedVersion ?? workspace.pinnedVersion ?? workspace.latest?.version;
  const snapshot = useCommand(
    () => (version === undefined ? Promise.resolve(undefined) : api.getSnapshot(name, version)),
    [name, version],
  );
  const [restore, setRestore] = useState<RestoreRun>();

  const refresh = () => {
    history.reload();
    snapshot.reload();
    onChanged();
  };

  const canRestore = workspace.latest !== null && workspace.isTrusted && !restore;

  return (
    <main className="content">
      <header className="content-header">
        <div className="title">
          <IconPicker workspace={name} />
          <div>
          <h1>{name}</h1>
          {workspace.description && <p className="muted">{workspace.description}</p>}
          {workspace.pinnedVersion !== null && (
            <p className="muted">Restore uses pinned v{workspace.pinnedVersion}.</p>
          )}
          </div>
        </div>
        <div className="actions">
          <ShortcutButton workspace={name} disabled={workspace.latest === null} />
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
          This workspace was imported and has not been approved. Review it with{" "}
          <code>contextflow workspace trust "{name}"</code> before restoring.
        </div>
      )}

      {restore && (
        <RestorePanel
          key={restore.key}
          workspace={name}
          version={restore.version}
          dryRun={restore.dryRun}
          onClose={() => setRestore(undefined)}
          onFinished={onChanged}
        />
      )}

      <CaptureCard workspace={name} disabled={workspace.isArchived} onCaptured={() => {
        setSelectedVersion(undefined);
        refresh();
      }} />

      <div className="columns">
        <section className="card">
          <header className="card-header">
            <h3>History</h3>
          </header>
          {history.error && <ErrorBanner error={history.error} />}
          {history.loading && !history.data && <Spinner />}
          {history.data?.length === 0 && <p className="muted">No snapshots yet. Capture one above.</p>}
          <ol className="timeline">
            {history.data?.map((s) => (
              <li key={s.version}>
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
            <SnapshotDetails snapshot={snapshot.data} />
          ) : (
            !snapshot.loading && <Empty title="Nothing captured yet">Open your apps, then capture.</Empty>
          )}
        </section>
      </div>
    </main>
  );
}

function SnapshotDetails({ snapshot }: { snapshot: SnapshotDetail }) {
  const windows = snapshot.applications.reduce((n, a) => n + a.windowCount, 0);
  return (
    <>
      <header className="card-header">
        <h3>
          v{snapshot.version} · {absoluteTime(snapshot.createdAt)}
        </h3>
        <Badge tone={captureStatusTone[snapshot.captureStatus]}>{snapshot.captureStatus}</Badge>
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
  onCaptured,
}: {
  workspace: string;
  disabled: boolean;
  onCaptured: () => void;
}) {
  const [note, setNote] = useState("");
  const [unminimize, setUnminimize] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError>();
  const [report, setReport] = useState<CaptureReport>();

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(undefined);
    setReport(undefined);
    try {
      const r = await api.capture({ workspace, note: note.trim() || undefined, unminimize });
      setReport(r);
      setNote("");
      onCaptured();
    } catch (err) {
      setError(toCommandError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="card">
      <form className="capture" onSubmit={submit}>
        <input
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="What are you working on? (saved with the snapshot)"
          aria-label="Progress note"
          disabled={busy || disabled}
        />
        <label className="check">
          <input
            type="checkbox"
            checked={unminimize}
            onChange={(e) => setUnminimize(e.target.checked)}
            disabled={busy || disabled}
          />
          Save minimized windows as normal
        </label>
        <button type="submit" className="primary" disabled={busy || disabled}>
          {busy ? "Capturing…" : "Capture now"}
        </button>
      </form>
      {error && <ErrorBanner error={error} onDismiss={() => setError(undefined)} />}
      {report && (
        <div className="banner banner-ok">
          Saved v{report.snapshot.version}
          {report.identicalToPrevious && " (identical to the previous version)"}.
          {report.pinnedVersion !== null &&
            ` Restore still uses pinned v${report.pinnedVersion}.`}
          {report.autoPruned.length > 0 && ` Auto-pruned ${plural(report.autoPruned.length, "old snapshot")}.`}
          {report.warnings.length > 0 && (
            <ul className="warnings">
              {report.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
        </div>
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
