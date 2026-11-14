import {
  AlertTriangle,
  AppWindow,
  CheckCircle2,
  Code2,
  FolderGit2,
  Globe,
  MessageSquare,
  Music2,
  Terminal,
  X,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import {
  Component,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ErrorInfo,
  type ReactNode,
} from "react";

import type { Tone } from "../format";
import type { CommandError } from "../ipc/client";

// ------------------------------------------------------------ small pieces

export function Badge({
  tone,
  icon: Icon,
  children,
}: {
  tone: Tone | "accent";
  icon?: LucideIcon;
  children: ReactNode;
}) {
  return (
    <span className={`badge badge-${tone}`}>
      {Icon && <Icon aria-hidden />}
      {children}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd>{children}</kbd>;
}

export function ErrorBanner({ error, onDismiss }: { error: CommandError; onDismiss?: () => void }) {
  return (
    <div className="banner banner-bad" role="alert">
      <span>
        <XCircle size={16} aria-hidden />
        {error.message}
      </span>
      {onDismiss && (
        <button className="ghost icon" onClick={onDismiss} aria-label="Dismiss" data-tip="Dismiss">
          <X size={14} />
        </button>
      )}
    </div>
  );
}

export function Notice({
  tone = "ok",
  children,
  onDismiss,
}: {
  tone?: "ok" | "warn" | "muted";
  children: ReactNode;
  onDismiss?: () => void;
}) {
  const Icon = tone === "warn" ? AlertTriangle : CheckCircle2;
  return (
    <div className={`banner banner-${tone}`} role="status">
      <span>
        {tone !== "muted" && <Icon size={16} aria-hidden />}
        {children}
      </span>
      {onDismiss && (
        <button className="ghost icon" onClick={onDismiss} aria-label="Dismiss" data-tip="Dismiss">
          <X size={14} />
        </button>
      )}
    </div>
  );
}

export function Empty({
  title,
  icon: Icon,
  children,
  actions,
}: {
  title: string;
  icon?: LucideIcon;
  children?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="empty">
      {Icon && (
        <div className="empty-icon" aria-hidden>
          <Icon size={26} />
        </div>
      )}
      <h2>{title}</h2>
      {children && <p className="muted">{children}</p>}
      {actions && <div className="actions">{actions}</div>}
    </div>
  );
}

export function Spinner({ label }: { label?: string }) {
  return (
    <span className="spinner" role="status">
      <span className="spinner-dot" aria-hidden />
      {label}
    </span>
  );
}

/** Placeholder rows while a list loads: calmer than a spinner and keeps the layout steady. */
export function SkeletonList({ rows = 4 }: { rows?: number }) {
  return (
    <div aria-busy="true" aria-label="Loading">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="skeleton skeleton-row" />
      ))}
    </div>
  );
}

export function SkeletonText({ lines = 3 }: { lines?: number }) {
  return (
    <div aria-busy="true" aria-label="Loading">
      {Array.from({ length: lines }, (_, i) => (
        <div key={i} className="skeleton skeleton-line" style={{ width: `${90 - i * 18}%` }} />
      ))}
    </div>
  );
}

// ------------------------------------------------------------ identity

/** A stable colour per workspace, so each one is recognisable at a glance. */
export function workspaceColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return `hsl(${h % 360} 52% 50%)`;
}

export function WorkspaceDot({ id, name }: { id: string; name: string }) {
  return (
    <span className="ws-dot" style={{ background: workspaceColor(id) }} aria-hidden>
      {name.trim().charAt(0) || "?"}
    </span>
  );
}

const APP_ICONS: Array<[RegExp, LucideIcon]> = [
  [/^(chrome|msedge|firefox|brave|opera|vivaldi|arc)$/, Globe],
  [/^(code|code - insiders|cursor|windsurf|devenv|idea64|pycharm64|rider64|webstorm64)$/, Code2],
  [/^(windowsterminal|wt|powershell|pwsh|cmd|wezterm|alacritty|warp|conhost)$/, Terminal],
  [/^(discord|slack|teams|ms-teams|telegram|whatsapp|claude)$/, MessageSquare],
  [/^(spotify|vlc|musicbee)$/, Music2],
  [/^(github desktop|githubdesktop|gitkraken|sourcetree)$/, FolderGit2],
];

