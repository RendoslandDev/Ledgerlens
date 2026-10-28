//! Orchestration: extractors -> calibrate -> merge -> validate -> adjust -> route -> persist.

use crate::calibrate::{adjust_with_validation, needs_review, PlattScaler};
use crate::rules;
use crate::validate::{validate, Issue, Severity, REQUIRED_FIELDS};
use ll_core::{
    Config, Confidence, Currency, Date, DocId, DocKind, DocStatus, ExtractedField, FieldName, FieldValue, LineItem, Result, Tier,
};
use ll_store::Store;
use std::collections::HashSet;

/// One extraction tier. The rules tier ships here; a model tier implements the same trait.
pub trait FieldExtractor: Send + Sync {
    fn name(&self) -> &str;
    fn extract(&self, text: &str, default_currency: Currency) -> Vec<ExtractedField>;
}

pub struct RulesExtractor;

impl FieldExtractor for RulesExtractor {
    fn name(&self) -> &str { "rules" }
    fn extract(&self, text: &str, default_currency: Currency) -> Vec<ExtractedField> {
        rules::extract_fields(text, default_currency)
    }
}

/// Everything learned about one document, before it is written anywhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Extraction {
    pub kind: DocKind,
    /// Final fields: calibrated, merged, and adjusted by validation. What a reviewer would see.
    pub fields: Vec<ExtractedField>,
    /// Calibrated and merged but *before* validation adjusted them. `persist` re-validates from
    /// here so that a human correction can lift penalties that a bad value had caused.
    pub merged: Vec<ExtractedField>,
    pub line_items: Vec<LineItem>,
    pub issues: Vec<Issue>,
    /// Fields a person should check (low confidence, or required and missing).
    pub review: Vec<FieldName>,
    pub status: DocStatus,
}

fn same_value(a: &FieldValue, b: &FieldValue) -> bool {
    match (a, b) {
        (FieldValue::Text(x), FieldValue::Text(y)) => x.trim().eq_ignore_ascii_case(y.trim()),
        _ => a == b,
    }
}

/// Picks one value per field from competing candidates. A `Human` value always wins; otherwise the
/// most confident wins (ties keep the earlier candidate), and other candidates that agree with it
/// corroborate it: confidence becomes `1 - (1-a)(1-b)...`, capped at 0.99.
pub fn merge(candidates: Vec<ExtractedField>) -> Vec<ExtractedField> {
    let mut out = Vec::new();
    for name in FieldName::ALL {
        let mut group: Vec<&ExtractedField> = candidates.iter().filter(|f| f.name == name).collect();
        if group.is_empty() { continue; }
        if let Some(h) = group.iter().filter(|f| f.tier == Tier::Human).max_by(|a, b| a.confidence.get().total_cmp(&b.confidence.get())) {
            out.push((*h).clone());
            continue;
        }
        group.sort_by(|a, b| b.confidence.get().total_cmp(&a.confidence.get())); // stable
        let mut best = group[0].clone();
        let mut miss = 1.0 - best.confidence.get();
        let mut agreed = false;
        for other in &group[1..] {
            if same_value(&other.value, &best.value) {
                miss *= 1.0 - other.confidence.get();
                agreed = true;
            }
        }
        if agreed { best.confidence = Confidence::new((1.0 - miss).min(0.99)); }
        out.push(best);
    }
    out
}

fn decide_status(review: &[FieldName], issues: &[Issue]) -> DocStatus {
    if review.is_empty() && !issues.iter().any(|i| i.severity == Severity::Error) {
        DocStatus::Extracted
    } else {
        DocStatus::NeedsReview
    }
}

pub struct Pipeline {
    extractors: Vec<Box<dyn FieldExtractor>>,
    scaler: PlattScaler,
    review_threshold: f32,
    default_currency: Currency,
    today: Date,
}

impl Pipeline {
    /// Rules tier only, identity calibration, today's date from the system clock.
    pub fn new(cfg: &Config) -> Result<Self> {
        cfg.validate()?;
        Ok(Pipeline {
            extractors: vec![Box::new(RulesExtractor)],
            scaler: PlattScaler::IDENTITY,
            review_threshold: cfg.pipeline.escalate_below,
            default_currency: Currency::new(&cfg.pipeline.default_currency)?,
            today: today_utc(),
        })
    }

    pub fn with_extractor(mut self, e: Box<dyn FieldExtractor>) -> Self { self.extractors.push(e); self }
    pub fn with_scaler(mut self, s: PlattScaler) -> Self { self.scaler = s; self }
    /// Pins "today" so validation is reproducible in tests and replays.
    pub fn with_today(mut self, d: Date) -> Self { self.today = d; self }

