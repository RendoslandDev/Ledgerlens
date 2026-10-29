//! Cross-field validators. These catch extraction mistakes that no single field can reveal, such
//! as a subtotal and tax that do not add up to the total.

use ll_core::{Currency, Date, DocKind, ExtractedField, FieldName, FieldValue, LineItem, Money, Transaction};

/// Fields a usable invoice record must have; missing ones route the document to review.
pub const REQUIRED_FIELDS: [FieldName; 3] = [FieldName::Vendor, FieldName::InvoiceDate, FieldName::Total];
/// A statement can only be checked (and exported) with its bank, period end, and both balances.
pub const STATEMENT_FIELDS: [FieldName; 4] =
    [FieldName::Vendor, FieldName::PeriodEnd, FieldName::OpeningBalance, FieldName::ClosingBalance];
/// A contract is identified by who signed it and when it starts.
pub const CONTRACT_FIELDS: [FieldName; 2] = [FieldName::Parties, FieldName::DocDate];

/// The fields a document of this kind must have before it can leave review.
pub fn required_fields(kind: DocKind) -> &'static [FieldName] {
    match kind {
        DocKind::Statement => &STATEMENT_FIELDS,
        DocKind::Contract => &CONTRACT_FIELDS,
        DocKind::Invoice | DocKind::Receipt | DocKind::Other => &REQUIRED_FIELDS,
    }
}

/// [`validate`] for invoices and receipts, or the statement and contract checks.
pub fn validate_for(
    kind: DocKind,
    fields: &[ExtractedField],
    items: &[LineItem],
    transactions: &[Transaction],
    today: Date,
) -> Vec<Issue> {
    match kind {
        DocKind::Statement => validate_statement(fields, transactions),
        DocKind::Contract => validate_contract(fields),
        DocKind::Invoice | DocKind::Receipt | DocKind::Other => validate(fields, items, today),
    }
}

fn missing(fields: &[ExtractedField], required: &[FieldName], out: &mut Vec<Issue>) {
    for n in required {
        if val(fields, *n).is_none() {
            out.push(issue("missing_field", Severity::Warning, &[*n], format!("missing {}", n.as_str())));
        }
    }
}

/// Opening balance plus every transaction must equal the closing balance, and each printed
/// running balance must follow from the one before it. Either failing means a row was misread.
pub fn validate_statement(fields: &[ExtractedField], transactions: &[Transaction]) -> Vec<Issue> {
    let mut out = Vec::new();
    missing(fields, &STATEMENT_FIELDS, &mut out);
    if let (Some(start), Some(end)) = (date(fields, FieldName::PeriodStart), date(fields, FieldName::PeriodEnd))
        && start > end {
            out.push(issue("period_reversed", Severity::Error, &[FieldName::PeriodStart, FieldName::PeriodEnd],
                format!("period start {start} is after period end {end}")));
        }
    if transactions.is_empty() {
        out.push(issue("no_transactions", Severity::Warning, &[], "no transactions were found".into()));
    }
    let (opening, closing) = (money(fields, FieldName::OpeningBalance), money(fields, FieldName::ClosingBalance));
    let currencies_agree = |currency: Currency| transactions.iter().all(|t| t.amount.currency == currency);
    if let (Some(open), Some(close)) = (opening, closing) {
        if open.currency != close.currency || !currencies_agree(open.currency) {
            out.push(issue("currency_mismatch", Severity::Error, &[FieldName::OpeningBalance, FieldName::ClosingBalance],
                "balances and transactions disagree on currency".into()));
        } else {
            let sum = transactions.iter().try_fold(open.minor, |sum, t| sum.checked_add(t.amount.minor));
            match sum {
                Some(sum) if sum.abs_diff(close.minor) > 1 => out.push(issue(
                    "balance_mismatch",
                    Severity::Error,
                    &[FieldName::OpeningBalance, FieldName::ClosingBalance],
                    format!("opening balance {} plus transactions is {}, but closing balance is {}",
                        fmt(open), fmt(Money::new(sum, open.currency)), fmt(close)),
                )),
                None => out.push(issue("transactions_sum_overflow", Severity::Error, &[],
                    "transaction amounts exceed the supported integer range".into())),
                _ => {}
            }
        }
    }
    let mut previous = opening;
    for (row, t) in transactions.iter().enumerate() {
        if let (Some(before), Some(after)) = (previous, t.balance)
            && before.currency == after.currency
            && before.minor.checked_add(t.amount.minor).is_some_and(|expected| expected.abs_diff(after.minor) > 1) {
                out.push(issue("running_balance_mismatch", Severity::Error, &[],
                    format!("row {} ({} {}) does not lead to its balance {}", row + 1, t.date, t.description, fmt(after))));
                break;
            }
        previous = t.balance.or_else(|| previous.and_then(|b| b.checked_add(t.amount).ok()));
    }
    out
}

