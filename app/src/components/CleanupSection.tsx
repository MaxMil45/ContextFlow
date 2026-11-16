import { useEffect, useMemo, useState } from "react";

import { formatBytes, formatCpu, friendlyAppName } from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type CleanupConfig,
  type CleanupPhase,
  type CleanupPlan,
  type CleanupReport,
  type CleanupTarget,
  type CloseMode,
  type CommandError,
  type PlanItem,
  type RunningApp,
} from "../ipc/client";
import { ConfirmDialog, Modal } from "./dialogs";
import { Badge, ErrorBanner, Spinner } from "./ui";

const TITLES: Record<CleanupPhase, { title: string; hint: string }> = {
  entering: {
    title: "When I enter this workspace",
    hint: "Closed when you restore it: distractions you don't want while you work here.",
  },
  leaving: {
    title: "When I leave this workspace",
    hint: "Closed when you restore a different workspace, or choose Leave workspace.",
  },
};

/** One line describing what a plan item would do. */
export function PlanItemStatus({ item }: { item: PlanItem }) {
  const a = item.action;
  switch (a.kind) {
    case "close":
      return (
        <span>
          Running · {a.processCount} {a.processCount === 1 ? "process" : "processes"} ·{" "}
          {formatBytes(a.ramBytes)} · {formatCpu(a.cpuPercent)} CPU
        </span>
      );
    case "not_running":
      return <span className="muted">Not running</span>;
    case "protected":
      return <span className="muted">Never closed: {a.reason}</span>;
    case "kept_by_workspace":
      return <span className="muted">Kept open: this workspace uses it</span>;
  }
}

/** What a cleanup would give back, in one sentence. */
export function PlanSummary({ plan }: { plan: CleanupPlan }) {
  const closing = plan.items.filter((i) => i.action.kind === "close").length;
  if (closing === 0) return <span className="muted">Nothing running would be closed right now.</span>;
  return (
    <span>
      Closing {closing === 1 ? "this program" : `these ${closing} programs`} would free about{" "}
      <strong>{formatBytes(plan.ramBytes)}</strong> of memory and <strong>{formatCpu(plan.cpuPercent)}</strong>{" "}
      of CPU.
    </span>
  );
}

const OUTCOME_TONE = {
  closed: "ok",
  terminated: "ok",
  still_running: "warn",
  not_running: "muted",
  protected: "muted",
  kept_by_workspace: "muted",
  failed: "bad",
} as const;

const OUTCOME_LABEL = {
  closed: "Closed",
  terminated: "Force closed",
  still_running: "Still running",
  not_running: "Not running",
  protected: "Refused",
  kept_by_workspace: "Kept",
  failed: "Failed",
} as const;

