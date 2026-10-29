import type {
  Activity,
  DocumentDetail,
  DocumentSummary,
  FieldEdit,
  WatchStatus,
} from "@ledgerlens/bindings";
import {
  CheckCheck,
  Columns3,
  Eye,
  EyeOff,
  FolderInput,
  Inbox,
  List,
  LoaderCircle,
  Search,
  Upload,
} from "lucide-react";
import { type RefObject } from "react";
import { ConfidenceMeter, StatusDot } from "../components/Status";
import { displayName, queueState, type QueueState } from "../data";
import { folderName, money, shortDate } from "../format";
import { BoardView } from "./BoardView";
import { ReviewPanel } from "./ReviewPanel";

export type QueueFilter = "all" | QueueState;
export type InboxLayout = "table" | "board";
export type SortKey = "newest" | "oldest" | "amount" | "vendor" | "confidence";

export const sortLabels: Record<SortKey, string> = {
  newest: "Newest",
  oldest: "Oldest",
  amount: "Largest amount",
  vendor: "Vendor A–Z",
  confidence: "Least confident",
};

/** Renders a search excerpt, highlighting the U+0001 … U+0002 marked terms without HTML. */
function Snippet({ text }: { text: string }) {
  const parts = text.split(/(\u0001[^\u0002]*\u0002)/);
  return (
    <span className="snippet">
      {parts.map((part, index) =>
        part.startsWith("\u0001") ? <mark key={index}>{part.slice(1, -1)}</mark> : part,
      )}
    </span>
  );
}

const filterLabels: { id: QueueFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "review", label: "Needs review" },
  { id: "ready", label: "Ready" },
  { id: "approved", label: "Approved" },
];

