import { useEffect, useRef, useState } from "react";

import { invalidateWorkspaceIcon, workspaceColor } from "./ui";
import { useCommand } from "../hooks";
import { api, toCommandError, type IconUpdate } from "../ipc/client";

const ACCEPT = ".png,.jpg,.jpeg,.bmp,.ico";

/**
 * The workspace's desktop-icon picture. Click it to choose a new picture or go
 * back to the standard icon; the desktop shortcut (if any) changes straight away.
 */
export function IconPicker({ workspace, name }: { workspace: string; name: string }) {
  const icon = useCommand(() => api.getWorkspaceIcon(workspace), [workspace]);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string }>();
  const fileInput = useRef<HTMLInputElement>(null);
  const root = useRef<HTMLDivElement>(null);

  // Close the menu on an outside click or Escape.
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

  // Hide the confirmation after a few seconds.
  useEffect(() => {
    if (!status?.ok) return;
    const t = window.setTimeout(() => setStatus(undefined), 6000);
    return () => window.clearTimeout(t);
  }, [status]);

  async function apply(run: () => Promise<IconUpdate>, done: string) {
    setBusy(true);
    setOpen(false);
    setStatus(undefined);
    try {
      const r = await run();
      setStatus({
        ok: true,
        text: r.shortcutPath
          ? `${done} The desktop icon was updated too.`
          : `${done} It will be used when you create the desktop icon.`,
      });
      icon.reload();
      invalidateWorkspaceIcon(workspace);
    } catch (e) {
      setStatus({ ok: false, text: toCommandError(e).message });
    } finally {
      setBusy(false);
    }
  }

  async function onFile(file: File | undefined) {
    if (!file) return;
    const bytes = new Uint8Array(await file.arrayBuffer());
    await apply(() => api.setWorkspaceIcon(workspace, bytes), "Icon changed.");
  }

  const custom = icon.data?.custom ?? false;

  return (
    <div className="icon-picker" ref={root}>
      <button
        className="icon-tile"
        style={custom ? { background: "var(--muted-bg)" } : { background: workspaceColor(workspace) }}
        onClick={() => setOpen((o) => !o)}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        data-tip="Change this workspace's desktop icon"
      >
        {custom && icon.data?.dataUrl ? (
          <img src={icon.data.dataUrl} alt="Workspace icon" />
        ) : (
          <span aria-hidden>{name.trim().charAt(0).toUpperCase() || "?"}</span>
        )}
      </button>
      {open && (
        <div className="menu" role="menu">
          <button role="menuitem" onClick={() => fileInput.current?.click()}>
            Change icon…
          </button>
          <button
            role="menuitem"
            disabled={!custom}
            onClick={() => apply(() => api.resetWorkspaceIcon(workspace), "Back to the standard icon.")}
          >
            Use default icon
          </button>
        </div>
      )}
      <input
        ref={fileInput}
        type="file"
        accept={ACCEPT}
        hidden
        onChange={(e) => {
          void onFile(e.target.files?.[0]);
          e.target.value = ""; // allow picking the same file again
        }}
      />
      {status && (
        <span className={`icon-status ${status.ok ? "ok" : "bad"}`} role="status">
          {status.text}
        </span>
      )}
    </div>
  );
}
