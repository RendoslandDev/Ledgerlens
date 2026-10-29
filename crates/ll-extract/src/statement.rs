//! Rules for bank and account statements: the header facts (bank, account, period, opening and
//! closing balances) and one [`Transaction`] per row.
//!
//! A row's direction (money in or out) comes from its running balance whenever one is printed:
//! the change from the previous balance must equal the row's amount. Only rows without that
//! evidence fall back to an explicit sign or a `CR`/`DR` marker. Validation then checks that the
//! opening balance plus every transaction equals the closing balance, so a misread row cannot slip
//! through unnoticed.

use crate::rules::{cells, date_re, detect_currency, fld, inv_date_re, find_date, last_amount, split_lines, vendor, DATE_PAT};
use ll_core::{Currency, Date, ExtractedField, FieldName, FieldValue, Money, Transaction};
use regex::Regex;
use std::sync::OnceLock;

macro_rules! lazy_re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static C: OnceLock<Regex> = OnceLock::new();
            C.get_or_init(|| {
                let p: String = $pat.to_string();
                Regex::new(&p).unwrap_or_else(|e| panic!("bad regex {p}: {e}"))
            })
        }
    };
}

/// Longest description kept for one row, after joining wrapped lines.
const MAX_DESCRIPTION_CHARS: usize = 200;

lazy_re!(account_re, r"(?i)\b(?:account|acct|a/c)\b\.?\s*(?:number|no\b\.?|num\b\.?|#)?\s*[:#]?\s*([0-9][0-9 \-*xX•]{2,30}[0-9])");
lazy_re!(period_re, format!(r"(?i)\b(?:period|from|for)\b[^\d\n]{{0,24}}?({DATE_PAT})\s*(?:-|–|—|to|through|thru|until)\s*({DATE_PAT})"));
lazy_re!(opening_re, r"(?i)\b(?:opening|beginning|starting|previous|start)\s+balance\b|\bbalance\s+(?:brought\s+forward|b/f|forward)\b|\bbrought\s+forward\b");
lazy_re!(closing_re, r"(?i)\b(?:closing|ending|new|final|end)\s+balance\b|\bbalance\s+(?:carried\s+forward|c/f)\b|\bcarried\s+forward\b");
// The column header of the transactions table.
lazy_re!(header_re, r"(?i)\bdate\b.*\b(?:description|details|particulars|narrative|transaction|payee|memo|reference)\b");
// A row's leading date: a full date, or day and month without a year ("03/02", "3 Feb", "Feb 3").
lazy_re!(row_date_re, format!(r"^\s*({DATE_PAT}|\d{{1,2}}[/.\-]\d{{1,2}}|\d{{1,2}}\s+[A-Za-z]{{3,9}}\.?|[A-Za-z]{{3,9}}\.?\s+\d{{1,2}})\b"));
// One cell that is only an amount, with an optional sign, symbol, or credit/debit marker.
lazy_re!(amount_cell_re, r"(?i)^(?:[-−]|\()?\s?(?:US\$|GH₵|(?:USD|EUR|GBP|GHS|NGN|KES|ZAR|CAD|AUD|JPY|CHF|INR|CNY|IDR)\s?|[$€£₵])?\s?\d[\d,.' ]*\)?\s*(?:cr|dr|od|-)?$");
// "03/02", "03/02/2026", "03.02.2026"; not "1.25", which is an amount.
lazy_re!(numeric_date_re, r"\b(\d{1,2})(?:/(\d{1,2})(?:/\d{2,4})?|[.\-](\d{1,2})[.\-]\d{2,4})\b");
lazy_re!(bank_name_re, r"(?i)\b(?:bank|banque|credit\s+union|building\s+society|savings|trust\s+company)\b");

/// Day-first (`03/02` = 3 February) or month-first. Read from the document itself: a first number
/// above 12 settles it; failing that, the currency's usual convention.
fn day_first(text: &str, currency: Currency) -> bool {
    for caps in numeric_date_re().captures_iter(text) {
        let second = caps.get(2).or(caps.get(3)).map_or("0", |m| m.as_str());
        let (a, b): (u32, u32) = (caps[1].parse().unwrap_or(0), second.parse().unwrap_or(0));
        if a > 12 && b <= 12 { return true; }
        if b > 12 && a <= 12 { return false; }
    }
    !matches!(currency.code(), "USD" | "CAD")
}

