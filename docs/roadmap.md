# Roadmap: what the docs plan that is not built yet

Compiled on 2026-10-27 from `architecture.md`, the ADRs, `threat-model.md`, `models/`, and
`benchmarks/`, plus an audit of the desktop app and the landing site. Each phase can ship on its
own. Downloads are listed with source, size, and licence; nothing is downloaded without approval.

## Already done

- `ll-app` service wired to the Tauri desktop app: import, text-layer PDFs, on-device OCR
  (macOS Vision), review, approval, Ask, reports, CSV export.
- File admission by magic bytes, 25 MB cap, isolated and time-limited parsing (threat model:
  "does not yet sniff file types, cap file size, or sandbox parsers" is now partly addressed;
  parsers run in-process threads, not an OS sandbox).

## Phase 1 — Desktop app completeness (no downloads) — done 2026-10-27

All items below are built and tested. Folder import uses the webview's folder picker and sends
file bytes, so the app still never receives filesystem paths; `ll-ingest::scan_paths` stays for
the Phase 3 watcher. Dead or missing UI found in the audit:

| Gap | Where | Plan |
| --- | --- | --- |
| Vendors button only shows a toast | Rail | Vendors page: spend, document count, last date per vendor, open a vendor's documents |
| Settings button only shows a toast | Rail | Settings page: workspace name, default currency, review threshold, OCR status, data folder, about |
| "Newest" sort is a static label | Inbox | Working sort: newest, oldest, amount, vendor, confidence |
| Export writes straight to Downloads | Reports | Threat model requires showing the target path and an explicit confirm step before writing |
| Workspace avatar hard-coded "NS" | Rail | Use the workspace name from settings |
| No shortcut reference | App | `?` opens a keyboard shortcuts sheet |
| Only file-by-file import | Inbox | "Import folder" through `ll-ingest::scan_paths` |
| Same receipt in two formats imports twice | Import | Likely-duplicate check (vendor + number + date + total) using the existing `DuplicateSuspected` issue code |
| No bulk review | Inbox | Approve all "Ready" documents in one action |
| Corrections are overwritten | Store | ADR 0002: "preserve user corrections as data" — correction history table and a field history view |
| Search only matches vendor/number/category | Inbox | `ll-store` plan: FTS5 full-text search over document text |
| No logging redaction tests | Threat model | Tests that command errors never contain document text, field values, or SQL parameters |

## Phase 2 — Typed IPC (ADR 0003) — done 2026-10-27

Generate `packages/bindings` with `tauri-specta` instead of hand-written types, plus a check that
fails CI when they drift. Download: crates `tauri-specta`/`specta` (source crates, small).

## Phase 3 — Durable background processing — done 2026-10-27

- Imports are admitted (checked and stored) at once and read by a background worker through the
  `ll-jobs` queue: leases, retries with backoff, crash recovery on start, cancellation of single
  or all waiting imports. OCR runs without holding the service lock, so the UI stays responsive.
- Opt-in folder watching with the threat model's controls: folder chosen only in the system
  dialog, broad/system/own-data folders refused, no symlinks followed, depth/count/size limits,
  per-scan batch limits, pause and stop, and the scope shown in Settings. Deleted documents are not
  re-imported unless the file changes.
- Watching polls every 15 seconds instead of subscribing to file events, so no new dependency was
  downloaded. The folder dialog is macOS-only for now.

## Phase 4 — Measured accuracy (benchmarks, `ll-eval`) — done 2026-10-27

- Downloaded the **CORD v2 test split** (234 MB, CC BY 4.0, SHA-256 checked) to
  `~/.cache/ledgerlens/datasets/cord-v2`, outside the repository. Reading parquet added the
  `parquet` crate (approved) to `ll-eval` only.
- `ll-eval cord-run` imports each receipt through the real app path and records the run in
  `benchmarks/results.json`. First run on an Apple M2: total F1 0.74, subtotal 0.78, tax 0.45,
  line prices 0.01. All 100 receipts went to review, so nothing wrong was auto-accepted. Details
  and the failure patterns are in `benchmarks/README.md`.
- Gaps it surfaced: IDR is not a supported currency; item-count lines ("TOTAL QTY 2") are read as
  totals; tax and subtotal are often missed; line items are read only below a table header.
- Rules v2 (2026-10-27) closed those gaps, tuned on the CORD validation split (downloaded with
  approval) with the test split held out. Test F1: total 0.77, subtotal 0.84, tax 0.79, line
  prices 0.68; 57 of 100 receipts fully correct (was 42). See `benchmarks/README.md`.
- SROIE is not proposed: its licence is research-only and unclear for this project.

## Phase 5 — Models and semantic search (`ll-infer`, `ll-embed`)

