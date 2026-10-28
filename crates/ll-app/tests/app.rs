//! End-to-end service tests over a real PDF (produced by macOS Quartz from `fixtures/invoice.txt`).

use ll_app::{App, ExportFormat, FieldEdit};
use ll_core::{Date, DocId};
use std::{fs, path::PathBuf, str::FromStr};

const INVOICE_PDF: &[u8] = include_bytes!("fixtures/invoice-cups.pdf");
/// A real, decodable PNG with nothing on it: the manual-entry path.
const PNG: &[u8] = include_bytes!("fixtures/blank.png");
/// A receipt rendered by WebKit, then saved as a compressed JPEG and as an image-only PDF. Only
/// the OCR tests read them, and OCR is built for macOS.
#[cfg(target_os = "macos")]
const RECEIPT_JPEG: &[u8] = include_bytes!("../../ll-vision/tests/fixtures/receipt.jpg");
#[cfg(target_os = "macos")]
const SCANNED_PDF: &[u8] = include_bytes!("../../ll-vision/tests/fixtures/scanned.pdf");

struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ll-app-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Self { dir }
    }

    fn app(&self) -> App {
        App::open(&self.dir).unwrap().with_today(Date::new(2026, 10, 27).unwrap())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn edit(field: &str, value: &str) -> FieldEdit {
    FieldEdit { field: field.into(), value: value.into() }
}

fn id(raw: &str) -> DocId {
    DocId::from_str(raw).unwrap()
}

#[test]
fn a_text_pdf_is_read_into_a_validated_record() {
    let workspace = Workspace::new("read");
    let mut app = workspace.app();
    let outcome = app.import("invoice.pdf", INVOICE_PDF).unwrap();
    assert!(!outcome.duplicate);
    let doc = outcome.document;
    assert_eq!(doc.vendor.as_deref(), Some("Kumasi Cloud Services Ltd"));
    assert_eq!(doc.invoice_number.as_deref(), Some("KCS-2026-0193"));
    assert_eq!(doc.date.as_deref(), Some("2026-09-14"));
    assert_eq!(doc.currency.as_deref(), Some("GHS"));
    assert_eq!(doc.total_minor, Some(172_500));
    assert_eq!(doc.kind, "invoice");
    assert_eq!(doc.status, "extracted", "a consistent invoice should not need review");

    let detail = app.detail(id(&doc.id)).unwrap();
    assert_eq!(detail.line_items.len(), 2);
    assert_eq!(detail.line_items[1].amount_minor, 30_000);
    assert!(detail.text.unwrap().contains("Managed hosting plan"));
    assert!(detail.issues.is_empty(), "{:?}", detail.issues);
}

#[test]
fn text_layer_pdfs_record_their_source() {
    let workspace = Workspace::new("source");
    let mut app = workspace.app();
    let doc = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    assert_eq!(doc.text_source.map(|source| source.as_str()), Some("text_layer"));
    assert_eq!(doc.note, None);
}

#[cfg(target_os = "macos")]
#[test]
fn photos_and_scans_are_read_with_on_device_ocr() {
    let workspace = Workspace::new("ocr");
    let mut app = workspace.app();
    let mut first_id = None;
    for (name, bytes) in [("receipt.jpg", RECEIPT_JPEG), ("scan.pdf", SCANNED_PDF)] {
        let doc = app.import(name, bytes).unwrap().document;
        assert_eq!(doc.text_source.map(|source| source.as_str()), Some("ocr"), "{name}");
        assert_eq!(doc.vendor.as_deref(), Some("GitHub, Inc."), "{name}");
        assert_eq!(doc.invoice_number.as_deref(), Some("GH-INV-58210"), "{name}");
        assert_eq!(doc.date.as_deref(), Some("2026-10-03"), "{name}");
        assert_eq!((doc.total_minor, doc.currency.as_deref()), (Some(3500), Some("USD")), "{name}");
        assert_eq!(doc.kind, "receipt", "{name}");
        match &first_id {
            None => {
                assert_eq!(doc.status, "extracted", "{name}: subtotal + tax = total corroborates the OCR");
                first_id = Some(doc.id.clone());
            }
            Some(first) => {
                // The scan is the same receipt as the photo: different bytes, same content.
                assert_eq!(doc.duplicate_of.as_ref(), Some(first), "{name}");
                assert_eq!(doc.status, "needs_review", "{name}: duplicates wait for a person");
            }
        }
        assert!(doc.note.unwrap().contains("OCR"));
        let detail = app.detail(id(&doc.id)).unwrap();
        assert_eq!(detail.line_items.len(), 2, "{name}: {:?}", detail.line_items);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn ocr_amounts_that_do_not_add_up_go_to_review() {
    let workspace = Workspace::new("ocr-review");
    let mut app = workspace.app();
    let doc = app.import("receipt.jpg", RECEIPT_JPEG).unwrap().document;
    let doc_id = id(&doc.id);
    // A misread tax is exactly the error the corroboration rule exists to catch.
    let detail = app.update_field(doc_id, &edit("tax", "8.00")).unwrap();
    assert_eq!(detail.summary.status, "needs_review");
}

#[test]
fn the_same_bytes_are_imported_once() {
    let workspace = Workspace::new("dedupe");
    let mut app = workspace.app();
    let first = app.import("a.pdf", INVOICE_PDF).unwrap();
    let second = app.import("copy of a.pdf", INVOICE_PDF).unwrap();
    assert!(second.duplicate);
    assert_eq!(first.document.id, second.document.id);
    assert_eq!(app.list().unwrap().len(), 1);
}

#[test]
fn hostile_input_is_rejected_or_contained() {
    let workspace = Workspace::new("hostile");
    let mut app = workspace.app();
    assert!(app.import("evil.pdf", b"MZ\x90\0not a pdf").is_err());
    assert!(app.import("empty.pdf", b"").is_err());
    assert!(app.list().unwrap().is_empty(), "rejected files leave nothing behind");

    let broken = app.import("../../broken.pdf", b"%PDF-1.7\nthis is not a real pdf").unwrap().document;
    assert_eq!(broken.status, "failed");
    assert_eq!(broken.file_name, "broken.pdf");
    assert!(broken.note.is_some());
}

#[test]
fn images_wait_for_manual_entry_then_validate() {
    let workspace = Workspace::new("manual");
    let mut app = workspace.app();
    let doc = app.import("receipt.png", PNG).unwrap().document;
    assert_eq!((doc.status.as_str(), doc.kind.as_str()), ("needs_review", "receipt"));
    assert!(doc.note.unwrap().contains("by hand"));
    let missing = app.detail(id(&doc.id)).unwrap().issues;
    assert!(missing.iter().any(|issue| issue.message == "Missing date"), "{missing:?}");
    assert_eq!(doc.text_source, None);
    let doc_id = id(&doc.id);

    assert!(app.approve(doc_id).is_err(), "an empty record cannot be approved");
    assert!(app.update_field(doc_id, &edit("total", "12abc")).is_err());
    assert!(app.update_field(doc_id, &edit("total", "99999999999999999")).is_err(), "beyond 2^53 minor units");
    assert!(app.update_field(doc_id, &edit("doc_date", "yesterday")).is_err());
    assert!(app.update_field(doc_id, &edit("category", "Bribes")).is_err());
    assert!(app.update_field(doc_id, &edit("vendor", &"x".repeat(201))).is_err());
    assert!(app.update_field(doc_id, &edit("not_a_field", "1")).is_err());

    app.update_field(doc_id, &edit("currency", "usd")).unwrap();
    app.update_field(doc_id, &edit("vendor", "Corner Café")).unwrap();
    app.update_field(doc_id, &edit("doc_date", "2026-10-02")).unwrap();
    let detail = app.update_field(doc_id, &edit("total", "18.40")).unwrap();
    assert_eq!(detail.summary.status, "extracted");
    assert_eq!(detail.summary.total_minor, Some(1840));
    assert_eq!(detail.summary.confidence, 100);
    assert!(detail.fields.iter().all(|field| field.tier == "human"));

    let approved = app.approve(doc_id).unwrap();
    assert_eq!(approved.status, "approved");
    app.update_field(doc_id, &edit("category", "meals")).unwrap();
    assert_eq!(app.summary(doc_id).unwrap().status, "approved", "categorising keeps approval");
    assert_eq!(app.summary(doc_id).unwrap().category.as_deref(), Some("Meals"));
    app.update_field(doc_id, &edit("total", "19.40")).unwrap();
    assert_eq!(app.summary(doc_id).unwrap().status, "extracted", "changing money re-opens approval");
}

#[test]
fn human_corrections_survive_reprocessing() {
    let workspace = Workspace::new("reprocess");
    let mut app = workspace.app();
    let doc = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    let doc_id = id(&doc.id);
    app.update_field(doc_id, &edit("vendor", "Kumasi Cloud")).unwrap();
    let queued = app.reprocess(doc_id).unwrap();
    assert_eq!(queued.status.as_str(), "queued");
    assert_eq!(app.run_pending().unwrap(), 1);
    let again = app.summary(doc_id).unwrap();
    assert_eq!(again.vendor.as_deref(), Some("Kumasi Cloud"));
    assert_eq!(again.total_minor, Some(172_500));
}

#[test]
fn questions_and_reports_use_reviewed_records_only() {
    let workspace = Workspace::new("ask");
    let mut app = workspace.app();
    let invoice = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    app.update_field(id(&invoice.id), &edit("category", "Software")).unwrap();
    let pending = app.import("receipt.png", PNG).unwrap().document;
    for (field, value) in [("vendor", "Corner Café"), ("total", "18.40"), ("currency", "USD")] {
        app.update_field(id(&pending.id), &edit(field, value)).unwrap();
    }
    assert_eq!(app.summary(id(&pending.id)).unwrap().status, "needs_review", "no date yet");

    let answer = app.ask("How much did we spend on software in September 2026?").unwrap();
    assert_eq!(answer.answer, "Software spend in September 2026: GHS 1,725.00 across 1 record.");
    assert!(answer.sql.unwrap().contains("?"), "values are bound, not inlined");
    assert_eq!(answer.rows[0].total_minor, 172_500);

    let everything = app.ask("total spend").unwrap();
    assert_eq!(everything.rows.len(), 1, "the unreviewed USD receipt is excluded");
    assert_eq!(everything.pending_review, 1);

    let review = app.ask("Which documents need review?").unwrap();
    assert_eq!(review.document_ids, std::slice::from_ref(&pending.id));

    assert!(app.ask("").is_err());
    assert!(app.ask(&"a".repeat(501)).is_err());

    let report = app.report().unwrap();
    assert_eq!((report.counts.total, report.counts.extracted, report.counts.needs_review), (2, 1, 1));
    assert_eq!(report.by_category[0].label.as_deref(), Some("Software"));
    assert_eq!(report.by_month[0].label.as_deref(), Some("2026-09"));
    assert_eq!(report.high_confidence_percent, Some(100));
}

#[test]
fn exports_never_overwrite_and_neutralise_formulas() {
    let workspace = Workspace::new("export");
    let mut app = workspace.app();
    let doc = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    app.update_field(id(&doc.id), &edit("vendor", "=HYPERLINK(\"http://x\")")).unwrap();
    let out = workspace.dir.join("out");
    fs::create_dir_all(&out).unwrap();

    let first = app.export(ExportFormat::Quickbooks, &out).unwrap();
    let second = app.export(ExportFormat::Quickbooks, &out).unwrap();
    assert_ne!(first.path, second.path);
    assert_eq!(first.rows, 1);
    let csv = fs::read_to_string(&first.path).unwrap();
    assert!(csv.starts_with("TxnDate,Name,Currency,Amount"));
    assert!(csv.contains("'=HYPERLINK"), "{csv}");
    assert!(csv.contains("1725.00"));
}

#[test]
fn files_round_trip_are_verified_and_deleted() {
    let workspace = Workspace::new("files");
    let mut app = workspace.app();
    let doc = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    let doc_id = id(&doc.id);
    let (media, bytes) = app.file(doc_id).unwrap();
    assert_eq!(media.mime(), "application/pdf");
    assert_eq!(bytes, INVOICE_PDF);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(workspace.dir.join("ledgerlens.db")).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the database is private to the user");
    }

    let blob = fs::read_dir(workspace.dir.join("blobs")).unwrap().next().unwrap().unwrap().path();
    let blob = fs::read_dir(blob).unwrap().next().unwrap().unwrap().path();
    fs::write(&blob, b"%PDF-1.7 tampered").unwrap();
    assert!(app.file(doc_id).is_err(), "tampered files are refused");

    app.delete(doc_id).unwrap();
    assert!(!blob.exists());
    assert!(app.list().unwrap().is_empty());
    assert!(app.delete(doc_id).is_err());
}

#[test]
fn data_persists_across_restarts() {
    let workspace = Workspace::new("restart");
    let doc_id = {
        let mut app = workspace.app();
        app.import("invoice.pdf", INVOICE_PDF).unwrap().document.id
    };
    let app = workspace.app();
    let summary = app.summary(id(&doc_id)).unwrap();
    assert_eq!(summary.total_minor, Some(172_500));
}

#[test]
fn settings_validate_persist_and_change_extraction() {
    let workspace = Workspace::new("settings");
    let mut app = workspace.app();
    let defaults = app.settings().unwrap();
    assert_eq!((defaults.workspace_name.as_str(), defaults.default_currency.as_str()), ("My workspace", "USD"));
    assert!((defaults.review_threshold - 0.85).abs() < 1e-6);

    let mut bad = defaults.clone();
    bad.review_threshold = 0.2;
    assert!(app.update_settings(&bad).is_err());
    bad = defaults.clone();
    bad.default_currency = "XYZ".into();
    assert!(app.update_settings(&bad).is_err());
    bad = defaults.clone();
    bad.workspace_name = "  ".into();
    assert!(app.update_settings(&bad).is_err());

    let saved = app
        .update_settings(&ll_app::Settings {
            workspace_name: "  Northstar Studio ".into(),
            default_currency: "ghs".into(),
            review_threshold: 0.6,
        })
        .unwrap();
    assert_eq!((saved.workspace_name.as_str(), saved.default_currency.as_str()), ("Northstar Studio", "GHS"));
    drop(app);
    let app = workspace.app();
    assert_eq!(app.settings().unwrap(), saved, "settings survive a restart");
    assert!(app.system_info().unwrap().data_dir.is_some());
}

#[test]
fn corrections_are_kept_as_history() {
    let workspace = Workspace::new("history");
    let mut app = workspace.app();
    let doc = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    let doc_id = id(&doc.id);
    app.update_field(doc_id, &edit("vendor", "Kumasi Cloud")).unwrap();
    app.update_field(doc_id, &edit("vendor", "Kumasi Cloud")).unwrap(); // unchanged: not recorded
    let detail = app.update_field(doc_id, &edit("due_date", "")).unwrap();
    assert_eq!(detail.history.len(), 2);
    assert_eq!(detail.history[0].field, "vendor");
    assert_eq!(detail.history[0].old.as_deref(), Some("Kumasi Cloud Services Ltd"));
    assert_eq!(detail.history[0].old_source.map(|tier| tier.as_str()), Some("rules"));
    assert_eq!(detail.history[0].new.as_deref(), Some("Kumasi Cloud"));
    assert_eq!((detail.history[1].field.as_str(), detail.history[1].new.as_deref()), ("due_date", None));
}

#[test]
fn repeated_receipts_are_flagged_as_duplicates() {
    let workspace = Workspace::new("duplicates");
    let mut app = workspace.app();
    let first = app.import("invoice.pdf", INVOICE_PDF).unwrap().document;
    assert_eq!(first.duplicate_of, None);
    // The same invoice entered by hand from a photo: different bytes, same vendor and number.
    let second = app.import("photo.png", PNG).unwrap().document;
    let second_id = id(&second.id);
    for (field, value) in [
        ("vendor", "kumasi cloud services ltd"),
        ("invoice_number", "KCS-2026-0193"),
        ("doc_date", "2026-09-14"),
        ("currency", "GHS"),
        ("total", "1725.00"),
    ] {
        app.update_field(second_id, &edit(field, value)).unwrap();
    }
    let detail = app.detail(second_id).unwrap();
    assert_eq!(detail.summary.duplicate_of.as_deref(), Some(first.id.as_str()));
    assert!(detail.issues[0].message.contains("duplicate"));
    assert_eq!(app.summary(id(&first.id)).unwrap().duplicate_of, None, "only the later copy is flagged");
    assert_eq!(detail.summary.status, "needs_review");
    assert_eq!(app.approve_ready().unwrap(), 1, "bulk approval skips the suspected duplicate");
    assert_eq!(app.summary(second_id).unwrap().status, "needs_review");
}

#[test]
fn search_reads_document_text_and_ignores_query_syntax() {
    let workspace = Workspace::new("search");
    let mut app = workspace.app();
    let doc = app.import("kumasi.pdf", INVOICE_PDF).unwrap().document;
    let hits = app.search("backup stor").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, doc.id);
    assert!(hits[0].snippet.contains('\u{1}'), "matches are marked: {:?}", hits[0].snippet);
    assert_eq!(app.search("kumasi").unwrap().len(), 1, "file names are searchable");
    for hostile in ["\"", "NEAR(a b)", "text:* OR", "') DROP TABLE x; --", "***", ""] {
        assert!(app.search(hostile).is_ok(), "{hostile:?} must not break the query");
    }
    assert!(app.search("nothing-like-this").unwrap().is_empty());
    app.delete(id(&doc.id)).unwrap();
    assert!(app.search("backup").unwrap().is_empty(), "deleted documents leave the index");
}

#[test]
fn vendors_bulk_approval_and_export_plan() {
    let workspace = Workspace::new("vendors");
    let mut app = workspace.app();
    app.import("invoice.pdf", INVOICE_PDF).unwrap();
    let manual = app.import("blank.png", PNG).unwrap().document;
    app.update_field(id(&manual.id), &edit("vendor", "Corner Café")).unwrap();

    let vendors = app.vendors().unwrap();
    assert_eq!(vendors.len(), 2);
    assert_eq!(vendors[0].name, "Kumasi Cloud Services Ltd");
    assert_eq!(vendors[0].totals[0].total_minor, 172_500);
    assert_eq!(vendors[1].needs_review, 1);
    assert!(vendors[1].totals.is_empty(), "unreviewed spend is not counted");

    assert_eq!(app.approve_ready().unwrap(), 1, "only the validated invoice is approved");
    assert_eq!(app.summary(id(&manual.id)).unwrap().status, "needs_review");

    let out = workspace.dir.join("plan");
    fs::create_dir_all(&out).unwrap();
    let plan = app.export_plan(ExportFormat::Xero, &out).unwrap();
    assert_eq!(plan.rows, 1);
    assert!(!std::path::Path::new(&plan.path).exists(), "planning writes nothing");
    let written = app.export(ExportFormat::Xero, &out).unwrap();
    assert_eq!(written.path, plan.path);
}
