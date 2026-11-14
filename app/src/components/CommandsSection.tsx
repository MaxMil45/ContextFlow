import { Play, Square, Terminal, Trash2 } from "lucide-react";
import { useState, type FormEvent } from "react";

import { useCommand } from "../hooks";
import { api, toCommandError, type CommandSpecDto, type WorkspaceCommandsDto } from "../ipc/client";
import { ErrorBanner, SkeletonText } from "./ui";

/** Split a typed command line into program and arguments, honouring "quotes". */
export function splitCommandLine(line: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  let any = false;
  for (const ch of line) {
    if (ch === '"') {
      quoted = !quoted;
      any = true;
    } else if (/\s/.test(ch) && !quoted) {
      if (any) out.push(cur);
      cur = "";
      any = false;
    } else {
      cur += ch;
      any = true;
    }
  }
  if (any) out.push(cur);
  return out;
}

const show = (c: CommandSpecDto) =>
  [c.program, ...c.args].map((a) => (a.includes(" ") ? `"${a}"` : a)).join(" ");

const SIDES = [
  {
    key: "onEnter" as const,
    title: "When I enter this workspace",
    hint: "Start what you need: a dev server, containers, a VPN. They open in their own window so you can see and stop them.",
    icon: Play,
    waitDefault: false,
  },
  {
    key: "onLeave" as const,
    title: "When I leave this workspace",
    hint: "Stop what you started. They run hidden and are waited for (up to a minute) before anything else closes.",
    icon: Square,
    waitDefault: true,
  },
];

/** A workspace's start and stop commands (in the Focus rules tab). */
export function CommandsSection({ workspace, trusted }: { workspace: string; trusted: boolean }) {
  const loaded = useCommand(() => api.getWorkspaceCommands(workspace), [workspace]);
  const [error, setError] = useState<string>();

  const save = async (next: WorkspaceCommandsDto) => {
    setError(undefined);
    try {
      await api.setWorkspaceCommands(workspace, next);
      loaded.reload();
    } catch (e) {
      setError(toCommandError(e).message);
    }
  };

  return (
    <section className="card">
      <header className="card-header">
        <div>
          <h3>
            <Terminal size={15} aria-hidden /> Start and stop commands
          </h3>
          <p className="muted small">
            Programs ContextFlow runs for you, exactly as written: never through a shell, never as administrator,
            and only for workspaces you created or approved. They stay on this computer and are never exported.
          </p>
        </div>
      </header>
      {!trusted && <p className="muted">Approve this imported workspace before giving it commands.</p>}
      {error && <ErrorBanner error={{ code: "conflict", message: error }} onDismiss={() => setError(undefined)} />}
      {loaded.loading && !loaded.data && <SkeletonText lines={2} />}
      {loaded.data && trusted && (
        <div className="cleanup-sides">
          {SIDES.map((side) => (
            <CommandSide
              key={side.key}
              side={side}
              list={loaded.data![side.key]}
              onChange={(list) => void save({ ...loaded.data!, [side.key]: list })}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function CommandSide({
  side,
  list,
  onChange,
}: {
  side: (typeof SIDES)[number];
  list: CommandSpecDto[];
  onChange: (list: CommandSpecDto[]) => void;
}) {
  const [line, setLine] = useState("");
  const [folder, setFolder] = useState("");
  const [wait, setWait] = useState(side.waitDefault);
  const Icon = side.icon;

  function add(e: FormEvent) {
    e.preventDefault();
    const [program, ...args] = splitCommandLine(line.trim());
    if (!program) return;
    onChange([...list, { program, args, folder: folder.trim() || null, wait }]);
    setLine("");
    setFolder("");
  }

  return (
    <div className="cleanup-side">
      <h4>{side.title}</h4>
      <p className="muted small">{side.hint}</p>
      {list.length === 0 && <p className="muted">No commands.</p>}
      <ul className="cleanup-list">
        {list.map((c, i) => (
          <li key={i} className="command-row">
            <Icon size={13} aria-hidden className="faint" />
            <span className="command-text">
              <code>{show(c)}</code>
              <span className="muted small">
                {c.folder ? `in ${c.folder} · ` : ""}
                {c.wait ? "waited for, hidden" : "own window"}
              </span>
            </span>
            <button
              className="ghost icon"
              aria-label={`Remove ${show(c)}`}
              data-tip="Remove"
              onClick={() => onChange(list.filter((_, j) => j !== i))}
            >
              <Trash2 size={14} />
            </button>
          </li>
        ))}
      </ul>
      <form className="command-form" onSubmit={add}>
        <input
          value={line}
          onChange={(e) => setLine(e.target.value)}
          placeholder={side.key === "onEnter" ? "npm run dev" : "docker compose down"}
          aria-label={`Command to run ${side.key === "onEnter" ? "when entering" : "when leaving"}`}
        />
        <input
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          placeholder="Folder (optional), e.g. C:\work\app"
          aria-label="Folder to run it in"
        />
        <label className="check">
          <input type="checkbox" checked={wait} onChange={(e) => setWait(e.target.checked)} />
          Wait for it to finish
        </label>
        <button type="submit" disabled={!line.trim()}>
          Add command
        </button>
      </form>
    </div>
  );
}
