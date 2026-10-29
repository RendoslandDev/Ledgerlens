import type { DocKind, DocStatus, DocumentSummary, FieldName } from "@ledgerlens/bindings";

/** The review states the UI shows, collapsed from the backend's document states. */
export type QueueState = "review" | "ready" | "approved" | "queued" | "failed";

export function queueState(status: DocStatus): QueueState {
  switch (status) {
    case "needs_review":
      return "review";
    case "extracted":
    case "done":
      return "ready";
    case "approved":
      return "approved";
    case "failed":
      return "failed";
    default:
      return "queued";
  }
}

export const stateLabels: Record<QueueState, string> = {
  review: "Needs review",
  ready: "Ready",
  approved: "Approved",
  queued: "Reading",
  failed: "Unreadable",
};

/** Matches the backend's review threshold (`escalate_below`, 0.85). */
export const HIGH_CONFIDENCE = 85;

export const fieldLabels: Record<FieldName, string> = {
  vendor: "Vendor",
  invoice_number: "Number",
  doc_date: "Date",
  due_date: "Due",
  currency: "Currency",
  subtotal: "Subtotal",
  tax: "Tax",
  total: "Total",
  payment_method: "Payment",
  category: "Category",
  account_number: "Account",
  period_start: "From",
  period_end: "To",
  opening_balance: "Opening",
  closing_balance: "Closing",
  parties: "Parties",
  end_date: "Ends",
  notice_period: "Notice",
  auto_renew: "Renews",
};

/** The same field reads differently on a statement or a contract. */
export function fieldLabel(name: FieldName, kind: DocKind, pro: boolean): string {
  if (!pro) return fieldLabels[name];
  if (kind === "statement" && name === "vendor") return "Bank";
  if (kind === "statement" && name === "doc_date") return "Issued";
  if (kind === "contract" && name === "doc_date") return "Starts";
  if (kind === "contract" && name === "total") return "Value";
  return fieldLabels[name];
}

const invoiceFields: FieldName[] = [
  "vendor",
  "invoice_number",
  "doc_date",
  "due_date",
  "currency",
  "subtotal",
  "tax",
  "total",
  "category",
];

/** The fields shown for each kind of document, in reading order. Without Pro, statements and
 * contracts are read as invoices, so they show the invoice fields. */
export function editableFields(kind: DocKind, pro: boolean): FieldName[] {
  switch (pro ? kind : "invoice") {
    case "statement":
      return ["vendor", "account_number", "period_start", "period_end", "currency", "opening_balance", "closing_balance"];
    case "contract":
      return ["parties", "doc_date", "end_date", "notice_period", "auto_renew", "currency", "total"];
    default:
      return invoiceFields;
  }
}

/** Fields that must be filled before a document of this kind can leave review (mirrors the backend). */
export function requiredFields(kind: DocKind, pro: boolean): FieldName[] {
  switch (pro ? kind : "invoice") {
    case "statement":
      return ["vendor", "period_end", "opening_balance", "closing_balance"];
    case "contract":
      return ["parties", "doc_date"];
    default:
      return ["vendor", "doc_date", "total"];
  }
}

export const currencies = [
  "USD", "EUR", "GBP", "GHS", "NGN", "KES", "ZAR", "CAD", "AUD", "JPY", "INR", "CNY", "IDR",
];

export function displayName(document: DocumentSummary): string {
  return document.vendor ?? document.fileName;
}
