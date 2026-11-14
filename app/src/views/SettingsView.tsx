import { Database, Globe, KeyRound, RotateCcw, SlidersHorizontal, Wrench, type LucideIcon } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { ConfirmDialog } from "../components/dialogs";
import { ExportDialog, ImportFlow } from "../components/ExportDialogs";
import { Badge, ErrorBanner, Notice, SkeletonText, Spinner } from "../components/ui";
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
import type { HealthCheck } from "../bindings/HealthCheck";

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

/** A key name the shortcut system understands, from a keyboard event's `code`. */
function keyName(code: string, key: string): string | undefined {
  // Some input methods give no key code; the key's own name will do then.
  if (code === "") {
    if (/^[a-zA-Z0-9]$/.test(key)) return key.toUpperCase();
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
    return key === " " ? "Space" : undefined;
  }
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (code === "Space") return "Space";
  return undefined;
}

/**
 * Records a key combination: click it, then press the keys you want. Needs
 * Ctrl, Alt or Shift as well as a letter, number or F-key, so it can't fire
 * while you type.
 */
function HotkeyField({
  value,
  onChange,
  label = "Quick capture shortcut",
  reset = "Ctrl+Alt+X",
}: {
  value: string;
  onChange: (v: string) => void;
  label?: string;
  /** What "Reset" puts back. */
  reset?: string;
}) {
  const [recording, setRecording] = useState(false);
  const [hint, setHint] = useState<string>();

  function onKeyDown(e: React.KeyboardEvent<HTMLButtonElement>) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      setRecording(false);
      setHint(undefined);
      return;
    }
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return; // wait for the real key
    const key = keyName(e.code, e.key);
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
    if (!key) return setHint("Use a letter, a number or an F-key.");
    if (mods.length === 0) return setHint("Hold Ctrl, Alt or Shift too, or it would trigger while you type.");
    onChange([...mods, key].join("+"));
    setRecording(false);
    setHint(undefined);
  }

  return (
    <div className="hotkey">
      <button
        className={`hotkey-box${recording ? " recording" : ""}`}
        onClick={() => setRecording(true)}
        onKeyDown={onKeyDown}
        onBlur={() => {
          setRecording(false);
          setHint(undefined);
        }}
        aria-label={label}
      >
        {recording ? "Press the keys…" : value || "Off"}
      </button>
      <button onClick={() => onChange(reset)} disabled={value === reset}>
        Reset
      </button>
      <button onClick={() => onChange("")} disabled={value === ""}>
        Turn off
      </button>
      {hint && <span className="muted small">{hint}</span>}
    </div>
  );
}

const NAV: Array<["general" | "capture" | "restore" | "data" | "advanced", string, LucideIcon]> = [
  ["general", "General", SlidersHorizontal],
  ["capture", "Capture", Globe],
  ["restore", "Restore", KeyRound],
  ["data", "Your data", Database],
  ["advanced", "Advanced", Wrench],
];

