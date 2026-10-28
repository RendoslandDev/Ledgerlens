import type { DocStatus, DocumentSummary, FieldName } from "@ledgerlens/bindings";

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
};

export const editableFields: FieldName[] = [
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

export const currencies = [
  "USD", "EUR", "GBP", "GHS", "NGN", "KES", "ZAR", "CAD", "AUD", "JPY", "INR", "CNY", "IDR",
];

export function displayName(document: DocumentSummary): string {
  return document.vendor ?? document.fileName;
}