    /// Pure: text in, [`Extraction`] out. Nothing is written.
    pub fn run(&self, text: &str) -> Extraction {
        let kind = rules::classify_kind(text);
        let mut candidates = Vec::new();
        for e in &self.extractors {
            for mut f in e.extract(text, self.default_currency) {
                f.confidence = self.scaler.apply(f.confidence);
                candidates.push(f);
            }
        }
        let merged = merge(candidates);
        let mut fields = merged.clone();
        let cur = fields
            .iter()
            .find(|f| f.name == FieldName::Currency)
            .and_then(|f| f.value.as_text())
            .and_then(|t| Currency::new(t).ok())
            .unwrap_or(self.default_currency);
        let line_items = rules::extract_line_items(text, cur);
        let issues = validate(&fields, &line_items, self.today);
        adjust_with_validation(&mut fields, &issues);
        let review = needs_review(&fields, &REQUIRED_FIELDS, self.review_threshold);
        let status = decide_status(&review, &issues);
        Extraction { kind, fields, merged, line_items, issues, review, status }
    }

    /// Writes an extraction to the store and returns the document's resulting status.
    ///
    /// Values a person has entered (`Tier::Human`) are never overwritten. Validation is re-run over
    /// the machine values *plus* those human values, so correcting one field can clear the
    /// penalties it caused on others and take the document out of review. Approved documents are
    /// left completely alone. Fields stored by an earlier run that this run did not find are kept
    /// but do not influence the status.
    pub fn persist(&self, store: &mut Store, doc: DocId, ex: &Extraction) -> Result<DocStatus> {
        let current = store.get_document(doc)?;
        if current.status == DocStatus::Approved { return Ok(DocStatus::Approved); }

        let human: Vec<ExtractedField> = store.fields(doc)?.into_iter().filter(|f| f.tier == Tier::Human).collect();
        let human_names: HashSet<FieldName> = human.iter().map(|f| f.name).collect();
        let mut combined: Vec<ExtractedField> = ex.merged.iter().filter(|f| !human_names.contains(&f.name)).cloned().collect();
        combined.extend(human);

        store.replace_line_items(doc, &ex.line_items)?;
        let items = store.line_items(doc)?;
        let issues = validate(&combined, &items, self.today);
        adjust_with_validation(&mut combined, &issues);

        for f in combined.iter().filter(|f| f.tier != Tier::Human) { store.upsert_field(doc, f)?; }
        store.set_kind(doc, ex.kind)?;
        store.refresh_invoice(doc)?;

        let review = needs_review(&combined, &REQUIRED_FIELDS, self.review_threshold);
        let status = decide_status(&review, &issues);
        store.set_status(doc, status)?;
        Ok(status)
    }
}

/// Civil date (UTC) for a Unix timestamp; Howard Hinnant's days-to-civil algorithm.
pub fn date_from_unix(secs: i64) -> Date {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    Date::new(y, m, d).unwrap_or(Date { year: 1970, month: 1, day: 1 })
}

fn today_utc() -> Date { date_from_unix(ll_core::now_unix()) }

#[cfg(test)]
mod tests {
    use super::*;
    use ll_core::Money;
    use ll_ocr::{recognize_page, ScriptedEngine};
    use ll_render::Image;

    const CLEAN: &str = "ACME Trading Ltd\nINVOICE\nInvoice No: INV-2024-0042\nInvoice Date: March 5, 2024\nDue Date: 2024-04-04\n\
        Description\tQty\tUnit Price\tAmount\nWidget\t2\t$50.00\t$100.00\nGadget\t1\t$25.00\t$25.00\n\
        Subtotal\t$125.00\nVAT (15%)\t$18.75\nTotal\t$143.75";

    fn pipeline() -> Pipeline {
        Pipeline::new(&Config::default()).unwrap().with_today(Date::new(2024, 6, 1).unwrap())
    }

    fn get(ex: &Extraction, n: FieldName) -> Option<&ExtractedField> { ex.fields.iter().find(|f| f.name == n) }

    fn ocr(text: &str) -> String {
        let engine = ScriptedEngine::from_text(&[text]);
        recognize_page(&engine, &Image::filled(8, 8, 1, 255).unwrap()).unwrap().text()
    }

