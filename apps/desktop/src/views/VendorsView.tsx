import type { VendorSummary } from "@ledgerlens/bindings";
import { ArrowRight, Building2, Search } from "lucide-react";
import { useState } from "react";
import { longDate, money } from "../format";

export function VendorsView({
  vendors,
  openVendor,
}: {
  vendors: VendorSummary[] | null;
  openVendor: (name: string) => void;
}) {
  const [filter, setFilter] = useState("");
  if (!vendors) {
    return <div className="loading-state">Loading vendors…</div>;
  }
  const needle = filter.trim().toLowerCase();
  const shown = vendors.filter(
    (vendor) =>
      !needle ||
      vendor.name.toLowerCase().includes(needle) ||
      vendor.categories.some((category) => category.toLowerCase().includes(needle)),
  );

  return (
    <section className="page">
      <div className="page-toolbar">
        <span className="dim">
          {vendors.length} vendor{vendors.length === 1 ? "" : "s"} · spend counts reviewed records
          only
        </span>
        <label className="filter-input">
          <Search size={14} aria-hidden="true" />
          <input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter vendors"
            aria-label="Filter vendors"
          />
        </label>
      </div>

      {vendors.length === 0 ? (
        <div className="empty-state first-run">
          <Building2 size={22} aria-hidden="true" />
          <strong>No vendors yet</strong>
          <span>Vendors appear here as documents are read or entered in the inbox.</span>
        </div>
      ) : (
        <div className="table vendor-table" role="grid" aria-label="Vendors">
          <div className="table-head" role="row">
            <span>Vendor</span>
            <span className="num">Documents</span>
            <span className="col-category">Categories</span>
            <span>Last document</span>
            <span className="num">Reviewed spend</span>
            <span />
          </div>
          {shown.map((vendor) => (
            <button
              key={vendor.name}
              role="row"
              className="table-row"
              onClick={() => openVendor(vendor.name)}
              title={`Show ${vendor.name}'s documents`}
            >
              <span className="cell-vendor">
                <strong>{vendor.name}</strong>
                {vendor.needsReview > 0 && (
                  <span className="tag review">{vendor.needsReview} to review</span>
                )}
              </span>
              <span className="mono num">{vendor.documents}</span>
              <span className="col-category dim">{vendor.categories.join(", ") || "—"}</span>
              <span className="mono dim">{vendor.lastDate ? longDate(vendor.lastDate) : "—"}</span>
              <span className="mono num">
                {vendor.totals.length
                  ? vendor.totals.map((row) => money(row.totalMinor, row.currency)).join(" · ")
                  : "—"}
              </span>
              <ArrowRight size={14} className="dim" aria-hidden="true" />
            </button>
          ))}
          {shown.length === 0 && (
            <div className="empty-state">
              <strong>No vendors match</strong>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
