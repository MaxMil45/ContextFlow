import type { ReactNode } from "react";

import type { CommandError } from "../ipc/client";
import type { Tone } from "../format";

export function Badge({ tone, children }: { tone: Tone; children: ReactNode }) {
  return <span className={`badge badge-${tone}`}>{children}</span>;
}

export function ErrorBanner({ error, onDismiss }: { error: CommandError; onDismiss?: () => void }) {
  return (
    <div className="banner banner-bad" role="alert">
      <span>{error.message}</span>
      {onDismiss && (
        <button className="link" onClick={onDismiss} aria-label="Dismiss">
          Dismiss
        </button>
      )}
    </div>
  );
}

export function Empty({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <h2>{title}</h2>
      {children}
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