    #[test]
    fn clean_invoice_is_extracted_without_review() {
        let ex = pipeline().run(CLEAN);
        assert_eq!(ex.kind, DocKind::Invoice);
        assert!(ex.issues.is_empty(), "{:?}", ex.issues);
        assert!(ex.review.is_empty(), "{:?}", ex.review);
        assert_eq!(ex.status, DocStatus::Extracted);
        assert_eq!(ex.line_items.len(), 2);
        // subtotal + tax = total corroborated all three
        assert!(get(&ex, FieldName::Total).unwrap().confidence.get() > 0.9);
    }

    #[test]
    fn works_end_to_end_from_ocr_output() {
        let text = ocr(CLEAN);
        assert_eq!(text, CLEAN, "the OCR seam preserves lines and column gaps");
        assert_eq!(pipeline().run(&text), pipeline().run(CLEAN));
    }

    #[test]
    fn inconsistent_totals_route_to_review() {
        let bad = CLEAN.replace("Total\t$143.75", "Total\t$153.75");
        let ex = pipeline().run(&bad);
        assert_eq!(ex.status, DocStatus::NeedsReview);
        assert!(ex.issues.iter().any(|i| i.code == "totals_mismatch"));
        assert!(ex.review.contains(&FieldName::Total) && ex.review.contains(&FieldName::Subtotal));
    }

    #[test]
    fn unsure_vendor_or_missing_fields_route_to_review() {
        let ex = pipeline().run(&CLEAN.replace("ACME Trading Ltd", "Sunrise Bakery"));
        assert_eq!(ex.review, [FieldName::Vendor]);
        assert_eq!(ex.status, DocStatus::NeedsReview);

        let empty = pipeline().run("hello there\nnothing useful");
        assert_eq!(empty.status, DocStatus::NeedsReview);
        assert!(empty.review.contains(&FieldName::Total) && empty.review.contains(&FieldName::InvoiceDate));
    }

    #[test]
    fn calibration_can_push_fields_under_the_threshold() {
        // a scaler learned from an over-confident extractor demotes everything to review
        let scaler = PlattScaler::fit(&(0..100).map(|i| (0.9f32, i < 50)).collect::<Vec<_>>());
        let ex = pipeline().with_scaler(scaler).run(CLEAN);
        assert_eq!(ex.status, DocStatus::NeedsReview);
    }

    struct Stub(Vec<ExtractedField>);
    impl FieldExtractor for Stub {
        fn name(&self) -> &str { "stub" }
        fn extract(&self, _: &str, _: Currency) -> Vec<ExtractedField> { self.0.clone() }
    }
    fn cand(name: FieldName, v: &str, conf: f32, tier: Tier) -> ExtractedField {
        ExtractedField { name, value: FieldValue::Text(v.into()), confidence: Confidence::new(conf), tier }
    }

    #[test]
    fn merge_prefers_confident_corroborates_and_lets_humans_win() {
        let m = merge(vec![
            cand(FieldName::Vendor, "Acme", 0.6, Tier::Rules),
            cand(FieldName::Vendor, "ACME ", 0.7, Tier::Model), // agrees (case/space-insensitive)
            cand(FieldName::InvoiceNumber, "A1", 0.9, Tier::Rules),
            cand(FieldName::InvoiceNumber, "A7", 0.8, Tier::Model), // disagrees: no boost, lower loses
        ]);
        let vendor = m.iter().find(|f| f.name == FieldName::Vendor).unwrap();
        assert_eq!(vendor.tier, Tier::Model);
        assert!((vendor.confidence.get() - (1.0 - 0.3 * 0.4)).abs() < 1e-6);
        let inv = m.iter().find(|f| f.name == FieldName::InvoiceNumber).unwrap();
        assert_eq!(inv.value, FieldValue::Text("A1".into()));
        assert!((inv.confidence.get() - 0.9).abs() < 1e-6);

        let h = merge(vec![cand(FieldName::Vendor, "Machine", 0.99, Tier::Model), cand(FieldName::Vendor, "Person", 0.2, Tier::Human)]);
        assert_eq!(h[0].value, FieldValue::Text("Person".into()));
        assert_eq!(h[0].confidence.get(), 0.2, "human confidence is not rewritten");
    }

    #[test]
    fn corroboration_is_capped() {
        let m = merge(vec![cand(FieldName::Vendor, "A", 0.99, Tier::Rules), cand(FieldName::Vendor, "A", 0.99, Tier::Model)]);
        assert!((m[0].confidence.get() - 0.99).abs() < 1e-6);
    }