/// A contract cannot end before it starts.
pub fn validate_contract(fields: &[ExtractedField]) -> Vec<Issue> {
    let mut out = Vec::new();
    missing(fields, &CONTRACT_FIELDS, &mut out);
    if let (Some(start), Some(end)) = (date(fields, FieldName::DocDate), date(fields, FieldName::EndDate))
        && end < start {
            out.push(issue("end_before_start", Severity::Error, &[FieldName::DocDate, FieldName::EndDate],
                format!("end date {end} is before start date {start}")));
        }
    for n in [FieldName::DocDate, FieldName::EndDate] {
        if let Some(d) = date(fields, n).filter(|d| implausible(*d)) {
            out.push(issue("implausible_date", Severity::Error, &[n], format!("{} {d} is outside 1990-2100", n.as_str())));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity { Warning, Error }

/// One finding. `fields` names the fields the finding casts doubt on, so calibration can lower
/// their confidence.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub code: &'static str,
    pub severity: Severity,
    pub fields: Vec<FieldName>,
    pub message: String,
}

fn issue(code: &'static str, severity: Severity, fields: &[FieldName], message: String) -> Issue {
    Issue { code, severity, fields: fields.to_vec(), message }
}

fn val(fields: &[ExtractedField], n: FieldName) -> Option<&FieldValue> {
    fields.iter().find(|f| f.name == n).map(|f| &f.value)
}
fn money(fields: &[ExtractedField], n: FieldName) -> Option<Money> { val(fields, n).and_then(FieldValue::as_money) }
fn date(fields: &[ExtractedField], n: FieldName) -> Option<Date> { val(fields, n).and_then(FieldValue::as_date) }

/// True when subtotal + tax = total (within one minor unit) in a single currency.
pub fn totals_consistent(fields: &[ExtractedField]) -> bool {
    match (money(fields, FieldName::Subtotal), money(fields, FieldName::Tax), money(fields, FieldName::Total)) {
        (Some(s), Some(t), Some(g)) => {
            s.currency == t.currency
                && t.currency == g.currency
                && s.minor.checked_add(t.minor).is_some_and(|sum| sum.abs_diff(g.minor) <= 1)
        }
        _ => false,
    }
}

fn fmt(m: Money) -> String { format!("{} {}", m.to_decimal_string(), m.currency) }

fn implausible(d: Date) -> bool { !(1990..=2100).contains(&d.year) }

/// Runs every check. `today` is a parameter (not read from the clock) so results are reproducible.
pub fn validate(fields: &[ExtractedField], items: &[LineItem], today: Date) -> Vec<Issue> {
    let mut out = Vec::new();
    missing(fields, &REQUIRED_FIELDS, &mut out);

    let (sub, tax, total) = (money(fields, FieldName::Subtotal), money(fields, FieldName::Tax), money(fields, FieldName::Total));

    // currency consistency across amounts, and against a declared currency
    let money_fields: Vec<(FieldName, Money)> =
        [(FieldName::Subtotal, sub), (FieldName::Tax, tax), (FieldName::Total, total)].into_iter().filter_map(|(n, m)| m.map(|m| (n, m))).collect();
    let declared = val(fields, FieldName::Currency).and_then(FieldValue::as_text).and_then(|t| Currency::new(t).ok());
    let mixed = money_fields.windows(2).any(|w| w[0].1.currency != w[1].1.currency);
    let contradicts = declared.is_some_and(|d| money_fields.iter().any(|(_, m)| m.currency != d));
    if mixed || contradicts {
        let mut names: Vec<FieldName> = money_fields.iter().map(|(n, _)| *n).collect();
        if contradicts { names.push(FieldName::Currency); }
        out.push(issue("currency_mismatch", Severity::Error, &names, "amounts disagree on currency".into()));
    }

    // arithmetic
    if let (Some(s), Some(t), Some(g)) = (sub, tax, total) {
        if !mixed && !totals_consistent(fields) {
            out.push(issue(
                "totals_mismatch",
                Severity::Error,
                &[FieldName::Subtotal, FieldName::Tax, FieldName::Total],
                format!("subtotal {} + tax {} != total {}", fmt(s), fmt(t), fmt(g)),
            ));
        }
    } else if let (Some(s), None, Some(g)) = (sub, tax, total)
        && s.currency == g.currency && g.minor < s.minor {
            out.push(issue(
                "total_below_subtotal",
                Severity::Error,
                &[FieldName::Subtotal, FieldName::Total],
                format!("total {} is below subtotal {}", fmt(g), fmt(s)),
            ));
        }
    if let Some(g) = total
        && g.minor <= 0 {
            out.push(issue("non_positive_total", Severity::Warning, &[FieldName::Total], format!("total is {}", fmt(g))));
        }

    // line items should add up to the subtotal (or to the total when there is no tax to explain a gap)
    if !items.is_empty() {
        let target = match (sub, tax, total) {
            (Some(s), _, _) => Some((FieldName::Subtotal, s)),
            (None, None, Some(g)) => Some((FieldName::Total, g)),
            _ => None,
        };
        if let Some((name, t)) = target
            && items.iter().all(|i| i.amount.currency == t.currency) {
                let sum = items.iter().try_fold(0i64, |sum, item| sum.checked_add(item.amount.minor));
                match sum {
                    Some(sum) if sum.abs_diff(t.minor) > 1 => out.push(issue(
                        "line_items_mismatch",
                        Severity::Warning,
                        &[name],
                        format!("line items sum to {} but {} is {}", fmt(Money::new(sum, t.currency)), name.as_str(), fmt(t)),
                    )),
                    None => out.push(issue(
                        "line_items_sum_overflow",
                        Severity::Warning,
                        &[name],
                        "line item amounts exceed the supported integer range".into(),
                    )),
                    _ => {}
                }
            }
    }

    // dates
    let (inv, due) = (date(fields, FieldName::InvoiceDate), date(fields, FieldName::DueDate));
    for (n, d) in [(FieldName::InvoiceDate, inv), (FieldName::DueDate, due)] {
        if let Some(d) = d.filter(|d| implausible(*d)) {
            out.push(issue("implausible_date", Severity::Error, &[n], format!("{} {d} is outside 1990-2100", n.as_str())));
        }
    }
    if let Some(i) = inv
        && i.to_days() > today.to_days() + 1 {
            out.push(issue("future_date", Severity::Warning, &[FieldName::InvoiceDate], format!("invoice date {i} is in the future")));
        }
    if let (Some(i), Some(d)) = (inv, due)
        && d.to_days() < i.to_days() {
            out.push(issue("due_before_invoice", Severity::Error, &[FieldName::DueDate, FieldName::InvoiceDate], format!("due {d} is before invoice date {i}")));
        }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_core::{Confidence, Tier};

    fn f(name: FieldName, value: FieldValue) -> ExtractedField {
        ExtractedField { name, value, confidence: Confidence::new(0.9), tier: Tier::Rules }
    }
    fn m(minor: i64) -> FieldValue { FieldValue::Money(Money::new(minor, Currency::USD)) }
    fn d(y: i32, mo: u8, day: u8) -> FieldValue { FieldValue::Date(Date::new(y, mo, day).unwrap()) }
    fn today() -> Date { Date::new(2024, 6, 1).unwrap() }
    fn codes(v: &[Issue]) -> Vec<&'static str> { v.iter().map(|i| i.code).collect() }

    fn tx(day: u8, minor: i64, balance: Option<i64>) -> Transaction {
        Transaction {
            date: Date::new(2024, 3, day).unwrap(), description: "row".into(),
            amount: Money::new(minor, Currency::USD), balance: balance.map(|b| Money::new(b, Currency::USD)),
        }
    }
    fn statement(open: i64, close: i64) -> Vec<ExtractedField> {
        vec![
            f(FieldName::Vendor, FieldValue::Text("Bank".into())),
            f(FieldName::PeriodEnd, d(2024, 3, 31)),
            f(FieldName::OpeningBalance, m(open)),
            f(FieldName::ClosingBalance, m(close)),
        ]
    }

    #[test]
    fn statements_must_reconcile() {
        let rows = [tx(2, 5_000, Some(15_000)), tx(3, -2_000, Some(13_000))];
        assert!(validate_statement(&statement(10_000, 13_000), &rows).is_empty());
        assert_eq!(codes(&validate_statement(&statement(10_000, 14_000), &rows)), ["balance_mismatch"]);
        // a sign read the wrong way breaks the running balance too
        let flipped = [tx(2, 5_000, Some(15_000)), tx(3, 2_000, Some(13_000))];
        let found = codes(&validate_statement(&statement(10_000, 13_000), &flipped));
        assert!(found.contains(&"running_balance_mismatch") && found.contains(&"balance_mismatch"), "{found:?}");
        assert!(codes(&validate_statement(&statement(0, 0), &[])).contains(&"no_transactions"));
        let fields = vec![f(FieldName::Vendor, FieldValue::Text("Bank".into()))];
        assert_eq!(codes(&validate_statement(&fields, &rows)), ["missing_field", "missing_field", "missing_field"]);
    }

    #[test]
    fn contracts_cannot_end_before_they_start() {
        let fields = vec![
            f(FieldName::Parties, FieldValue::Text("A; B".into())),
            f(FieldName::DocDate, d(2026, 3, 1)),
            f(FieldName::EndDate, d(2026, 2, 1)),
        ];
        assert_eq!(codes(&validate_contract(&fields)), ["end_before_start"]);
        assert_eq!(codes(&validate_contract(&fields[..2])), Vec::<&str>::new());
        assert_eq!(required_fields(DocKind::Contract), &CONTRACT_FIELDS);
    }

    fn good() -> Vec<ExtractedField> {
        vec![
            f(FieldName::Vendor, FieldValue::Text("Acme".into())),
            f(FieldName::InvoiceDate, d(2024, 3, 5)),
            f(FieldName::DueDate, d(2024, 4, 4)),
            f(FieldName::Subtotal, m(12500)),
            f(FieldName::Tax, m(1875)),
            f(FieldName::Total, m(14375)),
        ]
    }

    #[test]
    fn clean_invoice_has_no_issues() {
        assert!(validate(&good(), &[], today()).is_empty());
        assert!(totals_consistent(&good()));
    }

    #[test]
    fn missing_required_fields_warn() {
        let v = validate(&[f(FieldName::Vendor, FieldValue::Text("Acme".into()))], &[], today());
        assert_eq!(codes(&v), ["missing_field", "missing_field"]);
        assert!(v.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn totals_mismatch_and_rounding_tolerance() {
        let mut fs = good();
        fs.retain(|x| x.name != FieldName::Total);
        fs.push(f(FieldName::Total, m(14000)));
        let v = validate(&fs, &[], today());
        assert_eq!(codes(&v), ["totals_mismatch"]);
        assert_eq!(v[0].severity, Severity::Error);
        assert!(v[0].message.contains("143.75") || v[0].message.contains("140.00"), "{}", v[0].message);

        // one minor unit of rounding is tolerated, two is not
        for (total, ok) in [(14376, true), (14374, true), (14377, false)] {
            let mut fs = good();
            fs.retain(|x| x.name != FieldName::Total);
            fs.push(f(FieldName::Total, m(total)));
            assert_eq!(validate(&fs, &[], today()).is_empty(), ok, "total {total}");
        }
    }

    #[test]
    fn total_arithmetic_overflow_is_mismatch_not_panic() {
        let mut fields = good();
        for (name, minor) in [(FieldName::Subtotal, i64::MAX), (FieldName::Tax, 1), (FieldName::Total, 0)] {
            fields.retain(|field| field.name != name);
            fields.push(f(name, FieldValue::Money(Money::new(minor, Currency::USD))));
        }
        assert!(!totals_consistent(&fields));
        assert!(validate(&fields, &[], today()).iter().any(|issue| issue.code == "totals_mismatch"));
    }

    #[test]
    fn line_item_sum_overflow_is_a_warning() {
        let fields = vec![
            f(FieldName::Subtotal, m(i64::MAX)),
            f(FieldName::Total, m(i64::MAX)),
        ];
        let item = |minor| LineItem {
            description: "x".into(),
            quantity: 1.0,
            unit_price: Money::new(minor, Currency::USD),
            amount: Money::new(minor, Currency::USD),
        };
        let issues = validate(&fields, &[item(i64::MAX), item(1)], today());
        assert!(issues.iter().any(|issue| issue.code == "line_items_sum_overflow"));
    }

    #[test]
    fn total_below_subtotal_when_tax_missing() {
        let fs = vec![
            f(FieldName::Vendor, FieldValue::Text("A".into())), f(FieldName::InvoiceDate, d(2024, 1, 1)),
            f(FieldName::Subtotal, m(10000)), f(FieldName::Total, m(9000)),
        ];
        assert_eq!(codes(&validate(&fs, &[], today())), ["total_below_subtotal"]);
    }

    #[test]
    fn line_items_checked_against_subtotal() {
        let item = |minor| LineItem { description: "x".into(), quantity: 1.0, unit_price: Money::new(minor, Currency::USD), amount: Money::new(minor, Currency::USD) };
        assert!(validate(&good(), &[item(10000), item(2500)], today()).is_empty());
        let v = validate(&good(), &[item(10000), item(2000)], today());
        assert_eq!(codes(&v), ["line_items_mismatch"]);
        assert_eq!(v[0].fields, [FieldName::Subtotal]);
        assert_eq!(v[0].severity, Severity::Warning);
    }

    #[test]
    fn line_items_vs_total_only_when_no_tax_to_explain_the_gap() {
        let item = |minor| LineItem { description: "x".into(), quantity: 1.0, unit_price: Money::new(minor, Currency::USD), amount: Money::new(minor, Currency::USD) };
        let base = vec![f(FieldName::Vendor, FieldValue::Text("A".into())), f(FieldName::InvoiceDate, d(2024, 1, 1))];
        // total only: items must equal it
        let mut only_total = base.clone();
        only_total.push(f(FieldName::Total, m(5000)));
        assert_eq!(codes(&validate(&only_total, &[item(4000)], today())), ["line_items_mismatch"]);
        // total + tax but no subtotal: a gap is expected, so no complaint
        let mut with_tax = only_total.clone();
        with_tax.push(f(FieldName::Tax, m(750)));
        assert!(validate(&with_tax, &[item(4250)], today()).is_empty());
    }

    #[test]
    fn date_checks() {
        let mut fs = good();
        fs.retain(|x| x.name != FieldName::DueDate);
        fs.push(f(FieldName::DueDate, d(2024, 3, 1)));
        assert_eq!(codes(&validate(&fs, &[], today())), ["due_before_invoice"]);

        let mut fs = good();
        fs.retain(|x| x.name != FieldName::InvoiceDate && x.name != FieldName::DueDate);
        fs.push(f(FieldName::InvoiceDate, d(2024, 6, 20)));
        assert_eq!(codes(&validate(&fs, &[], today())), ["future_date"]);
        // one day of slack for timezones
        let mut fs2 = good();
        fs2.retain(|x| x.name != FieldName::InvoiceDate && x.name != FieldName::DueDate);
        fs2.push(f(FieldName::InvoiceDate, d(2024, 6, 2)));
        assert!(validate(&fs2, &[], today()).is_empty());

        let mut fs = good();
        fs.retain(|x| x.name != FieldName::InvoiceDate && x.name != FieldName::DueDate);
        fs.push(f(FieldName::InvoiceDate, d(1924, 3, 5)));
        assert_eq!(codes(&validate(&fs, &[], today())), ["implausible_date"]);
    }

    #[test]
    fn currency_checks() {
        let mut fs = good();
        fs.retain(|x| x.name != FieldName::Tax);
        fs.push(f(FieldName::Tax, FieldValue::Money(Money::new(1875, Currency::EUR))));
        let v = validate(&fs, &[], today());
        assert_eq!(codes(&v), ["currency_mismatch"], "mixed currencies are reported once, not also as a totals mismatch");

        let mut fs = good();
        fs.push(f(FieldName::Currency, FieldValue::Text("GHS".into())));
        let v = validate(&fs, &[], today());
        assert_eq!(codes(&v), ["currency_mismatch"]);
        assert!(v[0].fields.contains(&FieldName::Currency));
    }

    #[test]
    fn non_positive_total_warns() {
        let fs = vec![
            f(FieldName::Vendor, FieldValue::Text("A".into())), f(FieldName::InvoiceDate, d(2024, 1, 1)),
            f(FieldName::Total, m(-500)),
        ];
        assert_eq!(codes(&validate(&fs, &[], today())), ["non_positive_total"]);
    }
}
