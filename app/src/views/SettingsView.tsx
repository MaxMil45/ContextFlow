import { useEffect, useState, type ReactNode } from "react";

import { ConfirmDialog } from "../components/dialogs";
import { Badge, ErrorBanner, Spinner } from "../components/ui";
import { plural } from "../format";
import { useCommand } from "../hooks";
import {
  api,
  toCommandError,
  type AppInfo,
  type BrowserStatus,
  type CommandError,
  type RetentionResult,
  type Settings,
  type VerifyResult,
} from "../ipc/client";

/** A list of text values (program names, patterns) with add and remove. */
function ListEditor({
  label,
  hint,
  values,
  placeholder,
  onChange,
}: {
  label: string;
  hint?: string;
  values: string[];
  placeholder: string;
  onChange: (values: string[]) => void;
}) {
  const [text, setText] = useState("");
  const add = () => {
    const v = text.trim();
    if (v && !values.includes(v)) onChange([...values, v]);
    setText("");
  };
  return (
    <div className="field">
      <div className="field-label">{label}</div>
      {hint && <div className="muted small">{hint}</div>}
      <div className="chips">
        {values.map((v) => (
          <span className="chip" key={v}>
            {v}
            <button className="link" onClick={() => onChange(values.filter((x) => x !== v))} aria-label={`Remove ${v}`}>
              ×
            </button>
          </span>
        ))}
        {values.length === 0 && <span className="muted small">None</span>}
      </div>
      <form
        className="inline-form"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input value={text} onChange={(e) => setText(e.target.value)} placeholder={placeholder} aria-label={label} />
        <button type="submit" disabled={!text.trim()}>
          Add
        </button>
      </form>
    </div>
  );
}

function Toggle({
  checked,
  onChange,
  children,
  hint,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  children: ReactNode;
  hint?: string;
}) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        {children}
        {hint && <span className="muted small block">{hint}</span>}
      </span>
    </label>
  );
}

/** A whole number, or empty for "not set" when `optional`. */
function NumberField({
  label,
  value,
  min,
  max,
  optional = false,
  suffix,
  onChange,
}: {
  label: string;
  value: number | null;
  min: number;
  max?: number;
  optional?: boolean;
  suffix?: string;
  onChange: (v: number | null) => void;
}) {
  return (
    <label className="field number-field">
      <span className="field-label">{label}</span>
      <span>
        <input
          type="number"
          min={min}
          max={max}
          value={value ?? ""}
          placeholder={optional ? "not set" : undefined}
          onChange={(e) => {
            const raw = e.target.value;
            if (raw === "") return onChange(optional ? null : min);
            onChange(Math.trunc(Number(raw)));
          }}
        />
        {suffix && <span className="muted small"> {suffix}</span>}
      </span>
    </label>
  );
}