    #[test]
    fn extra_extractors_feed_the_merge() {
        let stub = Stub(vec![cand(FieldName::Vendor, "acme trading ltd", 0.8, Tier::Model)]);
        let ex = pipeline().with_extractor(Box::new(stub)).run(CLEAN);
        let v = get(&ex, FieldName::Vendor).unwrap();
        assert!(v.confidence.get() > 0.9, "rules (0.9) + model (0.8) agree");
    }

    fn insert(store: &Store, sha: &str) -> DocId { store.insert_document("/a.pdf", sha, DocKind::Other).unwrap().0.id }

    #[test]
    fn persists_and_marks_extracted() {
        let mut store = Store::open_in_memory().unwrap();
        let id = insert(&store, "h1");
        let p = pipeline();
        let status = p.persist(&mut store, id, &p.run(CLEAN)).unwrap();
        assert_eq!(status, DocStatus::Extracted);
        let doc = store.get_document(id).unwrap();
        assert_eq!((doc.status, doc.kind), (DocStatus::Extracted, DocKind::Invoice));
        assert_eq!(store.line_items(id).unwrap().len(), 2);
        let rows = store.invoices().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vendor.as_deref(), Some("ACME Trading Ltd"));
        assert_eq!(rows[0].total_minor, Some(14375));
        assert_eq!(rows[0].currency.as_deref(), Some("USD"));
        assert_eq!(rows[0].invoice_date, Some(Date::new(2024, 3, 5).unwrap()));
    }

    #[test]
    fn reprocessing_clears_line_items_when_none_are_found() {
        let mut store = Store::open_in_memory().unwrap();
        let id = insert(&store, "h4");
        let pipeline = pipeline();
        pipeline.persist(&mut store, id, &pipeline.run(CLEAN)).unwrap();
        assert_eq!(store.line_items(id).unwrap().len(), 2);

        pipeline.persist(&mut store, id, &pipeline.run("unstructured note")).unwrap();
        assert!(store.line_items(id).unwrap().is_empty());
    }

    #[test]
    fn human_corrections_survive_reextraction_and_clear_review() {
        let mut store = Store::open_in_memory().unwrap();
        let id = insert(&store, "h2");
        let p = pipeline();
        let bad = CLEAN.replace("Total\t$143.75", "Total\t$153.75");
        assert_eq!(p.persist(&mut store, id, &p.run(&bad)).unwrap(), DocStatus::NeedsReview);

        // reviewer fixes the total
        let fix = ExtractedField {
            name: FieldName::Total, value: FieldValue::Money(Money::new(14375, Currency::USD)),
            confidence: Confidence::ONE, tier: Tier::Human,
        };
        store.upsert_field(id, &fix).unwrap();
        // re-running the same bad extraction must not undo the correction, and the review clears
        assert_eq!(p.persist(&mut store, id, &p.run(&bad)).unwrap(), DocStatus::Extracted);
        let total = store.fields(id).unwrap().into_iter().find(|f| f.name == FieldName::Total).unwrap();
        assert_eq!((total.tier, total.value), (Tier::Human, fix.value));
        assert_eq!(store.invoices().unwrap()[0].total_minor, Some(14375));
    }

    #[test]
    fn approved_documents_are_not_touched() {
        let mut store = Store::open_in_memory().unwrap();
        let id = insert(&store, "h3");
        store.set_status(id, DocStatus::Approved).unwrap();
        let p = pipeline();
        assert_eq!(p.persist(&mut store, id, &p.run(CLEAN)).unwrap(), DocStatus::Approved);
        assert!(store.fields(id).unwrap().is_empty());
        assert!(p.persist(&mut store, DocId::new(), &p.run(CLEAN)).is_err(), "unknown document is an error");
    }

    #[test]
    fn pipeline_rejects_bad_config() {
        let mut cfg = Config::default();
        cfg.pipeline.escalate_below = 2.0;
        assert!(Pipeline::new(&cfg).is_err());
    }

    #[test]
    fn date_from_unix_matches_date_math() {
        assert_eq!(date_from_unix(0), Date::new(1970, 1, 1).unwrap());
        assert_eq!(date_from_unix(-1), Date::new(1969, 12, 31).unwrap());
        for (y, m, d) in [(2000, 2, 29), (2024, 3, 5), (2023, 12, 31), (1999, 1, 1), (2100, 3, 1), (1969, 7, 20)] {
            let date = Date::new(y, m, d).unwrap();
            assert_eq!(date_from_unix(date.to_days() * 86_400), date);
            assert_eq!(date_from_unix(date.to_days() * 86_400 + 86_399), date, "end of day stays on the same date");
        }
    }
}
