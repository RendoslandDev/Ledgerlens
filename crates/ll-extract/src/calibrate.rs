//! Confidence calibration and review routing.
//!
//! Extractors report a raw score; that number is only useful if "0.9" really means "right about
//! 90% of the time". [`PlattScaler`] learns that mapping from labelled outcomes, and
//! [`expected_calibration_error`] measures how well it worked. Validation results then adjust
//! confidence further, and [`needs_review`] applies the review threshold.

use crate::validate::{totals_consistent, Issue, Severity};
use ll_core::{Confidence, ExtractedField, FieldName, Tier};

fn sigmoid(z: f64) -> f64 { 1.0 / (1.0 + (-z).exp()) }

fn logit(p: f64) -> f64 {
    let p = p.clamp(1e-4, 1.0 - 1e-4);
    (p / (1.0 - p)).ln()
}

/// `calibrated = sigmoid(a * logit(raw) + b)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlattScaler {
    pub a: f32,
    pub b: f32,
}

impl PlattScaler {
    /// Leaves scores untouched (used until enough labelled data exists to fit).
    pub const IDENTITY: PlattScaler = PlattScaler { a: 1.0, b: 0.0 };

    pub fn apply(&self, raw: Confidence) -> Confidence {
        if *self == Self::IDENTITY { return raw; }
        Confidence::new(sigmoid(self.a as f64 * logit(raw.get() as f64) + self.b as f64) as f32)
    }

    /// Fits `a, b` to `(raw score, was correct)` samples by Newton's method on log-loss, with a
    /// backtracking line search so a step never increases the loss. (When every sample shares one
    /// score the Hessian is nearly singular and raw Newton steps are enormous; clamping them
    /// component-wise bends them off the descent direction, so the line search does that job.) A
    /// weak ridge toward the identity keeps degenerate data (all right, or all wrong) finite.
    pub fn fit(samples: &[(f32, bool)]) -> PlattScaler {
        if samples.is_empty() { return Self::IDENTITY; }
        const LAMBDA: f64 = 1.0;
        let data: Vec<(f64, f64)> = samples.iter().map(|&(p, y)| (logit(p as f64), if y { 1.0 } else { 0.0 })).collect();
        // -log-likelihood is softplus(z) - y*z; written this way it cannot overflow.
        let softplus = |z: f64| if z > 0.0 { z + (-z).exp().ln_1p() } else { z.exp().ln_1p() };
        let loss = |a: f64, b: f64| -> f64 {
            LAMBDA / 2.0 * ((a - 1.0).powi(2) + b * b) + data.iter().map(|&(x, y)| softplus(a * x + b) - y * (a * x + b)).sum::<f64>()
        };

        let (mut a, mut b) = (1.0f64, 0.0f64);
        for _ in 0..100 {
            let (mut ga, mut gb) = (LAMBDA * (a - 1.0), LAMBDA * b);
            let (mut haa, mut hab, mut hbb) = (LAMBDA, 0.0, LAMBDA);
            for &(x, y) in &data {
                let p = sigmoid(a * x + b);
                let w = p * (1.0 - p);
                ga += (p - y) * x;
                gb += p - y;
                haa += w * x * x;
                hab += w * x;
                hbb += w;
            }
            let det = haa * hbb - hab * hab;
            if det.abs() < 1e-12 { break; }
            let da = (hbb * ga - hab * gb) / det;
            let db = (haa * gb - hab * ga) / det;

            let before = loss(a, b);
            let mut t = 1.0;
            while t > 1e-10 && loss(a - t * da, b - t * db) > before { t *= 0.5; }
            if t <= 1e-10 { break; } // no descent possible: at the optimum to machine precision
            a -= t * da;
            b -= t * db;
            if (t * da).abs() < 1e-9 && (t * db).abs() < 1e-9 { break; }
        }
        PlattScaler { a: a.clamp(0.05, 20.0) as f32, b: b.clamp(-10.0, 10.0) as f32 }
    }
}

