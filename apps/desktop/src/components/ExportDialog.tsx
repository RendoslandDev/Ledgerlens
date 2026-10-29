import type { ExportFormat, ExportPlan } from "@ledgerlens/bindings";
import { ArrowDownToLine, ShieldAlert } from "lucide-react";
import { Modal } from "./Modal";

const formatNames: Record<ExportFormat, string> = {
  canonical: "Ledgerlens CSV",
  quickbooks: "QuickBooks CSV",
  xero: "Xero CSV",
  ofx: "Bank statements (OFX)",
};

/** Shows exactly what an export will write, and where, before anything is written. */
export function ExportDialog({
  format,
  plan,
  busy,
  onConfirm,
  onClose,
}: {
  format: ExportFormat;
  plan: ExportPlan;
  busy: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal
      title={format === "ofx" ? "Export reviewed statements" : "Export reviewed records"}
      onClose={onClose}
      footer={
        <>
          <button className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            className="button button-primary"
            onClick={onConfirm}
            disabled={busy || plan.rows === 0}
            autoFocus
          >
            <ArrowDownToLine size={14} aria-hidden="true" /> Write {plan.paths.length > 1 ? `${plan.paths.length} files` : "file"}
          </button>
        </>
      }
    >
      <dl className="kv">
        <div>
          <dt>Format</dt>
          <dd>{formatNames[format]}</dd>
        </div>
        <div>
          <dt>{format === "ofx" ? "Transactions" : "Records"}</dt>
          <dd className="mono">{plan.rows}</dd>
        </div>
      </dl>
      {plan.paths.length > 0 && (
        <>
          <p className="modal-label">
            {plan.paths.length > 1 ? "One file per account, saved to" : "The file will be saved to"}
          </p>
          <pre className="path-box mono">{plan.paths.join("\n")}</pre>
        </>
      )}
      {format === "ofx" && (
        <p className="muted-copy">
          Import it in QuickBooks Online (Banking › Upload from file) or Xero (Bank account › Import
          a statement). Each transaction carries a fixed ID, so importing the same statement twice
          doesn't double it.
        </p>
      )}
      <div className="notice warn">
        <ShieldAlert size={15} aria-hidden="true" />
        <span>
          {format === "ofx"
            ? "The file holds transactions, descriptions, and balances in plain text. Anyone who can open that folder can read it."
            : "The file holds vendor names, dates, and amounts in plain text. Anyone who can open that folder can read it."}
        </span>
      </div>
      {plan.rows === 0 && (
        <p className="muted-copy">
          {format === "ofx"
            ? "There are no reviewed statements to export yet."
            : "There are no reviewed records to export yet."}
        </p>
      )}
    </Modal>
  );
}
