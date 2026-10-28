//! Field-level scoring of extracted amounts against CORD labels, plus whether review routing was
//! right: a document Ledgerlens marks ready (no review) should have every labelled field correct.

use crate::{cord::CordTruth, FieldCounts};
use serde::Serialize;
use std::collections::BTreeMap;

/// The amount fields scored, in report order.
pub const FIELDS: [&str; 3] = ["total", "subtotal", "tax"];

/// What Ledgerlens produced for one receipt, amounts in hundredths of the currency unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prediction {
    pub total: Option<i64>,
    pub subtotal: Option<i64>,
    pub tax: Option<i64>,
    pub items: Vec<i64>,
    pub routing: Routing,
}

/// Where the import left the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Routing {
    /// Validated cleanly: no review asked for.
    Ready,
    #[default]
    NeedsReview,
    /// Unreadable.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Correct,
    /// Labelled, extracted, different value.
    Wrong,
    /// Labelled, not extracted.
    Missed,
    /// Extracted, not labelled.
    Unlabelled,
    /// Neither.
    Absent,
}

/// One receipt's result, for the per-document detail file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentScore {
    pub fields: BTreeMap<&'static str, Outcome>,
    pub items_matched: usize,
    pub items_labelled: usize,
    pub items_extracted: usize,
    /// Every labelled field was extracted correctly.
    pub labelled_correct: bool,
    pub routing: Routing,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scorecard {
    pub fields: BTreeMap<&'static str, FieldCounts>,
    pub items: FieldCounts,
    pub documents: u32,
    pub labelled_correct: u32,
    pub ready: u32,
    pub ready_correct: u32,
    pub needs_review: u32,
    pub failed: u32,
}

fn outcome(truth: Option<i64>, predicted: Option<i64>) -> Outcome {
    match (truth, predicted) {
        (Some(truth), Some(predicted)) if truth == predicted => Outcome::Correct,
        (Some(_), Some(_)) => Outcome::Wrong,
        (Some(_), None) => Outcome::Missed,
        (None, Some(_)) => Outcome::Unlabelled,
        (None, None) => Outcome::Absent,
    }
}

impl Scorecard {
    pub fn add(&mut self, truth: &CordTruth, prediction: &Prediction) -> DocumentScore {
        let pairs = [
            ("total", truth.total, prediction.total),
            ("subtotal", truth.subtotal, prediction.subtotal),
            ("tax", truth.tax, prediction.tax),
        ];
        let mut fields = BTreeMap::new();
        for (name, truth, predicted) in pairs {
            let result = outcome(truth, predicted);
            let counts = self.fields.entry(name).or_default();
            // A wrong value is both a false positive (it was asserted) and a false negative (the
            // right value was not found).
            match result {
                Outcome::Correct => counts.true_positive += 1,
                Outcome::Wrong => {
                    counts.false_positive += 1;
                    counts.false_negative += 1;
                }
                Outcome::Missed => counts.false_negative += 1,
                Outcome::Unlabelled => counts.false_positive += 1,
                Outcome::Absent => {}
            }
            fields.insert(name, result);
        }

        // Line prices as multisets: each labelled price can be matched once.
        let mut remaining = truth.items.clone();
        let mut matched = 0;
        for price in &prediction.items {
            if let Some(position) = remaining.iter().position(|label| label == price) {
                remaining.swap_remove(position);
                matched += 1;
            }
        }
        self.items.true_positive += matched as u64;
        self.items.false_positive += (prediction.items.len() - matched) as u64;
        self.items.false_negative += (truth.items.len() - matched) as u64;

        let labelled_correct = fields
            .values()
            .all(|result| !matches!(result, Outcome::Wrong | Outcome::Missed));
        self.documents += 1;
        self.labelled_correct += u32::from(labelled_correct);
        match prediction.routing {
            Routing::Ready => {
                self.ready += 1;
                self.ready_correct += u32::from(labelled_correct);
            }
            Routing::NeedsReview => self.needs_review += 1,
            Routing::Failed => self.failed += 1,
        }
        DocumentScore {
            fields,
            items_matched: matched,
            items_labelled: truth.items.len(),
            items_extracted: prediction.items.len(),
            labelled_correct,
            routing: prediction.routing,
        }
    }