/// Expected calibration error: the bin-weighted gap between mean confidence and accuracy.
/// 0 is perfectly calibrated.
pub fn expected_calibration_error(samples: &[(f32, bool)], bins: usize) -> f32 {
    if samples.is_empty() || bins == 0 { return 0.0; }
    let (mut conf, mut hits, mut n) = (vec![0.0f64; bins], vec![0.0f64; bins], vec![0usize; bins]);
    for &(p, y) in samples {
        let i = ((p.clamp(0.0, 1.0) as f64 * bins as f64) as usize).min(bins - 1);
        conf[i] += p as f64;
        hits[i] += if y { 1.0 } else { 0.0 };
        n[i] += 1;
    }
    let total = samples.len() as f64;
    let mut ece = 0.0;
    for i in 0..bins {
        if n[i] > 0 {
            let c = n[i] as f64;
            ece += (c / total) * ((conf[i] - hits[i]) / c).abs();
        }
    }
    ece as f32
}

/// Lets validation evidence move confidence: fields implicated in an `Error` are halved, in a
/// `Warning` shaved by 15%, and when subtotal + tax = total all three corroborate one another.
/// Human-entered values are never touched.
pub fn adjust_with_validation(fields: &mut [ExtractedField], issues: &[Issue]) {
    for issue in issues {
        let factor = match issue.severity { Severity::Error => 0.5, Severity::Warning => 0.85 };
        for name in &issue.fields {
            if let Some(f) = fields.iter_mut().find(|f| f.name == *name && f.tier != Tier::Human) {
                f.confidence = Confidence::new(f.confidence.get() * factor);
            }
        }
    }
    if totals_consistent(fields) {
        for f in fields.iter_mut() {
            if matches!(f.name, FieldName::Subtotal | FieldName::Tax | FieldName::Total) && f.tier != Tier::Human {
                let c = f.confidence.get();
                f.confidence = Confidence::new((c + (1.0 - c) * 0.5).min(0.99));
            }
        }
    }
}

