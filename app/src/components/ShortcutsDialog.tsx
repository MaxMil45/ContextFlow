import { Modal } from "./dialogs";
import { Kbd } from "./ui";

const GROUPS: Array<[string, Array<[string, string]>]> = [
  [
    "Anywhere in Windows",
    [
      ["Ctrl Alt W", "Open the workspace switcher (change it in Settings)"],
      ["Ctrl Alt X", "Save the workspace you're in (change it in Settings)"],
    ],
  ],
  [
    "In ContextFlow",
    [
      ["Ctrl K", "Find a workspace or action"],
      ["Ctrl N", "New workspace"],
      ["Ctrl 1–9", "Go to a workspace in the sidebar"],
      ["Ctrl Enter", "Switch to, or restore, the workspace you're looking at"],
      ["F2", "Edit the workspace's name and description"],
      ["Ctrl ,", "Settings"],
      ["Ctrl / or ?", "This list"],
      ["Esc", "Close a dialog or menu"],
    ],
  ],
];

/** Every keyboard shortcut in one place (Ctrl+/ or ?). */
export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="Keyboard shortcuts" onClose={onClose}>
      {GROUPS.map(([title, rows]) => (
        <section key={title} className="shortcut-group">
          <h4>{title}</h4>
          <dl className="shortcut-list">
            {rows.map(([keys, what]) => (
              <div key={keys}>
                <dt>
                  <Kbd>{keys}</Kbd>
                </dt>
                <dd>{what}</dd>
              </div>
            ))}
          </dl>
        </section>
      ))}
      <div className="modal-actions">
        <button className="primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}
