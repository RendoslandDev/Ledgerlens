use crate::{
    currency::Currency, document::{DocKind, Tier}, fields::{Confidence, FieldName},
    date::Date, geometry::BBox, money::Money,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineItem {
    pub description: String,
    pub quantity: f64,
    pub unit_price: Money,
    pub amount: Money,
}

/// One row of a statement. `amount` is signed from the account holder's side: money in is
/// positive, money out negative. `balance` is the running balance printed after it, if any.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub date: Date,
    pub description: String,
    pub amount: Money,
    pub balance: Option<Money>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource { Native, OcrNer, Vlm, User }

/// Provenance for one extracted value: powers click-to-highlight and the correction log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub field: FieldName,
    pub raw: String,
    pub confidence: Confidence,
    pub page_no: u32,
    pub bbox: Option<BBox>,
    pub source: EvidenceSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCode {
    TotalMismatch, LineSumMismatch, DateImplausible, CurrencyAmbiguous,
    MissingRequired, DuplicateSuspected, LowConfidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity { Info, Warn, Error }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub field: Option<FieldName>,
    pub code: IssueCode,
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordCandidate {
    pub kind: DocKind,
    pub tier: Tier,
    pub vendor: Option<String>,
    pub invoice_number: Option<String>,
    pub doc_date: Option<Date>,
    pub due_date: Option<Date>,
    pub currency: Option<Currency>,
    pub subtotal: Option<Money>,
    pub tax: Option<Money>,
    pub total: Option<Money>,
    pub category: Option<String>,
    pub line_items: Vec<LineItem>,
    pub evidence: Vec<Evidence>,
    pub issues: Vec<Issue>,
}

impl RecordCandidate {
    pub fn empty(kind: DocKind, tier: Tier) -> Self {
        Self { kind, tier, vendor: None, invoice_number: None, doc_date: None, due_date: None,
               currency: None, subtotal: None, tax: None, total: None, category: None,
               line_items: vec![], evidence: vec![], issues: vec![] }
    }

    /// Lowest confidence across critical fields. A critical field with no evidence counts as zero,
    /// so a record missing its total can never look "confident".
    pub fn min_critical_confidence(&self) -> Confidence {
        let mut min = Confidence::ONE;
        for f in FieldName::ALL.into_iter().filter(|f| f.is_critical(self.kind)) {
            let best = self.evidence.iter().filter(|e| e.field == f)
                .map(|e| e.confidence).fold(Confidence::ZERO, |a, b| if b > a { b } else { a });
            min = min.min(best);
        }
        min
    }

    pub fn has_blocking_issue(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
        //to. iterate through to find out if there is an error in the severuty/..
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(field: FieldName, c: f32) -> Evidence {
        Evidence { field, raw: String::new(), confidence: Confidence::new(c), page_no: 1, bbox: None, source: EvidenceSource::OcrNer }
    }
    #[test]
    fn missing_critical_field_counts_as_zero() {
        let mut r = RecordCandidate::empty(DocKind::Receipt, Tier::Fast);
        r.evidence = vec![ev(FieldName::Vendor, 0.9), ev(FieldName::DocDate, 0.95)]; // no Total
        assert_eq!(r.min_critical_confidence().get(), 0.0);
    }
    #[test]
    fn min_is_taken_over_critical_fields_only() {
        let mut r = RecordCandidate::empty(DocKind::Receipt, Tier::Fast);
        r.evidence = vec![ev(FieldName::Vendor, 0.9), ev(FieldName::DocDate, 0.8),
                          ev(FieldName::Total, 0.95), ev(FieldName::Category, 0.1)];
        assert_eq!(r.min_critical_confidence().get(), 0.8);
    }
    #[test]
    fn best_evidence_per_field_wins() {
        let mut r = RecordCandidate::empty(DocKind::Receipt, Tier::Fast);
        r.evidence = vec![ev(FieldName::Vendor, 0.4), ev(FieldName::Vendor, 0.9),
                          ev(FieldName::DocDate, 0.9), ev(FieldName::Total, 0.9)];
        assert_eq!(r.min_critical_confidence().get(), 0.9);
    }
    #[test]
    fn json_roundtrip() {
        let mut r = RecordCandidate::empty(DocKind::Invoice, Tier::Vlm);
        r.doc_date = Some(Date::new(2026, 3, 3).unwrap());
        r.total = Some(Money::new(1250, Currency::USD));
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RecordCandidate>(&j).unwrap(), r);
    }
}