/// Parses a date that may lack its year; `year_for` picks the year from the month.
fn parse_date(raw: &str, day_first: bool, year_for: impl Fn(u8) -> Option<i32>) -> Option<Date> {
    let parts: Vec<&str> = raw.split(|c: char| !c.is_ascii_alphanumeric()).filter(|p| !p.is_empty()).collect();
    let numeric = parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()));
    if !numeric {
        // Month names are unambiguous; add the year when it is missing.
        if let Ok(date) = Date::parse(raw) { return Some(date); }
        let month = Date::parse(&format!("{raw} 2000")).ok()?.month;
        return Date::parse(&format!("{raw} {}", year_for(month)?)).ok();
    }
    let number = |i: usize| parts.get(i).and_then(|p| p.parse::<i32>().ok());
    match parts.len() {
        3 if parts[0].len() == 4 => Date::new(number(0)?, number(1)? as u8, number(2)? as u8),
        2 | 3 => {
            let (day, month) = if day_first { (number(0)?, number(1)?) } else { (number(1)?, number(0)?) };
            let month = u8::try_from(month).ok()?;
            let year = match number(2) {
                Some(y) if parts[2].len() == 2 => if y < 70 { y + 2000 } else { y + 1900 },
                Some(y) => y,
                None => year_for(month)?,
            };
            Date::new(year, month, u8::try_from(day).ok()?)
        }
        _ => None,
    }
}

/// An amount cell's value, with its sign when the cell states one (`-`, brackets, `DR`, `OD`
/// mean money out or an overdrawn balance; `CR` means money in). `None` sign means unstated.
fn amount_cell(cell: &str, currency: Currency) -> Option<(Money, Option<bool>)> {
    let cell = cell.trim();
    if !amount_cell_re().is_match(cell) || date_re().is_match(cell) { return None; }
    let lower = cell.to_ascii_lowercase();
    let marker = if lower.ends_with("cr") { Some(true) }
        else if lower.ends_with("dr") || lower.ends_with("od") || lower.ends_with('-') { Some(false) }
        else { None };
    let body = lower.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '-' || c == ' ');
    let negative = body.starts_with(['-', '−', '(']);
    let digits: String = body.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',')).collect();
    let magnitude = Money::parse(&digits, currency).ok()?;
    let sign = if negative { Some(false) } else { marker };
    Some((Money::new(magnitude.minor.abs(), currency), sign))
}

fn signed(money: Money, positive: bool) -> Money {
    Money::new(if positive { money.minor } else { -money.minor }, money.currency)
}

/// A balance written on a labelled line ("Closing balance  1,250.00 CR").
fn labelled_balance(lines: &[&str], re: &Regex, currency: Currency) -> Option<Money> {
    for line in lines {
        let Some(m) = re.find(line) else { continue };
        let rest = &line[m.end()..];
        let from_cell = rest.split('\t').rev().find_map(|cell| amount_cell(cell, currency));
        if let Some((money, sign)) = from_cell {
            return Some(signed(money, sign.unwrap_or(true)));
        }
        if let Some(money) = last_amount(rest, currency) { return Some(money); }
    }
    None
}

/// Splits a row into its description and its trailing amount cells. Text separated by single
/// spaces (no column gaps) is split on spaces instead.
fn split_row(rest: &str, currency: Currency) -> (String, Vec<(Money, Option<bool>)>) {
    let mut parts: Vec<String> = cells(rest).into_iter().map(String::from).collect();
    if parts.len() == 1 {
        parts = rest.split_whitespace().map(String::from).collect();
    }
    let mut amounts = Vec::new();
    while let Some(last) = parts.last() {
        match amount_cell(last, currency) {
            Some(amount) => { amounts.push(amount); parts.pop(); }
            None => break,
        }
    }
    amounts.reverse();
    (parts.join(" "), amounts)
}

