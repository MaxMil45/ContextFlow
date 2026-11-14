import { useEffect, useState } from "react";

import { api, toCommandError, type CommandError, type CompareResult, type DiffStatus } from "../ipc/client";
import { Modal } from "./dialogs";
import { Badge, ErrorBanner, Spinner } from "./ui";

const STATUS_LABEL: Record<DiffStatus, string> = { added: "Added", removed: "Removed", changed: "Changed" };
const STATUS_TONE = { added: "ok", removed: "bad", changed: "warn" } as const;

/**
 * What differs between two snapshots, or between a snapshot and the screen
 * right now: advice first, then the changes grouped by category.
 */
export function CompareDialog({
  workspace,
  from,
  to,
  onClose,
}: {
  workspace: string;
  from: number;
  /** `undefined` = compare with the screen right now. */
  to?: number;
  onClose: () => void;
}) {
  const [result, setResult] = useState<CompareResult>();
  const [error, setError] = useState<CommandError>();

  useEffect(() => {
    api
      .compareSnapshots(workspace, from, to)
      .then(setResult)
      .catch((e) => setError(toCommandError(e)));
  }, [workspace, from, to]);

  return (
    <Modal title={result ? `${result.from} → ${result.to}` : "Comparing…"} onClose={onClose}>
      {error && <ErrorBanner error={error} />}
      {!result && !error && <Spinner label={to === undefined ? "Looking at your screen…" : "Comparing…"} />}
      {result && (
        <div className="compare">
          {result.isEmpty ? (
            <p>No differences.</p>
          ) : (
            <>
              {result.recommendations.length > 0 && (
                <div className="banner banner-warn compare-advice">
                  <strong>Before restoring</strong>
                  <ul>
                    {result.recommendations.map((r) => (
                      <li key={r}>{r}</li>
                    ))}
                  </ul>
                </div>
              )}
              {result.groups.map((g) => (
                <section key={g.category} className="compare-group">
                  <h4>
                    {g.label} <span className="muted">({g.entries.length})</span>
                  </h4>
                  <ul>
                    {g.entries.map((e, i) => (
                      <li key={`${e.title}-${i}`}>
                        <Badge tone={STATUS_TONE[e.status]}>{STATUS_LABEL[e.status]}</Badge>{" "}
                        <span className="mono">{e.title}</span>
                        {e.details.map((d) => (
                          <div key={d} className="muted small compare-detail">
                            {d}
                          </div>
                        ))}
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </>
          )}
        </div>
      )}
      <div className="modal-actions">
        <button onClick={onClose}>Close</button>
      </div>
    </Modal>
  );
}
