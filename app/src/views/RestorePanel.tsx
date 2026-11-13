import { useEffect, useRef, useState } from "react";

import { Badge, ErrorBanner, Spinner } from "../components/ui";
import { restoreStatusLabel, restoreStatusTone } from "../format";
import { api, toCommandError, type CommandError, type RestoreReport } from "../ipc/client";

type Props = {
  workspace: string;
  version?: number;
  dryRun: boolean;
  onClose: () => void;
  onFinished: () => void;
};

/**
 * Runs one restore: live progress lines from `restore://progress`, a Cancel
 * button while it runs, then the per-application report.
 */
export function RestorePanel({ workspace, version, dryRun, onClose, onFinished }: Props) {
  const [lines, setLines] = useState<string[]>([]);
  const [report, setReport] = useState<RestoreReport>();
  const [error, setError] = useState<CommandError>();
  const [cancelling, setCancelling] = useState(false);
  const started = useRef(false);

  useEffect(() => {
    // StrictMode runs effects twice in development; start the restore once.
    if (started.current) return;
    started.current = true;

    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await api.onRestoreProgress((p) => {
        if (p.workspace === workspace) setLines((l) => [...l, p.message]);
      });
      try {
        setReport(await api.restore({ workspace, version, dryRun }));
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

  const running = !report && !error;

  return (
    <section className="card restore-panel" aria-live="polite">
      <header className="card-header">
        <h3>
          {dryRun ? "Restore preview" : "Restoring"} · {workspace}
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

      {report?.note && (
        <blockquote className="resume-card">
          <span className="muted">Where you left off</span>
          {report.note}
        </blockquote>
      )}

      {running && (
        <div className="progress">
          <Spinner label={lines.at(-1) ?? "Starting…"} />
        </div>
      )}

      {error && <ErrorBanner error={error} />}

      {report && (
        <>
          <p className="summary">
            <Badge tone={restoreStatusTone[report.overall]}>{restoreStatusLabel[report.overall]}</Badge>
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
                    <td className="mono">{s.applicationId}</td>
                    <td>{s.message}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </section>
  );
}
