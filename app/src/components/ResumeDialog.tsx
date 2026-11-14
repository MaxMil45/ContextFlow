import { Clock, History } from "lucide-react";

import { relativeTime } from "../format";
import { useCommand } from "../hooks";
import { api, type WorkspaceSummary } from "../ipc/client";
import { Modal } from "./dialogs";
import { SkeletonText } from "./ui";

/** After a restart: offer to bring back the workspace you were in, once. */
export function ResumeDialog({
  workspace,
  onResume,
  onClose,
}: {
  workspace: WorkspaceSummary;
  onResume: () => void;
  onClose: () => void;
}) {
  const snapshot = useCommand(() => api.getSnapshot(workspace.id), [workspace.id]);
  const note = snapshot.data?.notes.at(-1);
  return (
    <Modal title="Pick up where you left off?" onClose={onClose}>
      <div className="modal-body">
        <p>
          ContextFlow didn't close normally last time (Windows restarted, or it stopped unexpectedly). You were in{" "}
          <strong>{workspace.name}</strong>.
        </p>
        {snapshot.loading && !snapshot.data && <SkeletonText lines={2} />}
        {snapshot.data && (
          <section className="resume-card">
            <span className="eyebrow">
              <History size={12} aria-hidden /> Its last save
            </span>
            <p className="resume-note">{note ?? "No note was saved with it."}</p>
            <div className="resume-meta">
              <span>
                <Clock size={13} aria-hidden /> Saved {relativeTime(snapshot.data.createdAt)}
              </span>
              <span>{snapshot.data.applications.length} apps</span>
            </div>
          </section>
        )}
      </div>
      <div className="modal-actions">
        <button onClick={onClose}>Not now</button>
        <button className="primary" onClick={onResume}>
          Bring it back
        </button>
      </div>
    </Modal>
  );
}
