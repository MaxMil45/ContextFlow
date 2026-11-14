import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Layers, Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";

const inTauri = "__TAURI_INTERNALS__" in window;

/**
 * The window's own title bar (the native one is turned off), so the app can
 * use one calm surface from edge to edge like other modern Windows apps.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!inTauri) return;
    const win = getCurrentWindow();
    let un: (() => void) | undefined;
    const sync = () => void win.isMaximized().then(setMaximized).catch(() => {});
    sync();
    void win.onResized(sync).then((u) => (un = u));
    return () => un?.();
  }, []);

  const run = (f: (w: ReturnType<typeof getCurrentWindow>) => Promise<unknown>) => () => {
    if (inTauri) void f(getCurrentWindow()).catch(() => {});
  };

  return (
    <header className="titlebar">
      <div className="titlebar-drag" data-tauri-drag-region>
        <Layers size={14} aria-hidden style={{ color: "var(--accent)", pointerEvents: "none" }} />
        <span className="titlebar-title" style={{ pointerEvents: "none" }}>
          ContextFlow
        </span>
      </div>
      <div className="window-controls">
        <button aria-label="Minimize" onClick={run((w) => w.minimize())}>
          <Minus size={14} />
        </button>
        <button aria-label={maximized ? "Restore" : "Maximize"} onClick={run((w) => w.toggleMaximize())}>
          {maximized ? <Copy size={12} /> : <Square size={12} />}
        </button>
        <button className="close" aria-label="Close" onClick={run((w) => w.close())}>
          <X size={15} />
        </button>
      </div>
    </header>
  );
}
