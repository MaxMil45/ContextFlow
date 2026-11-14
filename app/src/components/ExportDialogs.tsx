import { Lock } from "lucide-react";
import { useState } from "react";

import { api, type FileResult, type ImportedWorkspace } from "../ipc/client";
import { ConfirmDialog } from "./dialogs";

const MIN = 8;

/** Export, optionally protected with a passphrase. */
export function ExportDialog({
  workspaces,
  title,
  onDone,
  onClose,
}: {
  /** Ids to export; `undefined` = every workspace. */
  workspaces?: string[];
  title: string;
  onDone: (r: FileResult) => void;
  onClose: () => void;
}) {
  const [protect, setProtect] = useState(true);
  const [pass, setPass] = useState("");
  const [again, setAgain] = useState("");
  const problem = !protect
    ? undefined
    : pass.length < MIN
      ? `At least ${MIN} characters.`
      : pass !== again
        ? "The two passphrases don't match."
        : undefined;
  return (
    <ConfirmDialog
      title={title}
      confirmLabel={protect ? "Choose where to save…" : "Export without protection…"}
      disabled={!!problem}
      action={async () => {
        const r = await api.exportWorkspaces(workspaces, protect ? pass : undefined);
        if (r) onDone(r);
      }}
      onClose={onClose}
    >
      <p>
        The file holds window titles, folder paths, tab addresses and your notes, with every saved version.
      </p>
      <label className="check">
        <input type="checkbox" checked={protect} onChange={(e) => setProtect(e.target.checked)} />
        <span>
          <Lock size={13} aria-hidden /> Protect it with a passphrase (recommended if it leaves this computer)
        </span>
      </label>
      {protect && (
        <>
          <label className="field">
            <span className="field-label">Passphrase</span>
            <input className="wide" type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoComplete="new-password" />
          </label>
          <label className="field">
            <span className="field-label">Type it again</span>
            <input className="wide" type="password" value={again} onChange={(e) => setAgain(e.target.value)} autoComplete="new-password" />
          </label>
          <p className={`small ${problem ? "warn-text" : "muted"}`}>
            {problem ?? "Keep it safe: without it, nobody (including you) can open the file."}
          </p>
        </>
      )}
    </ConfirmDialog>
  );
}

/** Import, asking for the passphrase if the chosen file is protected. */
export function ImportFlow({
  onDone,
  onClose,
}: {
  onDone: (imported: ImportedWorkspace[]) => void;
  onClose: () => void;
}) {
  const [path, setPath] = useState<string>();
  const [pass, setPass] = useState("");

  // First step: pick the file (the Windows dialog opens straight away).
  if (!path) {
    return (
      <ConfirmDialog
        title="Import workspaces"
        confirmLabel="Choose a file…"
        action={async () => {
          const r = await api.importWorkspaces();
          if (!r) return;
          if (r.needsPassphrase) {
            setPath(r.needsPassphrase);
            throw { code: "conflict", message: "This file is protected: enter its passphrase." };
          }
          onDone(r.imported);
        }}
        onClose={onClose}
      >
        <p>Imported workspaces arrive unapproved: review the programs they would start before restoring them.</p>
      </ConfirmDialog>
    );
  }
  return (
    <ConfirmDialog
      title="This export is protected"
      confirmLabel="Import"
      disabled={!pass}
      action={async () => {
        const r = await api.importWorkspaces(path, pass);
        if (r) onDone(r.imported);
      }}
      onClose={onClose}
    >
      <label className="field">
        <span className="field-label">Passphrase</span>
        <input className="wide" type="password" value={pass} onChange={(e) => setPass(e.target.value)} />
      </label>
    </ConfirmDialog>
  );
}
