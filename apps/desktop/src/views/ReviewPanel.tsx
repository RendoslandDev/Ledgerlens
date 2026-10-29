import type {
  DocKind,
  DocumentDetail,
  FieldEdit,
  FieldName,
  FieldView,
} from "@ledgerlens/bindings";
import {
  CalendarClock,
  Check,
  ChevronDown,
  Copy,
  CircleAlert,
  ExternalLink,
  FileWarning,
  LoaderCircle,
  RefreshCw,
  ScanText,
  SkipForward,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState, type RefObject } from "react";
import { fileBytes } from "../api";
import { ConfidenceMeter, StatusDot } from "../components/Status";
import {
  currencies,
  displayName,
  editableFields,
  fieldLabel,
  fieldLabels,
  queueState,
  requiredFields,
  stateLabels,
} from "../data";
import { fileSize, money, timeAgo } from "../format";

const kinds: DocKind[] = ["invoice", "receipt", "statement", "contract", "other"];
const moneyFields: FieldName[] = ["subtotal", "tax", "total", "opening_balance", "closing_balance"];
const dateFields: FieldName[] = ["doc_date", "due_date", "period_start", "period_end", "end_date"];

export function ReviewPanel({
  detail,
  categories,
  update,
  approve,
  skip,
  reprocess,
  remove,
  cancelImport,
  openOriginal,
  openDocument,
  vendorRef,
}: {
  detail: DocumentDetail;
  categories: string[];
  update: (edit: FieldEdit) => Promise<boolean>;
  approve: () => void;
  skip: () => void;
  reprocess: () => void;
  remove: () => void;
  cancelImport: () => void;
  openOriginal: (() => void) | null;
  openDocument: (id: string) => void;
  vendorRef: RefObject<HTMLInputElement | null>;
}) {
  const { summary } = detail;
  const state = queueState(summary.status);
  const [showSource, setShowSource] = useState(true);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const fieldByName = new Map(detail.fields.map((field) => [field.name, field]));
  const currency = summary.currency ?? "USD";
  const canApprove = state !== "approved" && state !== "queued" && summary.kind !== "other";
  const isInvoice = summary.kind === "invoice" || summary.kind === "receipt" || summary.kind === "other";

  useEffect(() => {
    if (!confirmDelete) return;
    const timer = window.setTimeout(() => setConfirmDelete(false), 3000);
    return () => window.clearTimeout(timer);
  }, [confirmDelete]);

  return (
    <aside className="review" aria-label="Record review">
      <header className="review-head">
        <div className="review-title">
          <StatusDot state={state} />
          <div>
            <h2>{displayName(summary)}</h2>
            <small className="mono">
              {summary.fileName} · {fileSize(summary.sizeBytes)}
            </small>
          </div>
        </div>
        <span className={`tag ${state}`}>{stateLabels[state]}</span>
      </header>

      <div className="review-scroll">
        {state === "queued" && (
          <div className="notice reading" role="status">
            <LoaderCircle size={15} className="spin" aria-hidden="true" />
            <span>
              {summary.status === "processing"
                ? "Reading this document now. Fields fill in when it's done."
                : "Waiting to be read. Documents are read one at a time, in the order they were added."}
            </span>
          </div>
        )}
        {summary.note && (
          <div className={`notice ${state === "failed" ? "error" : ""}`}>
            {summary.textSource === "ocr" ? (
              <ScanText size={15} aria-hidden="true" />
            ) : (
              <FileWarning size={15} aria-hidden="true" />
            )}
            <span>{summary.note}</span>
          </div>
        )}
        {summary.noticeBy && (
          <div className="notice">
            <CalendarClock size={15} aria-hidden="true" />
            <span>
              To stop it {fieldByName.get("auto_renew")?.value === "yes" ? "renewing" : "running on"}, give
              notice by <strong className="mono">{summary.noticeBy}</strong>.
            </span>
          </div>
        )}
        {detail.issues.length > 0 && (
          <div className="notice warn">
            <CircleAlert size={15} aria-hidden="true" />
            <ul className="issue-list">
              {detail.issues.map((issue) => (
                <li key={issue.message}>{issue.message}</li>
              ))}
            </ul>
          </div>
        )}

        <section className="review-section">
          <div className="section-label">
            <span>Fields</span>
            <span className="section-meta">
              {summary.textSource === "ocr"
                ? "OCR · on-device"
                : summary.textSource === "text_layer"
                  ? "Text layer"
                  : "Manual"}
              <ConfidenceMeter value={summary.confidence} />
            </span>
          </div>
          <div className="fields">
            <label className="field">
              <span>Type</span>
              <select
                value={summary.kind}
                onChange={(event) => update({ field: "kind", value: event.target.value })}
              >
                {kinds.map((kind) => (
                  <option key={kind} value={kind}>
                    {kind[0].toUpperCase() + kind.slice(1)}
                  </option>
                ))}
              </select>
            </label>
            {editableFields(summary.kind).map((name) => (
              <FieldInput
                key={name}
                name={name}
                label={fieldLabel(name, summary.kind)}
                required={requiredFields(summary.kind).includes(name)}
                field={fieldByName.get(name)}
                currency={currency}
                categories={categories}
                update={update}
                inputRef={name === "vendor" || name === "parties" ? vendorRef : undefined}
              />
            ))}
          </div>
          <p className="field-hint">Changes save when you leave a field or press Enter.</p>
        </section>

        {summary.kind === "statement" && (
          <section className="review-section">
            <div className="section-label">
              <span>Transactions</span>
              <span className="section-meta mono">{detail.transactions.length}</span>
            </div>
            {detail.transactions.length ? (
              <div className="line-items">
                {detail.transactions.map((row, index) => (
                  <div className="line-item transaction" key={`${index}-${row.date}`}>
                    <span className="mono dim">{row.date}</span>
                    <span>{row.description}</span>
                    <span className={`mono num ${row.amountMinor < 0 ? "" : "credit"}`}>
                      {row.amountMinor > 0 ? "+" : ""}
                      {money(row.amountMinor, row.currency)}
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <p className="muted-copy">No transactions were found.</p>
            )}
          </section>
        )}

        {isInvoice && <section className="review-section">
          <div className="section-label">
            <span>Line items</span>
            <span className="section-meta mono">{detail.lineItems.length}</span>
          </div>
          {detail.lineItems.length ? (
            <div className="line-items">
              {detail.lineItems.map((item, index) => (
                <div className="line-item" key={`${index}-${item.description}`}>
                  <span>{item.description}</span>
                  <span className="mono dim">×{item.quantity}</span>
                  <span className="mono num">{money(item.amountMinor, item.currency)}</span>
                </div>
              ))}
            </div>
          ) : (
            <p className="muted-copy">No line items were found.</p>
          )}
        </section>}

        {summary.duplicateOf && (
          <div className="review-section duplicate-link">
            <button className="button" onClick={() => openDocument(summary.duplicateOf ?? "")}>
              <Copy size={14} aria-hidden="true" /> Open the earlier copy
            </button>
          </div>
        )}

        {detail.history.length > 0 && (
          <section className="review-section">
            <div className="section-label">
              <span>Corrections</span>
              <span className="section-meta mono">{detail.history.length}</span>
            </div>
            <ol className="history">
              {[...detail.history].reverse().map((change, index) => (
                <li key={index}>
                  <span className="history-field">{fieldLabels[change.field]}</span>
                  <span className="history-change">
                    <s className="dim">{change.old ?? "empty"}</s>
                    <span aria-hidden="true">→</span>
                    <span>{change.new ?? "cleared"}</span>
                  </span>
                  <time className="dim mono" dateTime={new Date(change.changedAt * 1000).toISOString()}>
                    {timeAgo(change.changedAt)}
                    {change.oldSource && change.oldSource !== "human" ? ` · was ${change.oldSource}` : ""}
                  </time>
                </li>
              ))}
            </ol>
          </section>
        )}

        <section className="review-section">
          <button
            className="section-label section-toggle"
            onClick={() => setShowSource(!showSource)}
            aria-expanded={showSource}
          >
            <span>Source</span>
            <ChevronDown size={14} className={showSource ? "" : "collapsed"} aria-hidden="true" />
          </button>
          {showSource && (
            <SourcePreview detail={detail} openOriginal={openOriginal} />
          )}
        </section>
      </div>

      <footer className="review-actions">
        <button
          className={`button icon-only ${confirmDelete ? "danger" : ""}`}
          onClick={() => (confirmDelete ? remove() : setConfirmDelete(true))}
          aria-label={confirmDelete ? "Confirm delete" : "Delete document"}
          title={confirmDelete ? "Click again to delete" : "Delete document"}
        >
          <Trash2 size={14} aria-hidden="true" />
        </button>
        {state === "queued" && (
          <button className="button" onClick={cancelImport} title="Stop this import">
            <X size={14} aria-hidden="true" /> Cancel import
          </button>
        )}
        {state !== "approved" && state !== "queued" && (
          <button
            className="button icon-only"
            onClick={reprocess}
            aria-label="Read the document again"
            title="Read the document again"
          >
            <RefreshCw size={14} aria-hidden="true" />
          </button>
        )}
        <span className="actions-spacer" />
        <button className="button" onClick={skip}>
          <SkipForward size={14} aria-hidden="true" /> Skip <kbd>J</kbd>
        </button>
        <button className="button button-primary" onClick={approve} disabled={!canApprove}>
          <Check size={15} aria-hidden="true" />
          {state === "approved" ? "Approved" : "Approve"} <kbd>A</kbd>
        </button>
      </footer>
    </aside>
  );
}

function FieldInput({
  name,
  label: labelText,
  required,
  field,
  currency,
  categories,
  update,
  inputRef,
}: {
  name: FieldName;
  label: string;
  required: boolean;
  field?: FieldView;
  currency: string;
  categories: string[];
  update: (edit: FieldEdit) => Promise<boolean>;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  const saved = field?.value ?? "";
  const [draft, setDraft] = useState(saved);
  useEffect(() => setDraft(saved), [saved]);

  async function commit(value: string) {
    if (value.trim() === saved) return;
    const ok = await update({ field: name, value: value.trim() });
    if (!ok) setDraft(saved);
  }

  const flagged = field?.needsReview || (!field && required);
  const label = (
    <span className="field-name">
      {labelText}
      {field && field.tier !== "human" && (
        <em className={field.needsReview ? "low" : ""}>{field.confidence}</em>
      )}
    </span>
  );

  if (name === "category" || name === "currency" || name === "auto_renew") {
    const options = name === "category" ? categories : name === "currency" ? currencies : ["yes", "no"];
    return (
      <label className={`field ${flagged ? "flagged" : ""}`}>
        {label}
        <select value={draft} onChange={(event) => { setDraft(event.target.value); void commit(event.target.value); }}>
          <option value="">{name === "category" ? "Uncategorized" : "—"}</option>
          {options.map((option) => (
            <option key={option}>{option}</option>
          ))}
        </select>
      </label>
    );
  }

  const isMoney = moneyFields.includes(name);
  const isDate = dateFields.includes(name);
  const input = (
    <input
      ref={inputRef}
      className={isMoney || isDate || name === "invoice_number" ? "mono" : ""}
      type={isDate ? "date" : "text"}
      inputMode={isMoney ? "decimal" : undefined}
      value={draft}
      maxLength={200}
      placeholder={isMoney ? "0.00" : name === "notice_period" ? "30 days" : name === "parties" ? "First party; second party" : ""}
      onChange={(event) => setDraft(event.target.value)}
      onBlur={(event) => void commit(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") {
          setDraft(saved);
          event.currentTarget.blur();
        }
      }}
      aria-label={labelText}
    />
  );
  return (
    <label className={`field ${flagged ? "flagged" : ""}`}>
      {label}
      {isMoney ? (
        <div className="money-input">
          <span className="mono">{currency}</span>
          {input}
        </div>
      ) : (
        input
      )}
    </label>
  );
}

function SourcePreview({
  detail,
  openOriginal,
}: {
  detail: DocumentDetail;
  openOriginal: (() => void) | null;
}) {
  const { summary } = detail;
  const isImage = summary.mediaType !== "pdf" && summary.mediaType !== "tiff";
  const [imageUrl, setImageUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!isImage) return;
    let url: string | null = null;
    let cancelled = false;
    fileBytes(summary.id)
      .then((bytes) => {
        if (cancelled) return;
        url = URL.createObjectURL(new Blob([bytes], { type: `image/${summary.mediaType}` }));
        setImageUrl(url);
      })
      .catch(() => setImageUrl(null));
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [isImage, summary.id, summary.mediaType]);

  return (
    <div className="source">
      {isImage && imageUrl && (
        <div className="paper-stage">
          <img className="source-image" src={imageUrl} alt={`Original of ${summary.fileName}`} />
        </div>
      )}
      {!isImage &&
        (detail.text ? (
          <pre className="source-text mono">{detail.text}</pre>
        ) : (
          <p className="muted-copy">No text layer to show.</p>
        ))}
{openOriginal && (
      <button className="button source-open" onClick={openOriginal}>
        <ExternalLink size={14} aria-hidden="true" /> Open original
      </button>
      )}
    </div>
  );
}