/// Everything the rules can read from a statement.
pub fn extract_statement(text: &str, default_currency: Currency) -> (Vec<ExtractedField>, Vec<Transaction>) {
    let lines = split_lines(text);
    let detected = detect_currency(text);
    let currency = detected.map_or(default_currency, |(c, _)| c);
    let day_first = day_first(text, currency);
    let mut out = Vec::new();

    if let Some((bank, conf)) = vendor(&lines) {
        // A heading that names a bank is the issuer, not a stray line.
        let conf = if bank_name_re().is_match(&bank) { conf.max(0.9) } else { conf };
        out.push(fld(FieldName::Vendor, FieldValue::Text(bank), conf));
    }
    if let Some(caps) = lines.iter().find_map(|line| account_re().captures(line)) {
        let digits: String = caps[1].chars().filter(char::is_ascii_digit).collect();
        if digits.len() >= 4 {
            // Only the last four digits are kept: enough to tell accounts apart.
            let masked = format!("••••{}", &digits[digits.len() - 4..]);
            out.push(fld(FieldName::AccountNumber, FieldValue::Text(masked), 0.9));
        }
    }

    // The period, read once without a year guess, anchors the year of rows that omit it.
    let mut period: Option<(Date, Date)> = None;
    for line in &lines {
        let Some(caps) = period_re().captures(line) else { continue };
        let no_year = |_: u8| None;
        if let (Some(start), Some(end)) = (parse_date(&caps[1], day_first, no_year), parse_date(&caps[2], day_first, no_year))
            && start <= end {
                period = Some((start, end));
                break;
            }
    }
    if let Some((start, end)) = period {
        out.push(fld(FieldName::PeriodStart, FieldValue::Date(start), 0.9));
        out.push(fld(FieldName::PeriodEnd, FieldValue::Date(end), 0.9));
    }
    if let Some((date, conf)) = find_date(&lines, inv_date_re(), true) {
        out.push(fld(FieldName::DocDate, FieldValue::Date(date), conf));
    }
    let anchor = period.map(|(_, end)| end).or_else(|| {
        out.iter().find(|f| f.name == FieldName::DocDate).and_then(|f| f.value.as_date())
    });
    let year_for = |month: u8| -> Option<i32> {
        match (period, anchor) {
            // A period that crosses New Year: months from the start onward belong to its year.
            (Some((start, end)), _) if start.year != end.year => {
                Some(if month >= start.month { start.year } else { end.year })
            }
            (_, Some(anchor)) => Some(if month > anchor.month { anchor.year - 1 } else { anchor.year }),
            _ => None,
        }
    };

    let mut opening = labelled_balance(&lines, opening_re(), currency);
    let closing = labelled_balance(&lines, closing_re(), currency);

    let start = lines.iter().position(|line| header_re().is_match(line)).map_or(0, |i| i + 1);
    let mut transactions: Vec<Transaction> = Vec::new();
    let mut previous_balance = opening;
    let mut continuing = false;
    for line in &lines[start..] {
        if line.trim().is_empty() { continuing = false; continue; }
        if opening_re().is_match(line) || closing_re().is_match(line) { continuing = false; continue; }
        let Some(caps) = row_date_re().captures(line) else {
            // A wrapped description line: no date and no amounts, right after a row.
            if continuing && let Some(last) = transactions.last_mut() {
                let (extra, amounts) = split_row(line, currency);
                if amounts.is_empty() && last.description.chars().count() + extra.len() < MAX_DESCRIPTION_CHARS {
                    last.description = format!("{} {}", last.description, extra.trim());
                    continue;
                }
            }
            continuing = false;
            continue;
        };
        let Some(date) = parse_date(&caps[1], day_first, year_for) else { continuing = false; continue };
        let (description, amounts) = split_row(&line[caps.get(0).map_or(0, |m| m.end())..], currency);
        if description.trim().is_empty() || amounts.is_empty() { continuing = false; continue; }

        let (moved, balance) = match amounts.as_slice() {
            [only] => (*only, None),
            [.., amount, balance] => {
                // "Debit  Credit  Balance" with both filled: the non-zero one is the movement.
                let movements = &amounts[..amounts.len() - 1];
                let amount = movements.iter().rev().find(|(m, _)| m.minor != 0).copied().unwrap_or(*amount);
                (amount, Some(signed(balance.0, balance.1.unwrap_or(true))))
            }
            [] => unreachable!("rows without amounts were skipped"),
        };
        let (magnitude, stated) = moved;
        let from_balance = match (previous_balance, balance) {
            (Some(before), Some(after)) => after.checked_sub(before).ok()
                .filter(|delta| delta.minor.abs() == magnitude.minor)
                .map(|delta| delta.minor >= 0),
            _ => None,
        };
        // Unstated and unprovable rows count as money out; reconciliation catches a wrong guess.
        let amount = signed(magnitude, from_balance.or(stated).unwrap_or(false));
        if opening.is_none() && transactions.is_empty() && let Some(after) = balance
            && (from_balance.is_some() || stated.is_some()) {
                opening = after.checked_sub(amount).ok();
            }
        previous_balance = balance.or_else(|| previous_balance.and_then(|b| b.checked_add(amount).ok()));
        transactions.push(Transaction { date, description: description.trim().to_string(), amount, balance });
        continuing = true;
    }

    if let Some(money) = opening { out.push(fld(FieldName::OpeningBalance, FieldValue::Money(money), 0.9)); }
    if let Some(money) = closing { out.push(fld(FieldName::ClosingBalance, FieldValue::Money(money), 0.9)); }
    if let Some((c, conf)) = detected {
        out.push(fld(FieldName::Currency, FieldValue::Text(c.code().to_string()), conf));
    }
    (out, transactions)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANK: &str = "Kumasi Community Bank\nStatement of Account\nAccount No: 0012-3456-7890\n\
        Statement period: 01/03/2026 to 31/03/2026\nCurrency: GHS\n\
        Date\tDescription\tDebit\tCredit\tBalance\n\
        01/03\tOpening balance\t\t\t1,000.00\n\
        02/03\tSalary March\t\t2,500.00\t3,500.00\n\
        05/03\tRent payment to\t1,200.00\t\t2,300.00\n\
        Landlord Ltd\n\
        14/03\tMTN Mobile Money\t45.50\t\t2,254.50\n\
        28/03\tInterest\t\t1.25\t2,255.75\n\
        Closing balance\t\t\t2,255.75";

    fn get(fields: &[ExtractedField], name: FieldName) -> Option<&FieldValue> {
        fields.iter().find(|f| f.name == name).map(|f| &f.value)
    }
    fn ghs(minor: i64) -> Money { Money::new(minor, Currency::GHS) }

    #[test]
    fn reads_header_facts_and_signs_rows_from_the_running_balance() {
        let (fields, rows) = extract_statement(BANK, Currency::USD);
        assert_eq!(get(&fields, FieldName::Vendor), Some(&FieldValue::Text("Kumasi Community Bank".into())));
        assert_eq!(get(&fields, FieldName::AccountNumber), Some(&FieldValue::Text("••••7890".into())));
        assert_eq!(get(&fields, FieldName::PeriodStart), Some(&FieldValue::Date(Date::new(2026, 3, 1).unwrap())));
        assert_eq!(get(&fields, FieldName::PeriodEnd), Some(&FieldValue::Date(Date::new(2026, 3, 31).unwrap())));
        assert_eq!(get(&fields, FieldName::OpeningBalance), Some(&FieldValue::Money(ghs(100_000))));
        assert_eq!(get(&fields, FieldName::ClosingBalance), Some(&FieldValue::Money(ghs(225_575))));
        let amounts: Vec<i64> = rows.iter().map(|r| r.amount.minor).collect();
        assert_eq!(amounts, [250_000, -120_000, -4_550, 125]);
        assert_eq!(rows[1].description, "Rent payment to Landlord Ltd", "wrapped lines are joined");
        assert_eq!(rows[0].date, Date::new(2026, 3, 2).unwrap(), "day-first dates take the period's year");
    }

    #[test]
    fn signed_amount_column_without_balances() {
        let text = "First Atlantic Bank\nAccount statement\nPeriod: Jan 1, 2026 - Jan 31, 2026\n\
            Date\tDetails\tAmount (USD)\n\
            Jan 3\tCoffee Roasters\t-12.40\n\
            Jan 9\tRefund from Acme\t30.00\n\
            Jan 12\tCard fee\t2.00 DR";
        let (fields, rows) = extract_statement(text, Currency::USD);
        assert!(get(&fields, FieldName::OpeningBalance).is_none());
        let amounts: Vec<i64> = rows.iter().map(|r| r.amount.minor).collect();
        // An unsigned amount with no balance to prove it is counted as money out.
        assert_eq!(amounts, [-1_240, -3_000, -200]);
        assert_eq!(rows[0].date, Date::new(2026, 1, 3).unwrap());
    }

    #[test]
    fn rows_before_new_year_take_the_earlier_year() {
        let text = "Bank\nStatement period 15/12/2025 to 14/01/2026\nDate\tDescription\tAmount\tBalance\n\
            Balance brought forward\t100.00\n20/12\tShop\t10.00\t90.00\n05/01\tPay\t50.00\t140.00";
        let (_, rows) = extract_statement(text, Currency::GBP);
        assert_eq!(rows[0].date, Date::new(2025, 12, 20).unwrap());
        assert_eq!(rows[1].date, Date::new(2026, 1, 5).unwrap());
        assert_eq!(rows[1].amount.minor, 5_000);
    }

    #[test]
    fn month_first_when_the_document_says_so() {
        assert!(!day_first("Period 03/01/2026 to 03/31/2026", Currency::GBP));
        assert!(day_first("Period 01/03/2026 to 31/03/2026", Currency::USD));
        assert!(!day_first("no numeric dates", Currency::USD));
    }

    #[test]
    fn amount_cells_and_markers() {
        let usd = Currency::USD;
        assert_eq!(amount_cell("1,250.00 CR", usd), Some((Money::new(125_000, usd), Some(true))));
        assert_eq!(amount_cell("(45.00)", usd), Some((Money::new(4_500, usd), Some(false))));
        assert_eq!(amount_cell("$9.99", usd), Some((Money::new(999, usd), None)));
        assert_eq!(amount_cell("Invoice 42", usd), None);
        assert_eq!(amount_cell("03/04/2026", usd), None);
    }
}