export function SettingsView({ info }: { info: AppInfo | undefined }) {
  const loaded = useCommand(() => api.getSettings(), []);
  const browser = useCommand(() => api.getBrowserStatus(), []);
  const [draft, setDraft] = useState<Settings>();
  const [saved, setSaved] = useState<Settings>();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<CommandError>();
  const [notice, setNotice] = useState<string>();
  const [retention, setRetention] = useState<RetentionResult>();
  const [verify, setVerify] = useState<VerifyResult>();
  const [busy, setBusy] = useState<string>();

  useEffect(() => {
    if (loaded.data) {
      setDraft(loaded.data);
      setSaved(loaded.data);
    }
  }, [loaded.data]);

  if (loaded.error) return <main className="content"><ErrorBanner error={loaded.error} /></main>;
  if (!draft || !saved) return <main className="content"><Spinner label="Loading settings…" /></main>;

  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const set = (patch: (s: Settings) => Settings) => {
    setNotice(undefined);
    setDraft((d) => (d ? patch(d) : d));
  };

  async function save() {
    setSaving(true);
    setError(undefined);
    try {
      const next = await api.saveSettings(draft!);
      setDraft(next);
      setSaved(next);
      setNotice("Settings saved.");
    } catch (e) {
      setError(toCommandError(e));
    } finally {
      setSaving(false);
    }
  }

  /** Run a data action and show what it did. */
  async function action<T>(name: string, run: () => Promise<T>, message: (r: NonNullable<T>) => string) {
    setBusy(name);
    setError(undefined);
    setNotice(undefined);
    try {
      const r = await run();
      if (r) setNotice(message(r as NonNullable<T>));
    } catch (e) {
      setError(toCommandError(e));
    } finally {
      setBusy(undefined);
    }
  }

  const b = draft.browser;
  const link = (s: BrowserStatus) => s.browsers;

  return (
    <main className="content settings">
      <header className="content-header sticky">
        <h1>Settings</h1>
        <div className="actions">
          <button disabled={!dirty || saving} onClick={() => setDraft(saved)}>
            Revert
          </button>
          <button className="primary" disabled={!dirty || saving} onClick={save}>
            {saving ? "Saving…" : "Save settings"}
          </button>
        </div>
      </header>

      {error && <ErrorBanner error={error} onDismiss={() => setError(undefined)} />}
      {notice && <div className="banner banner-ok">{notice}</div>}
      {dirty && <div className="banner banner-muted">You have unsaved changes.</div>}

      <section className="card">
        <h3>General</h3>
        <Toggle
          checked={draft.app.notifications}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, notifications: v } }))}
          hint="A Windows notification when a restore finishes, needs attention or fails."
        >
          Notifications
        </Toggle>
        <Toggle
          checked={draft.app.closeToTray}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, closeToTray: v } }))}
          hint="Closing the window keeps ContextFlow in the tray, ready for Capture and Restore."
        >
          Keep running in the tray when the window is closed
        </Toggle>
        <Toggle
          checked={draft.app.startWithWindows}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, startWithWindows: v } }))}
          hint="Starts hidden in the tray when you sign in to Windows."
        >
          Start with Windows
        </Toggle>
      </section>

      <section className="card">
        <h3>What gets captured</h3>
        <Toggle
          checked={draft.capture.includeWindows}
          onChange={(v) => set((s) => ({ ...s, capture: { ...s.capture, includeWindows: v } }))}
          hint="Programs, their windows and where they are on your monitors."
        >
          Windows and layout
        </Toggle>
        <ListEditor
          label="Never capture these programs"
          hint="Program names, like discord. Their windows are left out of every snapshot."
          values={draft.capture.excludedProcesses}
          placeholder="discord"
          onChange={(v) => set((s) => ({ ...s, capture: { ...s.capture, excludedProcesses: v } }))}
        />
        <ListEditor
          label="Skip windows whose title contains"
          hint="Not case-sensitive. Useful for private windows, e.g. “Incognito”."
          values={draft.capture.excludedTitlePatterns}
          placeholder="Incognito"
          onChange={(v) => set((s) => ({ ...s, capture: { ...s.capture, excludedTitlePatterns: v } }))}
        />
      </section>

      <section className="card">
        <h3>Browser tabs</h3>
        <Toggle
          checked={b.captureTabs}
          onChange={(v) => set((s) => ({ ...s, browser: { ...s.browser, captureTabs: v } }))}
          hint="Save and reopen the tabs of Chrome and Edge. Needs the ContextFlow browser extension."
        >
          Save and reopen browser tabs
        </Toggle>
        <ListEditor
          label="Never save tabs whose address contains"
          hint="Not case-sensitive, e.g. mybank.com or token="
          values={b.excludedUrlPatterns}
          placeholder="mybank.com"
          onChange={(v) => set((s) => ({ ...s, browser: { ...s.browser, excludedUrlPatterns: v } }))}
        />
        <NumberField
          label="Most tabs reopened per window"
          value={b.maxTabsPerWindow}
          min={1}
          max={200}
          onChange={(v) => set((s) => ({ ...s, browser: { ...s.browser, maxTabsPerWindow: v ?? 50 } }))}
        />

        <h4>Browser extension</h4>
        {browser.data ? (
          <>
            <ul className="status-list">
              {link(browser.data).map((x) => (
                <li key={x.name}>
                  <strong>{x.name}</strong>{" "}
                  <Badge tone={x.registered ? "ok" : "muted"}>{x.registered ? "set up" : "not set up"}</Badge>{" "}
                  <Badge tone={x.connected ? "ok" : "muted"}>
                    {x.connected ? `connected · ${plural(x.tabs, "tab")}` : "extension not connected"}
                  </Badge>
                </li>
              ))}
            </ul>
            <div className="actions left">
              <button
                disabled={busy === "browser"}
                onClick={() =>
                  action("browser", api.installBrowserIntegration, () => "Browser integration set up.").then(() =>
                    browser.reload(),
                  )
                }
              >
                {browser.data.manifestPresent ? "Repair setup" : "Set up Chrome and Edge"}
              </button>
              <button onClick={() => api.openFolder("extension").catch((e) => setError(toCommandError(e)))}>
                Open extension folder
              </button>
              <button
                className="danger"
                disabled={!browser.data.manifestPresent || busy === "browser"}
                onClick={() =>
                  action("browser", api.removeBrowserIntegration, () => "Browser integration removed.").then(() =>
                    browser.reload(),
                  )
                }
              >
                Remove
              </button>
              <button onClick={browser.reload}>Refresh</button>
            </div>
            <ol className="muted small steps-help">
              <li>Click “Set up Chrome and Edge”.</li>
              <li>In Chrome open chrome://extensions (Edge: edge://extensions) and turn on Developer mode.</li>
              <li>Click “Load unpacked” and choose the extension folder (use the button above).</li>
            </ol>
            <p className="muted small">
              The extension sees each tab's address and title only. It cannot read cookies, history, passwords
              or page content, and it ignores private windows.
            </p>
          </>
        ) : (
          <Spinner />
        )}
      </section>

      <section className="card">
        <h3>Restore</h3>
        <NumberField
          label="Longest a whole restore may take"
          value={draft.timeouts.restoreOverallSecs}
          min={10}
          max={1800}
          suffix="seconds"
          onChange={(v) => set((s) => ({ ...s, timeouts: { ...s.timeouts, restoreOverallSecs: v ?? 120 } }))}
        />
        <NumberField
          label="Longest a Git command may take"
          value={draft.timeouts.gitMs}
          min={100}
          max={600000}
          suffix="milliseconds"
          onChange={(v) => set((s) => ({ ...s, timeouts: { ...s.timeouts, gitMs: v ?? 10000 } }))}
        />
      </section>

      <section className="card">
        <h3>Keeping history</h3>
        <p className="muted small">
          Old snapshots are deleted only when they break <em>every</em> rule you set. The newest snapshot and a
          pinned one are always kept.
        </p>
        <NumberField
          label="Keep at least the newest"
          value={draft.retention.keepLast}
          min={1}
          optional
          suffix="snapshots"
          onChange={(v) => set((s) => ({ ...s, retention: { ...s.retention, keepLast: v } }))}
        />
        <NumberField
          label="Keep everything younger than"
          value={draft.retention.maxAgeDays}
          min={1}
          optional
          suffix="days"
          onChange={(v) => set((s) => ({ ...s, retention: { ...s.retention, maxAgeDays: v } }))}
        />
        <Toggle
          checked={draft.retention.autoPrune}
          onChange={(v) => set((s) => ({ ...s, retention: { ...s.retention, autoPrune: v } }))}
          hint="Applies the rules after every capture. Needs at least one rule above."
        >
          Clean up automatically
        </Toggle>
        <div className="actions left">
          <button
            disabled={dirty || busy === "retention"}
            title={dirty ? "Save your changes first" : undefined}
            onClick={() =>
              action("retention", api.previewRetention, (r) => {
                setRetention(r);
                return "";
              }).then(() => setNotice(undefined))
            }
          >
            Review what would be deleted…
          </button>
        </div>
      </section>

      <section className="card">
        <h3>Your data</h3>
        {info && (
          <dl className="paths">
            <dt>Database</dt>
            <dd className="mono">{info.databasePath}</dd>
            <dt>Settings file</dt>
            <dd className="mono">{info.configPath}</dd>
            <dt>Logs</dt>
            <dd className="mono">{info.logsPath}</dd>
          </dl>
        )}
        <div className="actions left">
          <button onClick={() => api.openFolder("data").catch((e) => setError(toCommandError(e)))}>Open data folder</button>
          <button onClick={() => api.openFolder("logs").catch((e) => setError(toCommandError(e)))}>Open logs</button>
        </div>
        <div className="actions left">
          <button
            disabled={busy === "backup"}
            onClick={() => action("backup", api.backupNow, (r) => `${r.message} ${r.path}`)}
          >
            Back up now…
          </button>
          <button
            disabled={busy === "verify"}
            onClick={() =>
              action("verify", api.verifyData, (r) => {
                setVerify(r);
                return r.ok
                  ? `All good: ${plural(r.snapshotsChecked, "snapshot")} checked.`
                  : "Problems were found; see below.";
              })
            }
          >
            Check data integrity
          </button>
          <button
            disabled={busy === "export"}
            onClick={() => action("export", () => api.exportWorkspaces(), (r) => `${r.message} ${r.path}`)}
          >
            Export all workspaces…
          </button>
          <button
            disabled={busy === "import"}
            onClick={() =>
              action("import", api.importWorkspaces, (r) =>
                r.length === 0
                  ? "Nothing imported."
                  : `Imported ${r.map((w) => `“${w.name}”`).join(", ")}. Imported workspaces must be approved before they can be restored.`,
              )
            }
          >
            Import workspaces…
          </button>
        </div>
        {verify && !verify.ok && (
          <div className="banner banner-bad">
            <div>
              {verify.databaseProblems.map((p) => (
                <div key={p}>{p}</div>
              ))}
              {verify.failures.map((f) => (
                <div key={`${f.workspace}-${f.version}`}>
                  “{f.workspace}” v{f.version} no longer matches its saved fingerprint. It is reported, not
                  repaired: restore from a backup or an export.
                </div>
              ))}
            </div>
          </div>
        )}
      </section>

      <section className="card">
        <h3>Logging</h3>
        <label className="field number-field">
          <span className="field-label">Detail level</span>
          <select value={draft.logLevel} onChange={(e) => set((s) => ({ ...s, logLevel: e.target.value }))}>
            {["error", "warn", "info", "debug", "trace"].map((l) => (
              <option key={l}>{l}</option>
            ))}
          </select>
        </label>
        <p className="muted small">
          Logs never contain window titles, full addresses or your notes. Changing this takes effect the next
          time ContextFlow starts.
        </p>
      </section>

      {retention && (
        <ConfirmDialog
          title="Delete old snapshots?"
          confirmLabel={retention.items.length ? "Delete them" : "Close"}
          danger={retention.items.length > 0}
          action={async () => {
            if (retention.items.length === 0) return;
            const done = await api.applyRetention();
            const n = done.items.reduce((sum, i) => sum + i.versions.length, 0);
            setNotice(`Deleted ${plural(n, "snapshot")}.`);
          }}
          onClose={() => setRetention(undefined)}
        >
          {retention.items.length === 0 ? (
            <p>Nothing to delete: every snapshot is kept by your rules.</p>
          ) : (
            <>
              <p>These snapshots break your rules and would be permanently deleted:</p>
              <ul className="cleanup-confirm">
                {retention.items.map((i) => (
                  <li key={i.workspace}>
                    <strong>{i.workspace}</strong>: {i.versions.map((v) => `v${v}`).join(", ")}
                  </li>
                ))}
              </ul>
              <p className="muted small">This can't be undone.</p>
            </>
          )}
        </ConfirmDialog>
      )}
    </main>
  );
}
