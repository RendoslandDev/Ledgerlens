import { Modal } from "./Modal";

const groups: { title: string; keys: [string[], string][] }[] = [
  {
    title: "Anywhere",
    keys: [
      [["⌘", "K"], "Search or run a command"],
      [["1"], "Inbox"],
      [["2"], "Ask"],
      [["3"], "Reports"],
      [["4"], "Vendors"],
      [[","], "Settings"],
      [["N"], "Add documents"],
      [["?"], "This list"],
    ],
  },
  {
    title: "Inbox",
    keys: [
      [["J"], "Next document"],
      [["K"], "Previous document"],
      [["A"], "Approve"],
      [["E"], "Edit the vendor"],
      [["/"], "Filter and search text"],
      [["Esc"], "Leave a field without saving"],
    ],
  },
];

export function ShortcutsSheet({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="Keyboard shortcuts" onClose={onClose}>
      <div className="shortcut-groups">
        {groups.map((group) => (
          <section key={group.title}>
            <div className="section-label">
              <span>{group.title}</span>
            </div>
            <dl className="shortcut-list">
              {group.keys.map(([keys, label]) => (
                <div key={label}>
                  <dt>{label}</dt>
                  <dd>
                    {keys.map((key) => (
                      <kbd key={key}>{key}</kbd>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Modal>
  );
}