export function appIcon(applicationId: string): LucideIcon {
  const id = applicationId.toLowerCase();
  return APP_ICONS.find(([re]) => re.test(id))?.[1] ?? AppWindow;
}

export function AppIcon({ applicationId }: { applicationId: string }) {
  const Icon = appIcon(applicationId);
  return (
    <span className="app-icon" aria-hidden>
      <Icon size={16} />
    </span>
  );
}

// ------------------------------------------------------------ menus

/** A dropdown that closes on outside click or Escape and supports arrow keys. */
export function Menu({
  label,
  trigger,
  children,
  align = "right",
}: {
  label: string;
  trigger: ReactNode;
  children: (close: () => void) => ReactNode;
  align?: "left" | "right";
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!open) return;
    root.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus();
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        root.current?.querySelector<HTMLElement>("button")?.focus();
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const items = [...(root.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])];
        if (!items.length) return;
        const at = items.indexOf(document.activeElement as HTMLElement);
        const next = e.key === "ArrowDown" ? (at + 1) % items.length : (at - 1 + items.length) % items.length;
        items[next]?.focus();
        e.preventDefault();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div className="menu-wrap" ref={root}>
      <button
        className="ghost icon"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label}
        data-tip={label}
        onClick={() => setOpen((o) => !o)}
      >
        {trigger}
      </button>
      {open && (
        <div className={`menu${align === "right" ? " menu-right" : ""}`} role="menu">
          {children(close)}
        </div>
      )}
    </div>
  );
}

export function MenuItem({
  icon: Icon,
  danger,
  hint,
  onSelect,
  close,
  children,
}: {
  icon?: LucideIcon;
  danger?: boolean;
  hint?: string;
  onSelect: () => void;
  close: () => void;
  children: ReactNode;
}) {
  return (
    <button
      role="menuitem"
      className={danger ? "danger" : undefined}
      onClick={() => {
        close();
        onSelect();
      }}
    >
      {Icon && <Icon size={15} aria-hidden />}
      {children}
      {hint && <kbd className="menu-kbd">{hint}</kbd>}
    </button>
  );
}

// ------------------------------------------------------------ toasts

type ToastTone = "ok" | "bad";
type ToastEntry = { id: number; text: string; tone: ToastTone; action?: { label: string; run: () => void } };
type ToastApi = {
  show: (text: string, opts?: { tone?: ToastTone; action?: ToastEntry["action"]; ms?: number }) => void;
};

const ToastContext = createContext<ToastApi>({ show: () => {} });
export const useToast = () => useContext(ToastContext);

/** Short-lived messages (saved, copied, undo) instead of banners and browser alerts. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastEntry[]>([]);
  const next = useRef(0);

  const dismiss = useCallback((id: number) => setItems((l) => l.filter((t) => t.id !== id)), []);
  const show = useCallback<ToastApi["show"]>(
    (text, opts = {}) => {
      const id = ++next.current;
      setItems((l) => [...l.slice(-3), { id, text, tone: opts.tone ?? "ok", action: opts.action }]);
      window.setTimeout(() => dismiss(id), opts.ms ?? (opts.action ? 7000 : 4000));
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={{ show }}>
      {children}
      <div className="toaster" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`toast-item ${t.tone}`} role="status">
            {t.tone === "ok" ? <CheckCircle2 size={16} aria-hidden /> : <XCircle size={16} aria-hidden />}
            <span className="toast-text">{t.text}</span>
            {t.action && (
              <button
                className="link"
                onClick={() => {
                  t.action!.run();
                  dismiss(t.id);
                }}
              >
                {t.action.label}
              </button>
            )}
            <button className="ghost icon" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <X size={14} />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

// ------------------------------------------------------------ error boundary

/** One render error must never leave a blank window. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error?: Error }> {
  state: { error?: Error } = {};

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ContextFlow UI error", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="crash">
        <Empty
          icon={AlertTriangle}
          title="Something went wrong in the window"
          actions={
            <>
              <button className="primary" onClick={() => window.location.reload()}>
                Reload
              </button>
            </>
          }
        >
          Your workspaces and snapshots are safe; this only affects what is on screen. Reloading usually fixes it.
        </Empty>
        <pre>{this.state.error.message}</pre>
      </div>
    );
  }
}