- Download **all-MiniLM-L6-v2 ONNX, int8 arm64** — Hugging Face
  `sentence-transformers/all-MiniLM-L6-v2`, `onnx/model_qint8_arm64.onnx` (23 MB) +
  `tokenizer.json`, licence Apache-2.0. Recorded in `models/manifest.json` with SHA-256 and size.
- ONNX Runtime via the `ort` crate (fetches the runtime library at build time, ~30 MB).
- `sqlite-vec` for vector search (crate, small). Enables "find documents like this" and search by
  meaning, with citations back to documents (`ll-query` plan).
- Cross-platform OCR for Windows and Linux. Both now build and ship (text PDFs only); OCR there is
  the next platform gap, along with folder watching (the folder dialog is macOS-only).

## Statements, contracts, and OFX — done 2026-10-29

Planned as **Ledgerlens Pro**, a paid tier after the App Store launch. Until then it is switched
off in every build: `ll-app` has a `pro` feature (off by default; the desktop crate forwards it,
`npx tauri dev --features pro`), and without it statements and contracts keep their type but are
read as invoices, approval and OFX export say they're part of Pro, and the UI hides their
fields. Purchase and licensing are not built yet. The tests run both ways (`App::with_pro`).

- **Bank statements:** bank, account (last four digits only), period, opening and closing
  balances, and one transaction per row (`ll-extract/src/statement.rs`, `transactions` table).
  A row's direction comes from the running balance; opening + transactions = closing is
  checked, and a statement that doesn't reconcile goes to review.
- **Contracts:** parties, start and end dates (end worked out from a stated term when needed),
  notice period, auto-renewal, and value, plus the last day to give notice
  (`ll-extract/src/contract.rs`). Key terms only; no clause analysis.
- **OFX export** (`ll-export/src/ofx.rs`): reviewed statements, one OFX 1.0.2 file per account,
  with stable `FITID`s so a re-import is a duplicate. Imported by QuickBooks Online and Xero.
- Choosing a document's type re-reads it with that type's rules, and the choice sticks.
- **Not yet:** tested on sample statements and contracts only. Real banks' layouts vary
  (multi-page statements, rows split across columns); collect redacted samples, then add a
  statement benchmark like the CORD one. No reminder when a contract's notice date nears.

## Phase 6 — Security before shipping installers

- Encryption at rest for the database, blobs, and cached copies (key in the OS keychain).
- Signed installers; revisit the threat model (it lists "shipping installers" as a review trigger).

## Deliberately not planned now

- **Sync relay** (`ll-sync`, `apps/relay` is empty): ADR 0004 says key provisioning, replay
  protection, device lifecycle, and conflict resolution must be designed and reviewed first.
- **QuickBooks/Xero API integrations**: need developer accounts and OAuth credentials you own,
  and would send records off the device, which Ledgerlens promises not to do. OFX and CSV files
  that the person imports cover the same need without a network connection.
- **VLM escalation** (ADR 0002): multi-gigabyte models; revisit after Phase 4 shows where the
  rules + OCR path fails.

## Landing site gaps

- "Get the preview" leads to build-from-source steps only; there is no download or release page.
- No privacy page and no footer links to the docs. The product image is an HTML illustration of
  the board view using the app's own test records, not a screenshot.

## Mobile and distribution (2026-10-28)

- iOS: Tauri 2 mobile app from the same Rust core and React UI, with Apple Vision OCR on the
  device. Folder import, folder watching, and "open in another app" are hidden on phones.
  Project in `apps/desktop/src-tauri/gen/apple`.
- Android: built from the same code (`gen/android`) and checked on an API 36 emulator. A text PDF
  imports and extracts; scanned PDFs and photos go to review with a "no OCR" note (no on-device
  OCR without adding ML Kit). Android's WebView can't pass request bodies to Tauri, so imports
  there send the file as `{ base64 }` JSON instead of a raw body (`api.ts`, `commands.rs`).
- Minimum versions: macOS 13 and iOS 16, the first releases with Vision's automatic language
  detection, which the OCR uses.
- Distribution: `docs/deploy.md` covers the website (GitHub Pages via
  `.github/workflows/site.yml`), macOS releases (`.github/workflows/release.yml`, ad-hoc signed
  until Developer ID secrets are added), the App Store, and Google Play. Store accounts, signing,
  and privacy-policy hosting are the owner's to set up.

## Design (2026-10-27)

The desktop app and the site share one layout with two personalities: soft and rounded in light
(the default), a monospace terminal in dark. The inbox has a table and a board view (drag to
Approved to approve), an empty workspace opens on a welcome screen, and Reports leads with a
workspace header and stat pills. Site copy was corrected to claim only what is built (macOS only,
no model tier, no network requests, no encryption at rest yet).