/// Fields a person should look at: any non-human field below `threshold`, plus any `required`
/// field that is missing. Returned in canonical field order.
pub fn needs_review(fields: &[ExtractedField], required: &[FieldName], threshold: f32) -> Vec<FieldName> {
    FieldName::ALL
        .into_iter()
        .filter(|n| match fields.iter().find(|f| f.name == *n) {
            Some(f) => f.tier != Tier::Human && f.confidence.get() < threshold,
            None => required.contains(n),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_core::{Currency, FieldValue, Money};

    /// `n` samples at `raw`, of which `hits` were correct.
    fn cluster(raw: f32, n: usize, hits: usize) -> Vec<(f32, bool)> { (0..n).map(|i| (raw, i < hits)).collect() }

    #[test]
    fn identity_is_a_no_op() {
        for v in [0.0, 0.3, 0.999, 1.0] {
            assert_eq!(PlattScaler::IDENTITY.apply(Confidence::new(v)).get(), v);
        }
        assert_eq!(PlattScaler::fit(&[]), PlattScaler::IDENTITY);
    }

    #[test]
    fn fixes_overconfidence() {
        // says 0.95, is right 60% of the time
        let s = cluster(0.95, 100, 60);
        let scaler = PlattScaler::fit(&s);
        let p = scaler.apply(Confidence::new(0.95)).get();
        assert!((p - 0.60).abs() < 0.03, "calibrated {p}");

        let before = expected_calibration_error(&s, 10);
        let after = expected_calibration_error(&s.iter().map(|&(r, y)| (scaler.apply(Confidence::new(r)).get(), y)).collect::<Vec<_>>(), 10);
        assert!(before > 0.3 && after < 0.05, "ece {before} -> {after}");
    }

    #[test]
    fn fits_a_monotonic_curve_across_score_levels() {
        let mut s = Vec::new();
        for (raw, hits) in [(0.5, 30), (0.7, 55), (0.9, 80), (0.99, 95)] { s.extend(cluster(raw, 100, hits)); }
        let scaler = PlattScaler::fit(&s);
        let out: Vec<f32> = [0.5, 0.7, 0.9, 0.99].iter().map(|&r| scaler.apply(Confidence::new(r)).get()).collect();
        assert!(out.windows(2).all(|w| w[0] < w[1]), "{out:?}");
        for (got, want) in out.iter().zip([0.30, 0.55, 0.80, 0.95]) {
            assert!((got - want).abs() < 0.12, "{out:?}");
        }
    }

    #[test]
    fn degenerate_data_stays_finite() {
        for s in [cluster(0.9, 50, 50), cluster(0.9, 50, 0)] {
            let sc = PlattScaler::fit(&s);
            assert!(sc.a.is_finite() && sc.b.is_finite());
            let p = sc.apply(Confidence::new(0.9)).get();
            assert!((0.0..=1.0).contains(&p));
        }
        // all-correct data should not push confidence down
        assert!(PlattScaler::fit(&cluster(0.9, 50, 50)).apply(Confidence::new(0.9)).get() > 0.9);
    }

    #[test]
    fn ece_basics() {
        assert_eq!(expected_calibration_error(&[], 10), 0.0);
        let perfect: Vec<(f32, bool)> = (0..10).map(|i| (0.9, i < 9)).collect();
        assert!(expected_calibration_error(&perfect, 10) < 1e-6);
        let bad = cluster(1.0, 10, 0);
        assert!((expected_calibration_error(&bad, 10) - 1.0).abs() < 1e-6);
    }

    fn fld(name: FieldName, value: FieldValue, conf: f32, tier: Tier) -> ExtractedField {
        ExtractedField { name, value, confidence: Confidence::new(conf), tier }
    }
    fn usd(minor: i64) -> FieldValue { FieldValue::Money(Money::new(minor, Currency::USD)) }

    #[test]
    fn errors_halve_warnings_shave_and_humans_are_untouched() {
        let mut fs = vec![
            fld(FieldName::Subtotal, usd(100), 0.8, Tier::Rules),
            fld(FieldName::Tax, usd(10), 0.8, Tier::Rules),
            fld(FieldName::Total, usd(999), 0.8, Tier::Human),
        ];
        let issues = vec![
            Issue { code: "totals_mismatch", severity: Severity::Error, fields: vec![FieldName::Subtotal, FieldName::Tax, FieldName::Total], message: String::new() },
            Issue { code: "line_items_mismatch", severity: Severity::Warning, fields: vec![FieldName::Subtotal], message: String::new() },
            Issue { code: "missing_field", severity: Severity::Warning, fields: vec![FieldName::Vendor], message: String::new() },
        ];
        adjust_with_validation(&mut fs, &issues);
        assert!((fs[0].confidence.get() - 0.8 * 0.5 * 0.85).abs() < 1e-6);
        assert!((fs[1].confidence.get() - 0.4).abs() < 1e-6);
        assert_eq!(fs[2].confidence.get(), 0.8, "human value untouched");
    }

    #[test]
    fn consistent_totals_corroborate_each_other() {
        let mut fs = vec![
            fld(FieldName::Subtotal, usd(10000), 0.8, Tier::Rules),
            fld(FieldName::Tax, usd(1500), 0.8, Tier::Rules),
            fld(FieldName::Total, usd(11500), 0.8, Tier::Rules),
            fld(FieldName::Vendor, FieldValue::Text("A".into()), 0.8, Tier::Rules),
        ];
        adjust_with_validation(&mut fs, &[]);
        for f in &fs[..3] { assert!((f.confidence.get() - 0.9).abs() < 1e-6); }
        assert_eq!(fs[3].confidence.get(), 0.8, "unrelated fields do not gain");
    }

    #[test]
    fn review_routing() {
        let fs = vec![
            fld(FieldName::Vendor, FieldValue::Text("A".into()), 0.6, Tier::Rules),
            fld(FieldName::Total, usd(1), 0.95, Tier::Rules),
            fld(FieldName::Tax, usd(1), 0.1, Tier::Human),
            fld(FieldName::DueDate, FieldValue::Text("x".into()), 0.85, Tier::Rules),
        ];
        let r = needs_review(&fs, &[FieldName::Vendor, FieldName::InvoiceDate, FieldName::Total], 0.85);
        // vendor: low; invoice_date: required but missing; tax: human (never); due_date: exactly at threshold passes
        assert_eq!(r, [FieldName::Vendor, FieldName::InvoiceDate]);
        assert!(needs_review(&[], &[], 0.85).is_empty());
    }
}