    /// The report section of a `results.json` run.
    pub fn report(&self) -> serde_json::Value {
        let metrics = |counts: FieldCounts| {
            serde_json::json!({
                "true_positive": counts.true_positive,
                "false_positive": counts.false_positive,
                "false_negative": counts.false_negative,
                "precision": counts.precision().map(round4),
                "recall": counts.recall().map(round4),
                "f1": counts.f1().map(round4),
            })
        };
        let fields: serde_json::Map<String, serde_json::Value> = FIELDS
            .iter()
            .map(|name| (name.to_string(), metrics(self.fields.get(name).copied().unwrap_or_default())))
            .collect();
        let share = |part: u32, whole: u32| (whole > 0).then(|| round4(f64::from(part) / f64::from(whole)));
        serde_json::json!({
            "fields": fields,
            "line_item_prices": metrics(self.items),
            "documents": {
                "count": self.documents,
                "all_labelled_fields_correct": self.labelled_correct,
                "ready_without_review": self.ready,
                "ready_and_correct": self.ready_correct,
                "ready_precision": share(self.ready_correct, self.ready),
                "needs_review": self.needs_review,
                "unreadable": self.failed,
            },
        })
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth() -> CordTruth {
        CordTruth { total: Some(3_850_000), subtotal: Some(3_500_000), tax: None, items: vec![2_500_000, 1_000_000, 1_000_000] }
    }

    #[test]
    fn wrong_values_count_against_precision_and_recall() {
        let mut card = Scorecard::default();
        let score = card.add(
            &truth(),
            &Prediction {
                total: Some(3_850_000),
                subtotal: Some(350_000),
                tax: Some(350_000),
                items: vec![1_000_000, 2_500_000, 9],
                routing: Routing::Ready,
            },
        );
        assert_eq!(score.fields["total"], Outcome::Correct);
        assert_eq!(score.fields["subtotal"], Outcome::Wrong);
        assert_eq!(score.fields["tax"], Outcome::Unlabelled);
        assert!(!score.labelled_correct);
        assert_eq!((score.items_matched, score.items_labelled, score.items_extracted), (2, 3, 3));

        let subtotal = card.fields["subtotal"];
        assert_eq!((subtotal.true_positive, subtotal.false_positive, subtotal.false_negative), (0, 1, 1));
        assert_eq!(card.fields["tax"].false_positive, 1);
        assert_eq!((card.items.true_positive, card.items.false_positive, card.items.false_negative), (2, 1, 1));
        assert_eq!((card.ready, card.ready_correct), (1, 0));
    }

    #[test]
    fn routing_precision_counts_only_ready_documents() {
        let mut card = Scorecard::default();
        let correct = Prediction { total: Some(3_850_000), subtotal: Some(3_500_000), ..Prediction::default() };
        card.add(&truth(), &Prediction { routing: Routing::Ready, ..correct.clone() });
        card.add(&truth(), &Prediction { routing: Routing::NeedsReview, ..correct });
        card.add(&truth(), &Prediction { routing: Routing::Failed, ..Prediction::default() });
        let report = card.report();
        assert_eq!(report["documents"]["count"], 3);
        assert_eq!(report["documents"]["all_labelled_fields_correct"], 2);
        assert_eq!(report["documents"]["ready_precision"], 1.0);
        assert_eq!(report["documents"]["unreadable"], 1);
        assert_eq!(report["fields"]["total"]["recall"], 0.6667);
        assert_eq!(report["fields"]["tax"]["f1"], serde_json::Value::Null);
    }
}
