import { CornerDownLeft, FileText, Search } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { DocumentSummary } from "@ledgerlens/bindings";
import { displayName, queueState, stateLabels } from "../data";
import { money } from "../format";

export interface Command {
  id: string;
  label: string;
  icon: ReactNode;
  shortcut?: string;
  run: () => void;
}

type Entry =
  | { kind: "command"; command: Command }
  | { kind: "record"; record: DocumentSummary };

export function CommandPalette({
  commands,
  records,
  openRecord,
  close,
}: {
  commands: Command[];
  records: DocumentSummary[];
  openRecord: (id: string) => void;
  close: () => void;
}) {
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const entries = useMemo<Entry[]>(() => {
    const needle = query.trim().toLowerCase();
    const matchedCommands = commands
      .filter((command) => !needle || command.label.toLowerCase().includes(needle))
      .map((command) => ({ kind: "command" as const, command }));
    const matchedRecords = records
      .filter(
        (record) =>
          needle &&
          [record.vendor, record.invoiceNumber, record.category, record.fileName].some(
            (value) => value?.toLowerCase().includes(needle),
          ),
      )
      .slice(0, 8)
      .map((record) => ({ kind: "record" as const, record }));
    return [...matchedRecords, ...matchedCommands];
  }, [commands, records, query]);

  useEffect(() => setCursor(0), [query]);
  useEffect(() => {
    listRef.current
      ?.querySelector<HTMLElement>(`[data-index="${cursor}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  function choose(entry: Entry | undefined) {
    if (!entry) return;
    close();
    if (entry.kind === "command") entry.command.run();
    else openRecord(entry.record.id);
  }

  const firstCommand = entries.findIndex((entry) => entry.kind === "command");

  return (
    <div className="palette-backdrop" onMouseDown={close}>
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="palette-input">
          <Search size={16} aria-hidden="true" />
          <input
            autoFocus
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search records or run a command…"
            aria-label="Command or record"
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setCursor((value) => Math.min(value + 1, entries.length - 1));
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setCursor((value) => Math.max(value - 1, 0));
              } else if (event.key === "Enter") {
                event.preventDefault();
                choose(entries[cursor]);
              } else if (event.key === "Escape") {
                event.preventDefault();
                close();
              }
            }}
          />
          <kbd>esc</kbd>
        </div>
        <div className="palette-list" ref={listRef}>
          {entries.length === 0 && (
            <div className="palette-empty">No records or commands match.</div>
          )}
          {entries.map((entry, index) => (
            <div key={entry.kind === "command" ? entry.command.id : entry.record.id}>
              {index === 0 && entry.kind === "record" && (
                <div className="palette-group">Records</div>
              )}
              {index === firstCommand && <div className="palette-group">Commands</div>}
              <button
                data-index={index}
                className={`palette-item ${index === cursor ? "active" : ""}`}
                onMouseMove={() => setCursor(index)}
                onClick={() => choose(entry)}
              >
                {entry.kind === "command" ? (
                  <>
                    <span className="palette-icon">{entry.command.icon}</span>
                    <span className="palette-label">{entry.command.label}</span>
                    {entry.command.shortcut && <kbd>{entry.command.shortcut}</kbd>}
                  </>
                ) : (
                  <>
                    <span className="palette-icon">
                      <FileText size={15} />
                    </span>
                    <span className="palette-label">
                      {displayName(entry.record)}
                      <small>
                        {entry.record.invoiceNumber ?? entry.record.fileName} ·{" "}
                        {stateLabels[queueState(entry.record.status)]}
                      </small>
                    </span>
                    <span className="mono palette-amount">
                      {entry.record.totalMinor !== null && entry.record.currency
                        ? money(entry.record.totalMinor, entry.record.currency)
                        : "—"}
                    </span>
                  </>
                )}
                {index === cursor && (
                  <CornerDownLeft size={13} className="palette-enter" aria-hidden="true" />
                )}
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
