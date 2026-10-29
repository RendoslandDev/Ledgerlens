import type { AmountRow, ExportFormat, Report } from "@ledgerlens/bindings";
import { ArrowDownToLine } from "lucide-react";
import { useState } from "react";
import { initials } from "../components/Rail";
import { money, monthLabel } from "../format";

export function ReportsView({
  pro,
  report,
  exportCsv,
  workspaceName,
  since,
  defaultCurrency,
}: {
  /** Offers the OFX statement export (Ledgerlens Pro). */
  pro: boolean;
  report: Report | null;
  exportCsv: (format: ExportFormat) => void;
  workspaceName: string;
  /** When the first document was added (Unix seconds), if any. */
  since: number | null;
  defaultCurrency: string | null;
}) {
  const [format, setFormat] = useState<ExportFormat>("canonical");
  if (!report) {
    return <section className="reports"><p className="dim">Loading…</p></section>;
  }
  const { counts } = report;
  const reviewed = counts.extracted + counts.approved;
  const primary = report.byCurrency[0];
  // Bars only compare amounts in the same currency as the largest total.
  const barCurrency = primary?.currency;

  return (
    <section className="reports">
      <header className="report-hero">
        <span className="report-avatar" aria-hidden="true">
          {initials(workspaceName)}
        </span>
        <div>
          <h2>{workspaceName}</h2>
          <p>
            {[
              since
                ? `Tracking since ${new Date(since * 1000).toLocaleDateString(undefined, { month: "long", year: "numeric" })}`
                : "No documents yet",
              `${counts.total} document${counts.total === 1 ? "" : "s"}`,
              defaultCurrency && `${defaultCurrency} by default`,
            ]
              .filter(Boolean)
              .map((part, index) => (
                <span key={index}>{part}</span>
              ))}
          </p>
        </div>
      </header>

      <div className="stat-strip">
        <div className="stat">
          <span>Total tracked</span>
          <strong className="mono">
            {primary ? money(primary.totalMinor, primary.currency) : "—"}
          </strong>
          <small>
            {report.byCurrency.length > 1
              ? `+ ${report.byCurrency.length - 1} other currenc${report.byCurrency.length === 2 ? "y" : "ies"}`
              : `${primary?.count ?? 0} reviewed records`}
          </small>
        </div>
        <div className="stat">
          <span>Reviewed</span>
          <strong className="mono">
            {reviewed}
            <em>/{counts.total}</em>
          </strong>
          <small>Ready or approved</small>
        </div>
        <div className="stat">
          <span>Open review</span>
          <strong className={`mono ${counts.needsReview ? "warn-text" : ""}`}>
            {counts.needsReview}
          </strong>
          <small>
            {counts.failed ? `${counts.failed} unreadable` : "Awaiting confirmation"}
          </small>
        </div>
        <div className="stat">
          <span>Read automatically</span>
          <strong className="mono">
            {report.highConfidencePercent ?? "—"}
            {report.highConfidencePercent !== null && <em>%</em>}
          </strong>
          <small>Required fields above the review threshold</small>
        </div>
      </div>

      {report.byCurrency.length === 0 ? (
        <div className="panel empty-report">
          <strong>No reviewed records yet</strong>
          <span>Reports fill in as documents are read or approved in the inbox.</span>
        </div>
      ) : (
        <div className="reports-grid">
          <Breakdown
            title="Spend by category"
            rows={report.byCategory}
            currency={barCurrency}
            label={(row) => row.label ?? "Uncategorized"}
          />
          <Breakdown
            title="Spend by vendor"
            rows={report.byVendor}
            currency={barCurrency}
            label={(row) => row.label ?? "Unknown vendor"}
          />
          <Breakdown
            title="Spend by month"
            rows={report.byMonth}
            currency={barCurrency}
            label={(row) => (row.label ? monthLabel(row.label) : "Undated")}
          />
          {report.byCurrency.length > 1 && (
            <Breakdown
              title="By currency"
              rows={report.byCurrency}
              currency={undefined}
              label={(row) => row.currency}
            />
          )}
        </div>
      )}

      <div className="reports-foot">
        <span>Currencies are totalled separately; no FX conversion is applied.</span>
        <span className="export-controls">
          <select
            value={format}
            onChange={(event) => setFormat(event.target.value as ExportFormat)}
            aria-label="Export format"
          >
            <option value="canonical">Ledgerlens CSV</option>
            <option value="quickbooks">QuickBooks CSV</option>
            <option value="xero">Xero CSV</option>
            {pro && <option value="ofx">Bank statements (OFX)</option>}
          </select>
          <button className="button" onClick={() => exportCsv(format)}>
            <ArrowDownToLine size={14} aria-hidden="true" /> Export
          </button>
        </span>
      </div>
    </section>
  );
}

function Breakdown({
  title,
  rows,
  currency,
  label,
}: {
  title: string;
  rows: AmountRow[];
  currency: string | undefined;
  label: (row: AmountRow) => string;
}) {
  // Bars compare amounts within one currency only; each currency has its own scale.
  const largest = new Map<string, number>();
  rows.forEach((row) =>
    largest.set(row.currency, Math.max(largest.get(row.currency) ?? 1, row.totalMinor)),
  );
  const mixed = largest.size > 1;
  return (
    <section className="panel">
      <div className="section-label">
        <span>{title}</span>
        <span className="section-meta">
          {mixed || !currency ? "Scaled per currency" : currency}
        </span>
      </div>
      <div className="bars">
        {rows.slice(0, 10).map((row, index) => (
          <div className="bar-row" key={`${row.label}-${row.currency}-${index}`}>
            <span className="bar-label">{label(row)}</span>
            <span className="bar-track">
              <i
                style={{
                  width: `${Math.max((row.totalMinor / (largest.get(row.currency) ?? 1)) * 100, 2)}%`,
                }}
              />
            </span>
            <span className="mono num">{money(row.totalMinor, row.currency)}</span>
            <span className="mono dim num">{row.count}</span>
          </div>
        ))}
      </div>
    </section>
  );
}