export function InboxView({
  documents,
  filteredDocuments,
  activeId,
  filter,
  setFilter,
  search,
  setSearch,
  searchRef,
  select,
  detail,
  loadingDetail,
  categories,
  update,
  approve,
  skip,
  reprocess,
  remove,
  cancelImport,
  layout,
  setLayout,
  approveById,
  cancelPending,
  activity,
  watch,
  openWatchSettings,
  openOriginal,
  vendorRef,
  pro,
  dragging,
  setDragging,
  importFiles,
  chooseFiles,
  chooseFolder,
  busy,
  sort,
  setSort,
  approveReady,
  snippets,
}: {
  documents: DocumentSummary[];
  filteredDocuments: DocumentSummary[];
  activeId: string | null;
  filter: QueueFilter;
  setFilter: (filter: QueueFilter) => void;
  search: string;
  setSearch: (search: string) => void;
  searchRef: RefObject<HTMLInputElement | null>;
  select: (id: string) => void;
  detail: DocumentDetail | null;
  loadingDetail: boolean;
  categories: string[];
  update: (edit: FieldEdit) => Promise<boolean>;
  approve: () => void;
  skip: () => void;
  reprocess: () => void;
  remove: () => void;
  cancelImport: () => void;
  layout: InboxLayout;
  setLayout: (layout: InboxLayout) => void;
  approveById: (id: string) => void;
  cancelPending: () => void;
  activity: Activity | null;
  watch: WatchStatus | null;
  openWatchSettings: () => void;
  openOriginal: (() => void) | null;
  vendorRef: RefObject<HTMLInputElement | null>;
  /** Statements and contracts are read with their own rules (Ledgerlens Pro). */
  pro: boolean;
  dragging: boolean;
  setDragging: (dragging: boolean) => void;
  importFiles: (files: FileList | File[]) => void;
  chooseFiles: () => void;
  chooseFolder: (() => void) | null;
  busy: string | null;
  sort: SortKey;
  setSort: (sort: SortKey) => void;
  approveReady: () => void;
  snippets: Map<string, string>;
}) {
  const counts: Record<QueueFilter, number> = {
    all: documents.length,
    review: 0,
    ready: 0,
    approved: 0,
    queued: 0,
    failed: 0,
  };
  documents.forEach((document) => (counts[queueState(document.status)] += 1));

  return (
    <section className="inbox-layout">
      <div
        className={`queue ${dragging ? "is-dragging" : ""}`}
        onDragOver={(event) => {
          // Only files from outside; a board card being dragged is not an import.
          if (!event.dataTransfer.types.includes("Files")) return;
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget as Node | null))
            setDragging(false);
        }}
        onDrop={(event) => {
          if (!event.dataTransfer.types.includes("Files")) return;
          event.preventDefault();
          setDragging(false);
          importFiles(event.dataTransfer.files);
        }}
      >
        <div className="queue-toolbar">
          <div className="segmented view-switch" role="radiogroup" aria-label="Inbox layout">
            <button
              role="radio"
              aria-checked={layout === "table"}
              className={layout === "table" ? "selected" : ""}
              onClick={() => setLayout("table")}
              title="Table"
            >
              <List size={14} aria-hidden="true" />
              <span className="view-switch-label">Table</span>
            </button>
            <button
              role="radio"
              aria-checked={layout === "board"}
              className={layout === "board" ? "selected" : ""}
              onClick={() => setLayout("board")}
              title="Board"
            >
              <Columns3 size={14} aria-hidden="true" />
              <span className="view-switch-label">Board</span>
            </button>
          </div>
          {layout === "table" && (
            <div className="segmented" role="tablist" aria-label="Filter documents">
              {filterLabels.map((item) => (
                <button
                  key={item.id}
                  role="tab"
                  aria-selected={filter === item.id}
                  className={filter === item.id ? "selected" : ""}
                  onClick={() => setFilter(item.id)}
                >
                  {item.label}
                  <span className="mono">{counts[item.id]}</span>
                </button>
              ))}
              {counts.failed > 0 && (
                <button
                  role="tab"
                  aria-selected={filter === "failed"}
                  className={filter === "failed" ? "selected" : ""}
                  onClick={() => setFilter("failed")}
                >
                  Unreadable <span className="mono">{counts.failed}</span>
                </button>
              )}
            </div>
          )}
          <label className="filter-input">
            <Search size={14} aria-hidden="true" />
            <input
              ref={searchRef}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") event.currentTarget.blur();
              }}
              placeholder="Search"
              aria-label="Filter records"
            />
            <kbd>/</kbd>
          </label>
          <select
            className="sort-select"
            value={sort}
            onChange={(event) => setSort(event.target.value as SortKey)}
            aria-label="Sort documents"
          >
            {(Object.keys(sortLabels) as SortKey[]).map((key) => (
              <option key={key} value={key}>
                {sortLabels[key]}
              </option>
            ))}
          </select>
          {counts.ready > 0 && (
            <button
              className="button"
              onClick={approveReady}
              title="Approve every document that validated cleanly"
            >
              <CheckCheck size={14} aria-hidden="true" /> Approve {counts.ready} ready
            </button>
          )}
        </div>

        {busy && <div className="progress-line" role="status">{busy}</div>}
        {!busy && activity && (activity.queued > 0 || activity.reading) && (
          <div className="progress-line reading-line" role="status">
            <LoaderCircle size={13} className="spin" aria-hidden="true" />
            <span>
              {activity.reading ? "Reading 1 document" : "Starting"}
              {activity.queued > 0 && ` · ${activity.queued} waiting`}
            </span>
            {activity.queued > 0 && (
              <button className="link-button" onClick={cancelPending}>
                Cancel waiting
              </button>
            )}
          </div>
        )}
        {watch?.folder && (
          <button
            className={`watch-chip ${watch.enabled ? "" : "paused"}`}
            onClick={openWatchSettings}
            title={watch.folder}
          >
            {watch.enabled ? <Eye size={13} aria-hidden="true" /> : <EyeOff size={13} aria-hidden="true" />}
            {watch.enabled ? "Watching" : "Paused"} <strong>{folderName(watch.folder)}</strong>
            {watch.lastScan?.error && <span className="tag failed">Can't read folder</span>}
          </button>
        )}

        {layout === "board" && documents.length > 0 ? (
          <BoardView
            documents={filteredDocuments}
            activeId={activeId}
            select={select}
            approveById={approveById}
            approveReady={approveReady}
            snippets={snippets}
          />
        ) : (
          <div className="table" role="grid" aria-label="Documents">
            {documents.length > 0 && (
              <div className="table-head" role="row">
                <span />
                <span>Vendor</span>
                <span>Number</span>
                <span className="col-category">Category</span>
                <span>Date</span>
                <span className="num">Amount</span>
                <span className="col-confidence">Conf.</span>
              </div>
            )}
            {filteredDocuments.map((document) => (
              <button
                key={document.id}
                role="row"
                data-record={document.id}
                className={`table-row ${document.id === activeId ? "active" : ""}`}
                onClick={() => select(document.id)}
                aria-selected={document.id === activeId}
              >
                <StatusDot state={queueState(document.status)} />
                <span className="cell-vendor">
                  <strong>{displayName(document)}</strong>
                  {snippets.has(document.id) ? (
                    <Snippet text={snippets.get(document.id) ?? ""} />
                  ) : (
                    document.vendor && <small>{document.fileName}</small>
                  )}
                  {document.duplicateOf && <span className="tag review">Duplicate?</span>}
                </span>
                <span className="mono dim">{document.invoiceNumber ?? "—"}</span>
                <span className="col-category dim">{document.category ?? "—"}</span>
                <span className="mono dim">{document.date ? shortDate(document.date) : "—"}</span>
                <span className="mono num">
                  {document.totalMinor !== null && document.currency
                    ? money(document.totalMinor, document.currency)
                    : "—"}
                </span>
                <span className="col-confidence">
                  <ConfidenceMeter value={document.confidence} />
                </span>
              </button>
            ))}
            {documents.length === 0 && (
              <div className="empty-state first-run">
                <Inbox size={22} aria-hidden="true" />
                <strong>No documents yet</strong>
                <span>
                  Drop invoices or receipts here, or add them from your computer. PDFs, scans, and
                  photos are read on this device.
                </span>
                <span className="first-run-actions">
                  <button className="button button-primary" onClick={chooseFiles}>
                    <Upload size={14} aria-hidden="true" /> Add documents
                  </button>
                  {chooseFolder && (
                <button className="button" onClick={chooseFolder}>
                    <FolderInput size={14} aria-hidden="true" /> Import a folder
                  </button>
                )}
                </span>
              </div>
            )}
            {documents.length > 0 && filteredDocuments.length === 0 && (
              <div className="empty-state">
                <Search size={18} aria-hidden="true" />
                <strong>No matching documents</strong>
                <span>Search looks at vendors, numbers, categories, file names, and document text.</span>
              </div>
            )}
          </div>
        )}

        {dragging && (
          <div className="drop-overlay">
            <Upload size={20} aria-hidden="true" />
            <strong>Drop to add documents</strong>
            <span>PDF, PNG, JPG, or TIFF · up to 25 MB</span>
          </div>
        )}
      </div>

      {detail ? (
        <ReviewPanel
          key={detail.summary.id}
          detail={detail}
          categories={categories}
          update={update}
          approve={approve}
          skip={skip}
          reprocess={reprocess}
          remove={remove}
          cancelImport={cancelImport}
          openOriginal={openOriginal}
          openDocument={select}
          vendorRef={vendorRef}
          pro={pro}
        />
      ) : (
        <aside className="review blank-review">
          <strong>{loadingDetail ? "Loading…" : "Nothing selected"}</strong>
          {!loadingDetail && <span>Pick a row, or press J to start at the top.</span>}
        </aside>
      )}
    </section>
  );
}
