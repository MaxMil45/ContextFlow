import { useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";

import {
  api,
  toCommandError,
  type CommandError,
  type DeleteOutcome,
  type TrustReview,
  type WorkspaceSummary,
} from "../ipc/client";
import { plural } from "../format";
import { ErrorBanner, Spinner } from "./ui";

const FOCUSABLE = 'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])';

/**
 * A centred dialog over a dimmed backdrop. Escape and a backdrop click close it,
 * Tab stays inside it, focus moves in when it opens and returns to whatever
 * opened it when it closes.
 */
export function Modal({
  title,
  onClose,
  busy = false,
  children,
}: {
  title: string;
  onClose: () => void;
  busy?: boolean;
  children: ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  // Callers pass inline handlers; keep the latest without re-running the focus logic.
  const latest = useRef({ onClose, busy });
  latest.current = { onClose, busy };

  // Runs once per dialog: remember the opener, take focus, give it back afterwards.
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const el = box.current;
    (el?.querySelector<HTMLElement>("input:not([type=checkbox]), button.primary, button.danger-fill") ??
      el?.querySelector<HTMLElement>(FOCUSABLE))?.focus();
    return () => opener?.focus?.();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !latest.current.busy) {
        e.stopPropagation();
        latest.current.onClose();
      }
      if (e.key === "Tab" && box.current) {
        const items = [...box.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
        if (!items.length) return;
        const first = items[0]!;
        const last = items[items.length - 1]!;
        if (e.shiftKey && document.activeElement === first) {
          last.focus();
          e.preventDefault();
        } else if (!e.shiftKey && document.activeElement === last) {
          first.focus();
          e.preventDefault();
        }
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <div
      className="backdrop"
      onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}
    >
      <div className="modal" role="dialog" aria-modal="true" aria-label={title} ref={box}>
        <h2>{title}</h2>
        {children}
      </div>
    </div>
  );
}

/** Runs `action`, keeps the dialog open and shows the error if it fails. */
function useAction(onDone: () => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError>();
  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(undefined);
    try {
      await action();
      onDone();
    } catch (e) {
      setError(toCommandError(e));
      setBusy(false);
    }
  }
  return { busy, error, run };
}

export function ConfirmDialog({
  title,
  children,
  confirmLabel,
  danger = false,
  action,
  onClose,
}: {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  action: () => Promise<unknown>;
  onClose: () => void;
}) {
  const { busy, error, run } = useAction(onClose);
  return (
    <Modal title={title} onClose={onClose} busy={busy}>
      <div className="modal-body">{children}</div>
      {error && <ErrorBanner error={error} />}
      <div className="modal-actions">
        <button onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button
          className={danger ? "danger-fill" : "primary"}
          disabled={busy}
          onClick={() => run(action)}
        >
          {busy ? "Working…" : confirmLabel}
        </button>
      </div>
    </Modal>
  );
}

