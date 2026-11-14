import { useEffect, useState } from "react";

import { plural } from "../format";
import {
  api,
  toCommandError,
  type CaptureDiscovery,
  type CaptureReport,
  type CommandError,
} from "../ipc/client";
import { Modal } from "./dialogs";
import { ErrorBanner, Spinner } from "./ui";

/**
 * Capture review: shows everything a capture would save right now (programs,
 * windows, tabs, folders, repositories) as a checklist, so you choose what
 * goes into the snapshot. Add a note, then save.
 */
export function CaptureReview({
  workspace,
  onSaved,
  onClose,
}: {
  workspace: string;
  onSaved: (report: CaptureReport) => void;
  onClose: () => void;
}) {
  const [found, setFound] = useState<CaptureDiscovery>();
  const [loadError, setLoadError] = useState<CommandError>();
  const [apps, setApps] = useState<Set<string>>(new Set());
  const [repos, setRepos] = useState<Set<string>>(new Set());
  const [note, setNote] = useState("");
  const [unminimize, setUnminimize] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<CommandError>();

  useEffect(() => {
    api
      .discoverCapture(workspace)
      .then((d) => {
        setFound(d);
        setApps(new Set(d.apps.map((a) => a.applicationId)));
        setRepos(new Set(d.repositories.map((r) => r.path)));
      })
      .catch((e) => setLoadError(toCommandError(e)));
  }, [workspace]);

  const toggle = (set: Set<string>, setter: (s: Set<string>) => void, key: string) => {
    const next = new Set(set);
    if (!next.delete(key)) next.add(key);
    setter(next);
  };

  async function save() {
    setSaving(true);
    setError(undefined);
    try {
      const report = await api.capture({
        workspace,
        note: note.trim() || undefined,
        unminimize,
        only: [...apps],
        repos: [...repos],
      });
      onSaved(report);
      onClose();
    } catch (e) {
      setError(toCommandError(e));
      setSaving(false);
    }
  }

  const nothing = apps.size === 0 && repos.size === 0;

  return (
    <Modal title={`Capture “${workspace}”`} onClose={onClose} busy={saving}>
      {loadError && <ErrorBanner error={loadError} />}
      {!found && !loadError && <Spinner label="Looking at what's open…" />}
      {found && (
        <>
          <p className="muted small">
            Tick what should go into this snapshot. Anything unticked is left out and won't be restored.
          </p>
          <div className="review-list">
            {found.apps.length === 0 && <p className="muted">No windows were found to save.</p>}
            {found.apps.map((a) => (
              <div className="review-item" key={a.applicationId}>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={apps.has(a.applicationId)}
                    onChange={() => toggle(apps, setApps, a.applicationId)}
                    disabled={saving}
                  />
                  <span className="mono">{a.applicationId}</span>
                  <span className="muted small">
                    {[
                      plural(a.windowCount, "window"),
                      a.tabCount > 0 && plural(a.tabCount, "tab"),
                      a.folderCount > 0 && plural(a.folderCount, "folder"),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </label>
                {a.windowTitles.length > 0 && (
                  <details className="review-titles">
                    <summary className="muted small">Windows</summary>
                    <ul>
                      {a.windowTitles.map((t, i) => (
                        <li key={i} className="small">
                          {t || <em>(no title)</em>}
                        </li>
                      ))}
                    </ul>
                  </details>
                )}
              </div>
            ))}
            {found.repositories.length > 0 && <h4>Git repositories</h4>}
            {found.repositories.map((r) => (
              <div className="review-item" key={r.path}>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={repos.has(r.path)}
                    onChange={() => toggle(repos, setRepos, r.path)}
                    disabled={saving}
                  />
                  <span className="mono">{r.path}</span>
                  <span className="muted small">
                    {r.branch ?? "detached"} · {r.staged} staged, {r.modified} modified, {r.untracked}{" "}
                    untracked
                  </span>
                </label>
              </div>
            ))}
          </div>
          <p className="muted small">
            {plural(found.monitorCount, "monitor")} will be remembered so windows return to the right screen.
            {found.excludedWindows > 0 && ` ${plural(found.excludedWindows, "window")} skipped by your exclusion rules.`}
          </p>
          {found.warnings.map((w) => (
            <div className="banner banner-warn" key={w}>
              {w}
            </div>
          ))}
          <input
            className="wide"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="What are you working on? (saved with the snapshot)"
            aria-label="Progress note"
            disabled={saving}
          />
          {found.minimizedWindows > 0 && (
            <label className="check">
              <input
                type="checkbox"
                checked={unminimize}
                onChange={(e) => setUnminimize(e.target.checked)}
                disabled={saving}
              />
              Save minimized windows as normal ({found.minimizedWindows} minimized)
            </label>
          )}
        </>
      )}
      {error && <ErrorBanner error={error} />}
      <div className="modal-actions">
        <button onClick={onClose} disabled={saving}>
          Cancel
        </button>
        <button className="primary" onClick={save} disabled={saving || !found || nothing}>
          {saving ? "Saving…" : "Save snapshot"}
        </button>
      </div>
    </Modal>
  );
}