/** What a cleanup that already ran did to each program. */
export function CleanupResults({ report }: { report: CleanupReport }) {
  if (report.results.length === 0) return null;
  return (
    <div className="cleanup-results">
      <div className="muted small">
        {report.phase === "leaving" ? "Closed when leaving" : "Closed when entering"} “{report.workspace}”
      </div>
      <table className="steps">
        <tbody>
          {report.results.map((r) => (
            <tr key={r.process}>
              <td>
                <Badge tone={OUTCOME_TONE[r.outcome]}>{OUTCOME_LABEL[r.outcome]}</Badge>
              </td>
              <td className="mono">{r.process}</td>
              <td>{r.message}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * Workspace Cleanup: which programs close when this workspace is entered or
 * left, with a preview of what that would free before anything is closed.
 */
export function CleanupSection({ workspace, onActivity }: { workspace: string; onActivity: () => void }) {
  const config = useCommand(() => api.getCleanupConfig(workspace), [workspace]);
  const [error, setError] = useState<CommandError>();
  const [report, setReport] = useState<CleanupReport>();

  async function save(next: CleanupConfig) {
    setError(undefined);
    try {
      await api.setCleanupConfig(workspace, next);
      config.reload();
    } catch (e) {
      setError(toCommandError(e));
    }
  }

  const data = config.data;
  return (
    <section className="card cleanup">
      <header className="card-header">
        <div>
          <h3>Focus rules</h3>
          <p className="muted small">
            Close the programs that distract you here, and free their memory and CPU. Only the programs you
            list are touched, and critical Windows programs are never closed.
          </p>
        </div>
      </header>
      {error && <ErrorBanner error={error} onDismiss={() => setError(undefined)} />}
      {config.error && <ErrorBanner error={config.error} />}
      {!data && !config.error && <Spinner />}
      {data && (
        <div className="cleanup-sides">
          {(["entering", "leaving"] as const).map((phase) => (
            <CleanupSide
              key={phase}
              workspace={workspace}
              phase={phase}
              targets={phase === "entering" ? data.onEnter : data.onExit}
              onChange={(targets) =>
                save(phase === "entering" ? { ...data, onEnter: targets } : { ...data, onExit: targets })
              }
              onRan={(r) => {
                setReport(r);
                onActivity();
              }}
            />
          ))}
        </div>
      )}
      {report && (
        <div className="cleanup-report">
          <CleanupResults report={report} />
          <button className="link" onClick={() => setReport(undefined)}>
            Dismiss
          </button>
        </div>
      )}
    </section>
  );
}

function CleanupSide({
  workspace,
  phase,
  targets,
  onChange,
  onRan,
}: {
  workspace: string;
  phase: CleanupPhase;
  targets: CleanupTarget[];
  onChange: (targets: CleanupTarget[]) => void;
  onRan: (report: CleanupReport) => void;
}) {
  const [refresh, setRefresh] = useState(0);
  const key = JSON.stringify(targets);
  // Refresh measures again; other reloads may reuse the last few seconds' sample.
  const plan = useCommand(
    () => (targets.length ? api.previewCleanup(workspace, phase, refresh > 0) : Promise.resolve(undefined)),
    [workspace, phase, key, refresh],
  );
  const [picking, setPicking] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const items = useMemo(() => new Map(plan.data?.items.map((i) => [i.process, i])), [plan.data]);
  const closing = plan.data?.items.filter((i) => i.action.kind === "close") ?? [];
  const anyForce = closing.some((i) => i.mode === "force");

  const setMode = (process: string, mode: CloseMode) =>
    onChange(targets.map((t) => (t.process === process ? { ...t, mode } : t)));

  return (
    <div className="cleanup-side">
      <h4>{TITLES[phase].title}</h4>
      <p className="muted small">{TITLES[phase].hint}</p>

      {targets.length === 0 && <p className="muted">No programs listed.</p>}
      <ul className="cleanup-list">
        {targets.map((t) => {
          const item = items.get(t.process);
          return (
            <li key={t.process}>
              <div className="cleanup-row">
                <span className="cleanup-name" title={item?.displayName ?? `${t.process}.exe`}>
                  {friendlyAppName(t.process)}
                </span>
                <select
                  value={t.mode}
                  onChange={(e) => setMode(t.process, e.target.value as CloseMode)}
                  aria-label={`How to close ${t.process}`}
                >
                  <option value="graceful">Close nicely</option>
                  <option value="force">Force close</option>
                </select>
                <button
                  className="link"
                  onClick={() => onChange(targets.filter((x) => x.process !== t.process))}
                  aria-label={`Remove ${t.process}`}
                >
                  Remove
                </button>
              </div>
              <div className="small">{item ? <PlanItemStatus item={item} /> : <span className="skeleton skeleton-line short" />}</div>
              {item && item.executables.length > 0 && (
                <div className={`small ${item.executables.length > 1 ? "warn-text" : "muted"}`} title={item.executables.join("\n")}>
                  {item.executables.length > 1
                    ? `Matches ${item.executables.length} different programs named ${t.process}.exe: ${item.executables.join(", ")}`
                    : `Matches ${item.executables[0]}`}
                </div>
              )}
            </li>
          );
        })}
      </ul>

      {plan.data && targets.length > 0 && (
        <p className="small cleanup-summary">
          <PlanSummary plan={plan.data} />
        </p>
      )}
      {plan.error && <ErrorBanner error={plan.error} />}

      <div className="actions left">
        <button onClick={() => setPicking(true)}>Add program…</button>
        {targets.length > 0 && <button onClick={() => setRefresh((n) => n + 1)}>Refresh</button>}
        <button
          className={anyForce ? "danger" : undefined}
          disabled={closing.length === 0}
          onClick={() => setConfirming(true)}
          title="Close these programs now, without restoring anything"
        >
          Close these now…
        </button>
      </div>
      {targets.some((t) => t.mode === "graceful") && (
        <p className="muted small">
          Some apps (Discord, Steam, launchers) keep running in the tray when closed. Use Force close for
          those.
        </p>
      )}

      {picking && (
        <AppPicker
          existing={targets.map((t) => t.process)}
          onPick={(process) => {
            onChange([...targets, { process, mode: "graceful" }]);
            setPicking(false);
          }}
          onClose={() => setPicking(false)}
        />
      )}
      {confirming && plan.data && (
        <ConfirmDialog
          title="Close these programs now?"
          confirmLabel={anyForce ? "Close (some by force)" : "Close"}
          danger={anyForce}
          action={async () => onRan(await api.runCleanupNow(workspace, phase))}
          onClose={() => {
            setConfirming(false);
            setRefresh((n) => n + 1);
          }}
        >
          <ul className="cleanup-confirm">
            {closing.map((i) => (
              <li key={i.process}>
                {friendlyAppName(i.process)} ·{" "}
                {i.mode === "force" ? "force close" : "close nicely"} · <PlanItemStatus item={i} />
              </li>
            ))}
          </ul>
          <p>
            <PlanSummary plan={plan.data} />
          </p>
          {anyForce && (
            <p className="muted small">
              Force close ends a program if it doesn't close when asked. Anything unsaved in it is lost, except that a program showing a dialog (such as "Save changes?") is left running for you to answer.
            </p>
          )}
        </ConfirmDialog>
      )}
    </div>
  );
}

/** Pick one of the running programs, or type a name for one that isn't running. */
function AppPicker({
  existing,
  onPick,
  onClose,
}: {
  existing: string[];
  onPick: (process: string) => void;
  onClose: () => void;
}) {
  const [apps, setApps] = useState<RunningApp[]>();
  const [error, setError] = useState<CommandError>();
  const [query, setQuery] = useState("");
  const [typed, setTyped] = useState("");

  useEffect(() => {
    api
      .listRunningApps()
      .then(setApps)
      .catch((e) => setError(toCommandError(e)));
  }, []);

  const shown = apps?.filter((a) => a.process.includes(query.trim().toLowerCase()));

  return (
    <Modal title="Add a program" onClose={onClose}>
      <input
        className="wide"
        placeholder="Search running programs…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Search running programs"
      />
      {error && <ErrorBanner error={error} />}
      {!apps && !error && <Spinner label="Looking at what's running…" />}
      {shown && (
        <div className="picker-list">
          <table>
            <thead>
              <tr>
                <th>Program</th>
                <th className="num">Memory</th>
                <th className="num">CPU</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.map((a) => {
                const added = existing.includes(a.process);
                return (
                  <tr key={a.process} className={a.protected ? "dim" : undefined}>
                    <td>
                      <span className="mono">{a.displayName}</span>
                      {a.processCount > 1 && <span className="muted small"> ×{a.processCount}</span>}
                      {a.protected && <div className="muted small">Never closed: {a.protected}</div>}
                    </td>
                    <td className="num">{formatBytes(a.ramBytes)}</td>
                    <td className="num">{formatCpu(a.cpuPercent)}</td>
                    <td className="num">
                      <button disabled={!!a.protected || added} onClick={() => onPick(a.process)}>
                        {added ? "Added" : "Add"}
                      </button>
                    </td>
                  </tr>
                );
              })}
              {shown.length === 0 && (
                <tr>
                  <td colSpan={4} className="muted">
                    Nothing running matches.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
      <form
        className="picker-type"
        onSubmit={(e) => {
          e.preventDefault();
          if (typed.trim()) onPick(typed.trim());
        }}
      >
        <input
          placeholder="Not running? Type its name, like discord"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          aria-label="Program name"
        />
        <button type="submit" disabled={!typed.trim()}>
          Add
        </button>
      </form>
      <div className="modal-actions">
        <button onClick={onClose}>Cancel</button>
      </div>
    </Modal>
  );
}
