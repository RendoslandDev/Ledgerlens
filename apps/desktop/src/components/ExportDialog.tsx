import type { ExportFormat, ExportPlan } from "@ledgerlens/bindings";
import { ArrowDownToLine, ShieldAlert } from "lucide-react";
import { Modal } from "./Modal";

const formatNames: Record<ExportFormat, string> = {
  canonical: "Ledgerlens CSV",
  quickbooks: "QuickBooks CSV",
  xero: "Xero CSV",
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
      title="Export reviewed records"
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
            <ArrowDownToLine size={14} aria-hidden="true" /> Write file
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
          <dt>Records</dt>
          <dd className="mono">{plan.rows}</dd>
        </div>
      </dl>
      <p className="modal-label">The file will be saved to</p>
      <pre className="path-box mono">{plan.path}</pre>
      <div className="notice warn">
        <ShieldAlert size={15} aria-hidden="true" />
        <span>
          The file holds vendor names, dates, and amounts in plain text. Anyone who can open that
          folder can read it.
        </span>
      </div>
      {plan.rows === 0 && (
        <p className="muted-copy">There are no reviewed records to export yet.</p>
      )}
    </Modal>
  );
}
