import { useEffect, useRef, useState } from "react";

import { CleanupResults, PlanItemStatus, PlanSummary } from "../components/CleanupSection";
import { Badge, ErrorBanner, Notice, Spinner } from "../components/ui";
import { Clock, GitBranch } from "lucide-react";
import { formatDuration, friendlyAppName, relativeTime, restoreStatusLabel, restoreStatusTone } from "../format";
import { api, toCommandError, type CleanupPlan, type CommandError, type RestoreReport } from "../ipc/client";

type Props = {
  workspace: string;
  /** The workspace name, for the title. */
  label: string;
  version?: number;
  dryRun: boolean;
  /** "switch" saves the workspace you're in first, then restores this one. */
  mode?: "restore" | "switch";
  /** A report that already exists (a switch from the switcher): show it, run nothing. */
  initialReport?: RestoreReport;
  /** Restore only these programs (a retry). */
  only?: string[];
  /** From a preview: go ahead and restore for real. */
  onStart?: () => void;
  /** Restore again just the programs that failed. */
  onRetry?: (ids: string[]) => void;
  onClose: () => void;
  onFinished: () => void;
};

/**
 * Runs one restore: live progress lines from `restore://progress`, a Cancel
 * button while it runs, then the per-application report.
 */
export function RestorePanel({ workspace, label, version, dryRun, mode = "restore", initialReport, only, onStart, onRetry, onClose, onFinished }: Props) {
  const [lines, setLines] = useState<string[]>([]);
  const [report, setReport] = useState<RestoreReport | undefined>(initialReport);
  const [error, setError] = useState<CommandError>();
  const [cancelling, setCancelling] = useState(false);
  const [cleanupPlan, setCleanupPlan] = useState<CleanupPlan>();
  const started = useRef(false);

  useEffect(() => {
    // StrictMode runs effects twice in development; start the restore once.
    if (started.current || initialReport) return;
    started.current = true;

    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await api.onRestoreProgress((p) => {
        if (p.workspace === workspace) setLines((l) => [...l, p.message]);
      });
      try {
        const request = { workspace, version, dryRun, only };
        setReport(await (mode === "switch" && !dryRun && !only ? api.switchWorkspace(request) : api.restore(request)));
      } catch (e) {
        setError(toCommandError(e));
      } finally {
        unlisten?.();
        onFinished();
      }
    })();
    return () => unlisten?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A preview also shows what the cleanup would close when entering.
  useEffect(() => {
    if (!dryRun) return;
    api
      .previewCleanup(workspace, "entering")
      .then(setCleanupPlan)
      .catch(() => {});
  }, [dryRun, workspace]);

  const running = !report && !error;
  // Programs worth trying again: they failed or only partly opened.
  const retryable =
    !dryRun && report
      ? report.steps
          .filter((s) => s.status === "failed" || s.status === "partial_success")
          .map((s) => s.applicationId)
      : [];

  return (
    <section className="card restore-panel" aria-live="polite">
      <header className="card-header">
        <h3>
          {dryRun ? "Restore preview" : only ? "Retrying" : mode === "switch" ? "Switching to" : "Restoring"} · {label}
          {report && ` v${report.version}`}
        </h3>
        {running ? (
          <button
            className="danger"
            disabled={cancelling || dryRun}
            onClick={async () => {
              setCancelling(true);
              await api.cancelRestore().catch(() => {});
            }}
          >
            {cancelling ? "Cancelling…" : "Cancel"}
          </button>
        ) : (
          <button onClick={onClose}>Close</button>
        )}
      </header>

      {report && !dryRun && !report.cancelled && <WelcomeBack report={report} />}

      {running && (
        <div className="progress">
          <Spinner label={lines.at(-1) ?? "Starting…"} />
          {lines.length > 1 && (
            <ol className="log">
              {lines.map((l, i) => (
                <li key={i} className={i === lines.length - 1 ? "current" : undefined}>
                  {l}
                </li>
              ))}
            </ol>
          )}
        </div>
      )}

      {error && <ErrorBanner error={error} />}

      {dryRun && cleanupPlan && cleanupPlan.items.length > 0 && (
        <div className="cleanup-results">
          <div className="muted small">Cleanup that would run first</div>
          <ul className="cleanup-confirm">
            {cleanupPlan.items.map((i) => (
              <li key={i.process}>
                <span className="mono">{i.displayName}</span> · <PlanItemStatus item={i} />
              </li>
            ))}
          </ul>
          <p className="small">
            <PlanSummary plan={cleanupPlan} />
          </p>
        </div>
      )}

      {report?.cleanups.map((c) => <CleanupResults key={c.phase} report={c} />)}

      {report && (
        <>
          <p className="summary">
            {dryRun ? (
              <>
                <Badge tone="muted">Preview</Badge>
                <span className="muted"> Nothing was started or moved.</span>
              </>
            ) : (
              <>
                <Badge tone={restoreStatusTone[report.overall]}>{restoreStatusLabel[report.overall]}</Badge>
                <span className="muted num"> Ready in {formatDuration(report.durationMs)}</span>
                {report.layoutNote && <span className="muted"> · {report.layoutNote}</span>}
              </>
            )}
            {report.cancelled && <span className="muted"> Cancelled: apps not yet started were skipped.</span>}
          </p>
          {report.steps.length === 0 ? (
            <p className="muted">Nothing to restore: no applications in this snapshot.</p>
          ) : (
            <table className="steps">
              <tbody>
                {report.steps.map((s) => (
                  <tr key={s.applicationId}>
                    <td>
                      <Badge tone={restoreStatusTone[s.status]}>{restoreStatusLabel[s.status]}</Badge>
                    </td>
                    <td title={s.applicationId}>{friendlyAppName(s.applicationId)}</td>
                    <td>{s.message}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {dryRun && onStart && (
            <div className="actions left">
              <button className="primary" onClick={onStart}>
                Start restore
              </button>
            </div>
          )}
          {retryable.length > 0 && onRetry && (
            <div className="actions left">
              <button className="primary" onClick={() => onRetry(retryable)}>
                Retry {retryable.length === 1 ? retryable[0] : `${retryable.length} programs`}
              </button>
              <span className="muted small">Only the programs that didn't come back are restored again.</span>
            </div>
          )}
        </>
      )}
    </section>
  );
}

/** The moment you come back: what you were doing, and what happened to the workspace you left. */
function WelcomeBack({ report }: { report: RestoreReport }) {
  const repo = report.repositories[0];
  const dirty = repo ? repo.staged + repo.modified + repo.untracked : 0;
  return (
    <>
      <section className="resume-card" aria-label="Where you left off">
        <span className="eyebrow">Welcome back · where you left off</span>
        <p className="resume-note">
          {report.note ?? "No note was saved with this version."}
        </p>
        <div className="resume-meta">
          <span>
            <Clock size={13} aria-hidden /> Saved {relativeTime(report.savedAt)}
          </span>
          {repo && (
            <span>
              <GitBranch size={13} aria-hidden /> {repo.branch ?? "detached"}
              {dirty > 0 ? ` · ${dirty} changed ${dirty === 1 ? "file" : "files"}` : " · clean"}
            </span>
          )}
        </div>
      </section>
      {report.switchedFrom && (
        <Notice tone={report.autoSaveNote ? "warn" : "ok"}>
          {report.autoSavedVersion !== null
            ? `Saved “${report.switchedFrom}” as v${report.autoSavedVersion} before switching.`
            : `Left “${report.switchedFrom}”. ${report.autoSaveNote ?? ""}`}
        </Notice>
      )}
    </>
  );
}