export function SettingsView({ info }: { info: AppInfo | undefined }) {
  const loaded = useCommand(() => api.getSettings(), []);
  const browser = useCommand(() => api.getBrowserStatus(), []);
  const hotkeys = useCommand(() => api.getHotkeyStatus(), []);
  const [draft, setDraft] = useState<Settings>();
  const [saved, setSaved] = useState<Settings>();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<CommandError>();
  const [notice, setNotice] = useState<string>();
  const [retention, setRetention] = useState<RetentionResult>();
  const [verify, setVerify] = useState<VerifyResult>();
  const [busy, setBusy] = useState<string>();
  const [health, setHealth] = useState<HealthCheck[]>();
  const [dataDialog, setDataDialog] = useState<"export" | "import">();
  const [tab, setTab] = useState<"general" | "capture" | "restore" | "data" | "advanced">("general");

  useEffect(() => {
    if (loaded.data) {
      setDraft(loaded.data);
      setSaved(loaded.data);
    }
  }, [loaded.data]);

  if (loaded.error) return <main id="main" tabIndex={-1} className="content"><ErrorBanner error={loaded.error} /></main>;
  if (!draft || !saved) return <main id="main" tabIndex={-1} className="content"><div className="page"><SkeletonText lines={5} /></div></main>;

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
      hotkeys.reload();
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
    <main id="main" tabIndex={-1} className="content settings">
      <div className="page">
        <header className="page-head">
          <h1>Settings</h1>
        </header>

        {error && <ErrorBanner error={error} onDismiss={() => setError(undefined)} />}
        {notice && <Notice onDismiss={() => setNotice(undefined)}>{notice}</Notice>}

        <div className="settings-layout">
          <nav className="settings-nav" aria-label="Settings sections">
            {NAV.map(([key, label, Icon]) => (
              <button key={key} aria-current={tab === key ? "page" : undefined} onClick={() => setTab(key)}>
                <Icon size={15} aria-hidden /> {label}
              </button>
            ))}
          </nav>
          <div className="settings-panels">
      <section className="card" hidden={tab !== "general"}>
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

      <section className="card" hidden={tab !== "general"}>
        <h3>Workspace switcher</h3>
        <p className="muted small">
          Press this from anywhere to open a small picker. Choose a workspace and ContextFlow saves where you
          are, puts it away, and brings the other one back.
        </p>
        <HotkeyField
          value={draft.app.switcherHotkey}
          label="Switcher shortcut"
          reset="Ctrl+Alt+W"
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, switcherHotkey: v } }))}
        />
        {hotkeys.data?.switcher && <Notice tone="warn">{hotkeys.data.switcher}</Notice>}
        <Toggle
          checked={draft.app.autoSaveOnSwitch}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, autoSaveOnSwitch: v } }))}
          hint="Saves a new version of the workspace you're leaving, so coming back finds it as you left it. It is skipped if none of its programs are open, so an emptied desktop never overwrites it."
        >
          Save the workspace I'm leaving
        </Toggle>
        <Toggle
          checked={draft.app.offerResume}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, offerResume: v } }))}
          hint="If Windows restarts or ContextFlow closes without Quit, it offers to bring back the workspace you were in."
        >
          Offer to pick up where I left off after a restart
        </Toggle>
        <label className="field number-field">
          <span>
            <span className="field-label block">Save the workspace I'm in while I work</span>
            <span className="muted small">
              A quiet checkpoint, so a restart or crash loses little. Skipped when nothing changed.
            </span>
          </span>
          <select
            value={draft.app.checkpointMinutes}
            onChange={(e) => set((s) => ({ ...s, app: { ...s.app, checkpointMinutes: Number(e.target.value) } }))}
          >
            {[0, 10, 20, 30, 60].map((m) => (
              <option key={m} value={m}>
                {m === 0 ? "Never" : `Every ${m} minutes`}
              </option>
            ))}
          </select>
        </label>
      </section>

      <section className="card" hidden={tab !== "general"}>
        <h3>Quick capture</h3>
        <p className="muted small">
          Press this from anywhere to save the workspace you're in, without opening ContextFlow. A small
          card appears in the corner of the screen and confirms what was saved. With no active workspace
          it saves into the one you used last.
        </p>
        <HotkeyField
          value={draft.app.captureHotkey}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, captureHotkey: v } }))}
        />
        {hotkeys.data?.capture && <Notice tone="warn">{hotkeys.data.capture}</Notice>}
        <p className="muted small">
          If another program already uses the combination, saving will say so and keep your old one.
        </p>
      </section>

      <section className="card" hidden={tab !== "capture"}>
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
        <ListEditor
          label="Keep these window titles private"
          hint="Windows whose title contains one of these are still saved and restored, but their title is stored as “(hidden)”: client names, documents, banking. Not case-sensitive."
          values={draft.capture.hiddenTitlePatterns}
          placeholder="Bank"
          onChange={(v) => set((s) => ({ ...s, capture: { ...s.capture, hiddenTitlePatterns: v } }))}
        />
        <ListEditor
          label="Always-on apps"
          hint="Apps you use everywhere, like music or chat. They're never saved into a workspace, so switching never moves them, and Focus rules never close them."
          values={draft.capture.alwaysOn}
          placeholder="spotify"
          onChange={(v) => set((s) => ({ ...s, capture: { ...s.capture, alwaysOn: v } }))}
        />
      </section>

      <section className="card" hidden={tab !== "capture"}>
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

      <section className="card" hidden={tab !== "restore"}>
        <h3>Restore</h3>
        <Toggle
          checked={draft.app.matchDisplaySetup}
          onChange={(v) => set((s) => ({ ...s, app: { ...s.app, matchDisplaySetup: v } }))}
          hint="Capture a workspace once docked and once on the laptop screen: each restore uses the layout made for the screens connected now. A pinned version always wins."
        >
          Use the layout saved for the screens I have now
        </Toggle>
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

      <section className="card" hidden={tab !== "data"}>
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

      <section className="card" hidden={tab !== "data"}>
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
            onClick={() => setDataDialog("export")}
          >
            Export all workspaces…
          </button>
          <button
            onClick={() => setDataDialog("import")}
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

      <section className="card" hidden={tab !== "advanced"}>
        <h3>Health check</h3>
        <p className="muted small">
          Looks for the things that most often stop ContextFlow working: the database, backups, the browser
          extension, shortcuts and more. Nothing is sent anywhere.
        </p>
        {health && (
          <ul className="health-list">
            {health.map((c) => (
              <li key={c.name} className={`health-${c.status}`}>
                <span className="health-dot" aria-label={c.status} />
                <span>
                  <strong>{c.name}</strong> <span className="muted">{c.message}</span>
                </span>
              </li>
            ))}
          </ul>
        )}
        <div className="actions left">
          <button
            disabled={busy === "health"}
            onClick={() => action("health", api.healthCheck, (r) => {
              setHealth(r);
              const issues = r.filter((c) => c.status !== "ok").length;
              return issues ? `${issues} thing(s) to look at.` : "Everything looks fine.";
            })}
          >
            {busy === "health" ? "Checking…" : health ? "Check again" : "Run health check"}
          </button>
          <button
            disabled={busy === "diagnostics"}
            onClick={() => action("diagnostics", api.saveDiagnostics, (r) => `${r.message} ${r.path}`)}
          >
            Save diagnostics report…
          </button>
        </div>
      </section>

      <section className="card" hidden={tab !== "advanced"}>
        <h3>About ContextFlow</h3>
        <dl className="paths">
          <dt>Version</dt>
          <dd>{info?.version ?? "…"}</dd>
          <dt>Settings file</dt>
          <dd className="mono">{info?.configPath ?? "…"}</dd>
          <dt>Database</dt>
          <dd className="mono">{info?.databasePath ?? "…"}</dd>
          <dt>Logs</dt>
          <dd className="mono">{info?.logsPath ?? "…"}</dd>
        </dl>
        <p className="muted small">
          Local-first: your workspaces, snapshots and notes never leave this computer. Licensed MIT OR
          Apache-2.0.
        </p>
        <div className="actions left">
          <button onClick={() => api.openFolder("logs").catch((e) => setError(toCommandError(e)))}>Open logs</button>
        </div>
      </section>

      <section className="card" hidden={tab !== "advanced"}>
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

      {dataDialog === "export" && (
        <ExportDialog
          title="Export all workspaces"
          onDone={(r) => setNotice(`${r.message} ${r.path}`)}
          onClose={() => setDataDialog(undefined)}
        />
      )}
      {dataDialog === "import" && (
        <ImportFlow
          onDone={(r) =>
            setNotice(
              r.length === 0
                ? "Nothing imported."
                : `Imported ${r.map((w) => `“${w.name}”`).join(", ")}. Imported workspaces must be approved before they can be restored.`,
            )
          }
          onClose={() => setDataDialog(undefined)}
        />
      )}
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
          </div>
        </div>

        {dirty && (
          <div className="savebar" role="region" aria-label="Unsaved changes">
            <span className="muted">You have unsaved changes.</span>
            <div className="actions">
              <button disabled={saving} onClick={() => setDraft(saved)}>
                <RotateCcw size={14} aria-hidden /> Discard
              </button>
              <button className="primary" disabled={saving} onClick={save}>
                {saving ? "Saving…" : "Save changes"}
              </button>
            </div>
          </div>
        )}
      </div>
    </main>
  );
}
