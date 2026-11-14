import { CornerDownLeft } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

export type PaletteCommand = { id: string; label: string; hint?: string; run: () => void };

/** Ctrl+K: type to find a workspace or an action, Enter to run it. */
export function Palette({ commands, onClose }: { commands: PaletteCommand[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    input.current?.focus();
    return () => opener?.focus?.();
  }, []);

  const matches = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return commands;
    return commands.filter((c) => words.every((w) => c.label.toLowerCase().includes(w)));
  }, [commands, query]);

  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    list.current?.children[index]?.scrollIntoView({ block: "nearest" });
  }, [index]);

  const run = (c: PaletteCommand | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };

  return (
    <div
      className="backdrop palette-backdrop"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          else if (e.key === "ArrowDown") {
            e.preventDefault();
            setIndex((i) => Math.min(i + 1, matches.length - 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setIndex((i) => Math.max(i - 1, 0));
          } else if (e.key === "Enter") run(matches[index]);
        }}
      >
        <input
          ref={input}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Find a workspace or action…"
          aria-label="Search workspaces and actions"
          role="combobox"
          aria-expanded
          aria-controls="palette-list"
        />
        <div className="palette-list" id="palette-list" role="listbox" ref={list}>
          {matches.length === 0 && <div className="palette-empty">Nothing matches “{query}”.</div>}
          {matches.map((c, i) => (
            <button
              key={c.id}
              role="option"
              aria-selected={i === index}
              className="palette-item"
              tabIndex={-1}
              onMouseMove={() => setIndex(i)}
              onClick={() => run(c)}
            >
              {c.label}
              {c.hint && <span className="hint">{c.hint}</span>}
            </button>
          ))}
        </div>
        <div className="palette-foot">
          <span>↑↓ to move</span>
          <span>
            <CornerDownLeft size={11} style={{ verticalAlign: "-1px" }} aria-hidden /> to run
          </span>
          <span>Esc to close</span>
        </div>
      </div>
    </div>
  );
}
