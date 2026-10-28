import type { DocumentSummary } from "@ledgerlens/bindings";
import { CheckCheck, CircleAlert, Copy, LoaderCircle, ScanText } from "lucide-react";
import { useState } from "react";
import { displayName, queueState, stateLabels, type QueueState } from "../data";
import { money, shortDate } from "../format";

/** The MIME type a card carries while dragged, so file drops and card drops never mix. */
export const CARD_DRAG_TYPE = "application/x-ledgerlens-document";

const columns: { state: QueueState; hint: string }[] = [
  { state: "queued", hint: "New imports wait here while they're read." },
  { state: "review", hint: "Nothing needs checking." },
  { state: "ready", hint: "Documents that validated cleanly land here." },
  { state: "approved", hint: "Drag a reviewed document here to approve it." },
];

/**
 * The inbox as columns by status (Octopus-style cards). Dropping a card on Approved approves
 * it; every other column follows the document's status and is not a drop target.
 */
export function BoardView({
  documents,
  activeId,
  select,
  approveById,
  approveReady,
  snippets,
}: {
  documents: DocumentSummary[];
  activeId: string | null;
  select: (id: string) => void;
  approveById: (id: string) => void;
  approveReady: () => void;
  snippets: Map<string, string>;
}) {
  const [dropping, setDropping] = useState(false);
  const failed = documents.filter((document) => document.status === "failed");
  const visible = failed.length > 0 ? [...columns, { state: "failed" as const, hint: "" }] : columns;

  return (
    <div className="board" role="list" aria-label="Documents by status">
      {visible.map(({ state, hint }) => {
        const cards = documents.filter((document) => queueState(document.status) === state);
        const isApproved = state === "approved";
        return (
          <section
            key={state}
            className={`board-column ${isApproved && dropping ? "is-drop" : ""}`}
            role="listitem"
            aria-label={`${stateLabels[state]}, ${cards.length}`}
            onDragOver={(event) => {
              if (!isApproved || !event.dataTransfer.types.includes(CARD_DRAG_TYPE)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              setDropping(true);
            }}
            onDragLeave={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropping(false);
            }}
            onDrop={(event) => {
              const id = event.dataTransfer.getData(CARD_DRAG_TYPE);
              setDropping(false);
              if (!isApproved || !id) return;
              event.preventDefault();
              event.stopPropagation();
              approveById(id);
            }}
          >
            <header className="board-head">
              <span className={`status-dot ${state}`} aria-hidden="true" />
              <strong>{stateLabels[state]}</strong>
              <span className="board-count mono">{cards.length}</span>
              {state === "ready" && cards.length > 0 && (
                <button className="link-button" onClick={approveReady} title="Approve every ready document">
                  <CheckCheck size={13} aria-hidden="true" /> Approve all
                </button>
              )}
            </header>
            <div className="board-cards">
              {cards.map((document) => (
                <BoardCard
                  key={document.id}
                  document={document}
                  active={document.id === activeId}
                  select={select}
                  snippet={snippets.get(document.id)}
                />
              ))}
              {cards.length === 0 && hint && <p className="board-empty">{hint}</p>}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function BoardCard({
  document,
  active,
  select,
  snippet,
}: {
  document: DocumentSummary;
  active: boolean;
  select: (id: string) => void;
  snippet: string | undefined;
}) {
  const state = queueState(document.status);
  const draggable = state === "review" || state === "ready";
  return (
    <button
      className={`board-card ${active ? "active" : ""}`}
      onClick={() => select(document.id)}
      aria-pressed={active}
      draggable={draggable}
      onDragStart={(event) => {
        event.dataTransfer.setData(CARD_DRAG_TYPE, document.id);
        event.dataTransfer.effectAllowed = "move";
      }}
      title={draggable ? "Drag to Approved to approve" : undefined}
    >
      <span className="board-card-top">
        <strong>{displayName(document)}</strong>
        <span className="mono board-amount">
          {document.totalMinor !== null && document.currency ? money(document.totalMinor, document.currency) : "—"}
        </span>
      </span>
      {snippet ? (
        <small className="snippet">{snippet.replace(/[\u0001\u0002]/g, "")}</small>
      ) : (
        <small>
          {[document.date && shortDate(document.date), document.invoiceNumber].filter(Boolean).join(" · ") ||
            document.fileName}
        </small>
      )}
      <span className="board-card-tags">
        {state === "queued" ? (
          <span className="tag">
            <LoaderCircle size={11} className="spin" aria-hidden="true" />
            {document.status === "processing" ? "Reading" : "Waiting"}
          </span>
        ) : (
          <span className={`tag cat-${categoryTone(document.category ?? document.kind)}`}>
            {document.category ?? capitalize(document.kind)}
          </span>
        )}
        {document.duplicateOf && (
          <span className="tag review">
            <Copy size={11} aria-hidden="true" /> Duplicate?
          </span>
        )}
        {document.textSource === "ocr" && (
          <span className="tag" title="Read with on-device OCR">
            <ScanText size={11} aria-hidden="true" /> OCR
          </span>
        )}
        {state === "review" && document.confidence < 85 && document.confidence > 0 && (
          <span className="tag review" title={`${document.confidence}% confidence`}>
            <CircleAlert size={11} aria-hidden="true" /> {document.confidence}%
          </span>
        )}
      </span>
    </button>
  );
}

/** A stable soft colour per category, like Octopus's labels. */
function categoryTone(label: string): number {
  let hash = 0;
  for (const character of label) hash = (hash * 31 + character.charCodeAt(0)) >>> 0;
  return hash % 5;
}

function capitalize(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}
