import { useEffect, useState } from "react";

import { api, type ToastMessage } from "../ipc/client";

/**
 * The small confirmation card shown in the corner of the screen after the
 * quick-capture shortcut. It lives in its own transparent, click-through window.
 */
export function Toast() {
  const [msg, setMsg] = useState<ToastMessage>();

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    api.onToast(setMsg).then((u) => (disposed ? u() : (unlisten = u)));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  if (!msg) return null;
  return (
    <div className={`toast toast-${msg.kind}`} role="status" aria-live="polite">
      <div className="toast-icon" aria-hidden>
        {msg.kind === "saved" ? "✓" : msg.kind === "error" ? "!" : <span className="spinner-dot" />}
      </div>
      <div>
        <div className="toast-title">{msg.title}</div>
        {msg.body && <div className="toast-body">{msg.body}</div>}
      </div>
    </div>
  );
}