export function RenameDialog({
  workspace,
  onRenamed,
  onClose,
}: {
  workspace: WorkspaceSummary;
  onRenamed: (renamed: WorkspaceSummary) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(workspace.name);
  const { busy, error, run } = useAction(onClose);
  const unchanged = name.trim() === workspace.name;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (unchanged || !name.trim()) return;
    void run(async () => onRenamed(await api.renameWorkspace(workspace.id, name.trim())));
  }

  return (
    <Modal title="Rename workspace" onClose={onClose} busy={busy}>
      <form onSubmit={submit}>
        <input
          className="wide"
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label="Workspace name"
          maxLength={120}
          disabled={busy}
          onFocus={(e) => e.currentTarget.select()}
        />
        <p className="muted small">
          Snapshots and the pinned version are kept. An existing desktop icon keeps working under its old
          name; use Desktop icon to make one with the new name.
        </p>
        {error && <ErrorBanner error={error} />}
        <div className="modal-actions">
          <button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={busy || unchanged || !name.trim()}>
            {busy ? "Renaming…" : "Rename"}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/**
 * Review and approve an imported workspace. Approval lets it start every
 * program listed here (from any of its snapshots), so they are all shown.
 */
export function TrustDialog({
  workspace,
  name,
  onApproved,
  onClose,
}: {
  /** The workspace id. */
  workspace: string;
  name: string;
  onApproved: () => void;
  onClose: () => void;
}) {
  const [review, setReview] = useState<TrustReview>();
  const [loadError, setLoadError] = useState<CommandError>();
  const { busy, error, run } = useAction(onClose);

  useEffect(() => {
    api
      .getTrustReview(workspace)
      .then(setReview)
      .catch((e) => setLoadError(toCommandError(e)));
  }, [workspace]);

  return (
    <Modal title="Approve imported workspace" onClose={onClose} busy={busy}>
      <p>
        <strong>{name}</strong> came from a file. If you approve it, restoring can start these
        programs on this computer. Approve it only if you recognise all of them.
      </p>
      {loadError && <ErrorBanner error={loadError} />}
      {!review && !loadError && <Spinner label="Reading the workspace…" />}
      {review && (
        <div className="program-list">
          {review.programs.length === 0 && <p className="muted">It doesn't start any programs.</p>}
          {review.programs.map((p) => (
            <div className="program" key={`${p.applicationId}|${p.executablePath}|${p.arguments}`}>
              <div className="mono">{p.executablePath}</div>
              {p.arguments && <div className="muted small mono">arguments: {p.arguments}</div>}
              <div className="muted small">in {p.versions.map((v) => `v${v}`).join(", ")}</div>
            </div>
          ))}
        </div>
      )}
      {error && <ErrorBanner error={error} />}
      <div className="modal-actions">
        <button onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button
          className="primary"
          disabled={busy || !review}
          onClick={() =>
            run(async () => {
              await api.approveWorkspace(workspace);
              onApproved();
            })
          }
        >
          {busy ? "Approving…" : "Approve"}
        </button>
      </div>
    </Modal>
  );
}

/** Delete a workspace, and optionally its desktop icon(s). */
export function DeleteWorkspaceDialog({
  workspace,
  name,
  snapshotCount,
  onDeleted,
  onClose,
}: {
  /** The workspace id. */
  workspace: string;
  name: string;
  snapshotCount: number;
  /** Called after a successful delete; `outcome.shortcutError` may still be set. */
  onDeleted: (outcome: DeleteOutcome) => void;
  onClose: () => void;
}) {
  const [shortcuts, setShortcuts] = useState<string[]>();
  const [alsoShortcuts, setAlsoShortcuts] = useState(true);
  const { busy, error, run } = useAction(onClose);

  useEffect(() => {
    api
      .getWorkspaceShortcuts(workspace)
      .then(setShortcuts)
      .catch(() => setShortcuts([])); // not being able to look is not a reason to block the dialog
  }, [workspace]);

  const fileName = (p: string) => p.split("\\").pop() ?? p;

  return (
    <Modal title={`Delete "${name}"?`} onClose={onClose} busy={busy}>
      <div className="modal-body">
        <p>
          {snapshotCount > 0
            ? `This permanently deletes the workspace and its ${plural(snapshotCount, "snapshot")}.`
            : "This permanently deletes the workspace."}{" "}
          It can't be undone.
        </p>
        {shortcuts && shortcuts.length > 0 ? (
          <label className="check">
            <input
              type="checkbox"
              checked={alsoShortcuts}
              onChange={(e) => setAlsoShortcuts(e.target.checked)}
              disabled={busy}
            />
            <span>
              Also move its desktop {shortcuts.length === 1 ? "icon" : "icons"} to the Recycle Bin (
              {shortcuts.map(fileName).join(", ")})
            </span>
          </label>
        ) : (
          <p className="muted small">
            To keep the workspace but hide it, archive it instead.
          </p>
        )}
        {shortcuts && shortcuts.length > 0 && !alsoShortcuts && (
          <p className="muted small">The icon will stop working; you'd need to delete it yourself.</p>
        )}
      </div>
      {error && <ErrorBanner error={error} />}
      <div className="modal-actions">
        <button onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button
          className="danger-fill"
          disabled={busy}
          onClick={() =>
            run(async () => onDeleted(await api.deleteWorkspace(workspace, alsoShortcuts && !!shortcuts?.length)))
          }
        >
          {busy ? "Deleting…" : "Delete workspace"}
        </button>
      </div>
    </Modal>
  );
}

/** Name a new workspace; optionally review and save what is open right now into it. */
export function NewWorkspaceDialog({
  onCreated,
  onClose,
}: {
  onCreated: (workspace: WorkspaceSummary, captureNow: boolean) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [captureNow, setCaptureNow] = useState(true);
  const { busy, error, run } = useAction(onClose);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    void run(async () => onCreated(await api.createWorkspace(name.trim()), captureNow));
  }

  return (
    <Modal title="New workspace" onClose={onClose} busy={busy}>
      <form className="modal-body" onSubmit={submit}>
        <input
          className="wide"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. University project"
          aria-label="Workspace name"
          maxLength={120}
          disabled={busy}
        />
        <label className="check">
          <input
            type="checkbox"
            checked={captureNow}
            onChange={(e) => setCaptureNow(e.target.checked)}
            disabled={busy}
          />
          <span>Save the apps I have open now into it</span>
        </label>
        <p className="muted small">
          {captureNow
            ? "You'll choose which windows, tabs and folders go in before anything is saved."
            : "It starts empty. Arrange your apps later and capture when you're ready."}
        </p>
        {error && <ErrorBanner error={error} />}
        <div className="modal-actions">
          <button type="button" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={busy || !name.trim()}>
            {busy ? "Creating…" : captureNow ? "Create and review" : "Create"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
