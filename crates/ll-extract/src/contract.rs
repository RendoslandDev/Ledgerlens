//! Rules for contracts: who the parties are, when it starts and ends, how much notice ending it
//! takes, whether it renews by itself, and what it is worth. These are the facts a person needs
//! to avoid an unwanted renewal; clause-by-clause review is out of scope.

use crate::rules::{detect_currency, fld, last_amount, split_lines, DATE_PAT};
use ll_core::{Currency, Date, ExtractedField, FieldName, FieldValue};
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

const MAX_PARTY_CHARS: usize = 80;
const NUMBER_WORDS: &str = "one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|fourteen|fifteen|eighteen|twenty|twenty-four|thirty|thirty-six|forty-five|sixty|ninety";

lazy_re!(between_re, r"(?is)\bbetween\s*:?\s+(.{2,300}?)\s+and\s+(.{2,300}?)(?:\.\s|\.$|\n\n|$)");
// "Client: Acme Ltd", "Landlord - Ama Mensah": labelled parties, one per line.
lazy_re!(party_label_re, r"(?i)^\s*(?:party\s+[a-b12]|client|customer|provider|service\s+provider|supplier|vendor|contractor|consultant|landlord|tenant|lessor|lessee|licensor|licensee|employer|employee|buyer|seller|company)\s*[:\-]\s*(.{2,80}?)\s*$");
lazy_re!(start_re, format!(r"(?i)\b(?:effective\s+(?:date|as\s+of|from|on)|commencement\s+date|commenc\w*\s+on|start\s+date|starts?\s+on|dated(?:\s+as\s+of)?|made\s+(?:on|as\s+of)|entered\s+into\s+(?:on|as\s+of))\b\W{{0,6}}(?:the\s+)?({DATE_PAT})"));
lazy_re!(day_of_re, r"(?i)\b(?:this|on\s+the)\s+(\d{1,2})(?:st|nd|rd|th)?\s+day\s+of\s+([A-Za-z]{3,9}),?\s+(\d{4})");
lazy_re!(end_re, format!(r"(?i)\b(?:end\s+date|expir\w*(?:\s+date)?|terminat\w*\s+on|ends?\s+on|until|through|up\s+to\s+and\s+including)\b\W{{0,6}}(?:on\s+)?(?:the\s+)?({DATE_PAT})"));
lazy_re!(term_re, format!(r"(?i)\b(?:term|period|duration)\s+of\s+(\d{{1,3}}|{NUMBER_WORDS})\s*(?:\(\d{{1,3}}\)\s*)?(day|week|month|year)s?\b"));
lazy_re!(notice_re, format!(r"(?i)\b(\d{{1,3}}|{NUMBER_WORDS})\s*(?:\(\d{{1,3}}\)\s*)?(day|week|month)(?:s['’]?|['’]s)?\s+(?:(?:prior|advance|written|advance\s+written|prior\s+written)\s+)?notice\b|\bnotice\s+(?:period\s+)?of\s+(?:at\s+least\s+|not\s+less\s+than\s+)?(\d{{1,3}}|{NUMBER_WORDS})\s*(?:\(\d{{1,3}}\)\s*)?(day|week|month)s?\b"));
lazy_re!(renew_yes_re, r"(?i)\b(?:automatically\s+renew|renew\s+automatically|auto[-\s]?renew|shall\s+be\s+renewed\s+automatically|renew\s+for\s+(?:successive|additional|further)|evergreen)\w*");
lazy_re!(renew_no_re, r"(?i)\b(?:shall|will)\s+not\s+(?:be\s+)?(?:automatically\s+)?renew|\bno\s+automatic\s+renewal\b|\bnot\s+renew\s+automatically\b");
lazy_re!(value_re, r"(?i)\b(?:total\s+(?:contract\s+)?(?:value|price|fee|fees|amount|consideration)|contract\s+(?:value|price|sum)|(?:annual|monthly|fixed)\s+(?:fee|rent|price)|rent\s+of|fee\s+of|sum\s+of|price\s+of)\b");
lazy_re!(money_hint_re, r"(?:US\$|GH₵|[$€£₵]|\b(?:USD|EUR|GBP|GHS|NGN|KES|ZAR|CAD|AUD|JPY|CHF|INR|CNY|IDR)\b|\d[.,]\d{2}\b)");

fn number(word: &str) -> Option<u32> {
    if let Ok(n) = word.parse() { return Some(n); }
    let names: Vec<&str> = NUMBER_WORDS.split('|').collect();
    let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 18, 20, 24, 30, 36, 45, 60, 90];
    names.iter().position(|name| name.eq_ignore_ascii_case(word)).map(|i| values[i])
}

/// "Acme Ltd, a company registered in Ghana (the "Client")" -> "Acme Ltd".
fn clean_party(raw: &str) -> Option<String> {
    let raw = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = raw.to_ascii_lowercase();
    let cut = [" (", ",", ";", " whose ", " having ", " with its ", " located ", " registered ", " hereinafter"]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .unwrap_or(raw.len());
    let name = raw[..cut].trim().trim_matches(|c: char| matches!(c, '"' | '“' | '”' | '\'' | ':')).trim();
    let name = name.strip_prefix("the ").or_else(|| name.strip_prefix("The ")).unwrap_or(name);
    let letters = name.chars().filter(|c| c.is_alphabetic()).count();
    (letters >= 2 && name.chars().count() <= MAX_PARTY_CHARS).then(|| name.to_string())
}

fn parties(text: &str, lines: &[&str]) -> Option<(String, f32)> {
    let labelled: Vec<String> = lines
        .iter()
        .filter_map(|line| party_label_re().captures(line))
        .filter_map(|caps| clean_party(&caps[1]))
        .take(2)
        .collect();
    if labelled.len() == 2 {
        return Some((labelled.join("; "), 0.85));
    }
    // The first "between X and Y" in the opening of the document.
    let head: String = text.chars().take(4_000).collect();
    let caps = between_re().captures(&head)?;
    let (first, second) = (clean_party(&caps[1])?, clean_party(&caps[2])?);
    (first != second).then(|| (format!("{first}; {second}"), 0.8))
}

fn add_term(start: Date, count: u32, unit: &str) -> Option<Date> {
    let count = i64::from(count);
    let end = match unit {
        "day" => Date::from_days(start.to_days() + count),
        "week" => Date::from_days(start.to_days() + 7 * count),
        _ => {
            let months = if unit == "year" { 12 * count } else { count };
            let total = i64::from(start.month) - 1 + months;
            let year = start.year + i32::try_from(total.div_euclid(12)).ok()?;
            let month = u8::try_from(total.rem_euclid(12) + 1).ok()?;
            // 31 January plus one month is the last day of February.
            (1..=start.day).rev().find_map(|day| Date::new(year, month, day))?
        }
    };
    // A term of one year from 1 March ends on the last day of February.
    Some(Date::from_days(end.to_days() - 1))
}

pub fn extract_contract(text: &str, default_currency: Currency) -> Vec<ExtractedField> {
    let lines = split_lines(text);
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let detected = detect_currency(text);
    let currency = detected.map_or(default_currency, |(c, _)| c);
    let mut out = Vec::new();

    if let Some((names, conf)) = parties(text, &lines) {
        out.push(fld(FieldName::Parties, FieldValue::Text(names), conf));
    }

    let start = start_re()
        .captures(&flat)
        .and_then(|caps| Date::parse(&caps[1]).ok())
        .or_else(|| {
            let caps = day_of_re().captures(&flat)?;
            Date::parse(&format!("{} {} {}", &caps[1], &caps[2], &caps[3])).ok()
        });
    if let Some(date) = start {
        out.push(fld(FieldName::DocDate, FieldValue::Date(date), 0.85));
    }

    let stated_end = end_re().captures(&flat).and_then(|caps| Date::parse(&caps[1]).ok());
    let term_end = term_re().captures(&flat).and_then(|caps| {
        let count = number(&caps[1])?;
        add_term(start?, count, &caps[2].to_ascii_lowercase())
    });
    match (stated_end, term_end) {
        (Some(end), _) => out.push(fld(FieldName::EndDate, FieldValue::Date(end), 0.85)),
        // Worked out from the start and the term, so a person should glance at it.
        (None, Some(end)) => out.push(fld(FieldName::EndDate, FieldValue::Date(end), 0.75)),
        _ => {}
    }

    if let Some(caps) = notice_re().captures(&flat) {
        let (count, unit) = match (caps.get(1), caps.get(2)) {
            (Some(count), Some(unit)) => (count.as_str(), unit.as_str()),
            _ => (&caps[3], &caps[4]),
        };
        if let Some(count) = number(count) {
            let unit = unit.to_ascii_lowercase();
            let plural = if count == 1 { "" } else { "s" };
            out.push(fld(FieldName::NoticePeriod, FieldValue::Text(format!("{count} {unit}{plural}")), 0.85));
        }
    }

    let renews = if renew_no_re().is_match(&flat) { Some("no") } else if renew_yes_re().is_match(&flat) { Some("yes") } else { None };
    if let Some(answer) = renews {
        out.push(fld(FieldName::AutoRenew, FieldValue::Text(answer.into()), 0.85));
    }

    // The contract value only when it is written as money ("GH₵ 12,000.00", "$500"), not a count.
    for m in value_re().find_iter(&flat) {
        let window: String = flat[m.end()..].chars().take(60).collect();
        // Up to the end of the first number: "of $850.00 per month, deposit $1,700" -> "of $850.00".
        let Some(first_digit) = window.find(|c: char| c.is_ascii_digit()) else { continue };
        let token_end = window[first_digit..]
            .find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.'))
            .map_or(window.len(), |i| first_digit + i);
        let first = window[..token_end].trim_end_matches(['.', ',']);
        if !money_hint_re().is_match(first) { continue; }
        if let Some(money) = last_amount(first, currency).filter(|money| money.minor > 0) {
            out.push(fld(FieldName::Total, FieldValue::Money(money), 0.75));
            break;
        }
    }

    if let Some((c, conf)) = detected {
        out.push(fld(FieldName::Currency, FieldValue::Text(c.code().to_string()), conf));
    }
    out
}

/// The last day to give notice so the contract does not renew (or ends cleanly): the end date
/// minus the notice period. `None` when either is unknown or the notice cannot be read.
pub fn notice_deadline(end: Date, notice: &str) -> Option<Date> {
    let mut parts = notice.split_whitespace();
    let count = i64::from(number(parts.next()?)?);
    let unit = parts.next()?.trim_end_matches('s');
    match unit {
        "day" => Some(Date::from_days(end.to_days() - count)),
        "week" => Some(Date::from_days(end.to_days() - 7 * count)),
        "month" => {
            let total = i64::from(end.month) - 1 - count;
            let year = end.year + i32::try_from(total.div_euclid(12)).ok()?;
            let month = u8::try_from(total.rem_euclid(12) + 1).ok()?;
            (1..=end.day).rev().find_map(|day| Date::new(year, month, day))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICES: &str = "MASTER SERVICES AGREEMENT\n\
        This Agreement is made on 1 March 2026 by and between Kumasi Cloud Services Ltd, a company \
        registered in Ghana (the \"Provider\"), and Adom Bakery Limited (the \"Client\").\n\n\
        1. Term. This Agreement starts on the Effective Date and continues for a term of twelve (12) months.\n\
        It shall renew automatically for successive one-year terms unless either party gives \
        sixty (60) days' written notice.\n\
        2. Fees. The Client shall pay a total contract value of GH₵ 18,000.00 in monthly instalments.";

    fn get(fields: &[ExtractedField], name: FieldName) -> Option<&FieldValue> {
        fields.iter().find(|f| f.name == name).map(|f| &f.value)
    }
    fn date(y: i32, m: u8, d: u8) -> FieldValue { FieldValue::Date(Date::new(y, m, d).unwrap()) }

    #[test]
    fn reads_the_key_terms_of_a_services_agreement() {
        let fields = extract_contract(SERVICES, Currency::USD);
        assert_eq!(get(&fields, FieldName::Parties), Some(&FieldValue::Text("Kumasi Cloud Services Ltd; Adom Bakery Limited".into())));
        assert_eq!(get(&fields, FieldName::DocDate), Some(&date(2026, 3, 1)));
        assert_eq!(get(&fields, FieldName::EndDate), Some(&date(2027, 2, 28)), "twelve months from 1 March");
        assert_eq!(get(&fields, FieldName::NoticePeriod), Some(&FieldValue::Text("60 days".into())));
        assert_eq!(get(&fields, FieldName::AutoRenew), Some(&FieldValue::Text("yes".into())));
        assert_eq!(get(&fields, FieldName::Total), Some(&FieldValue::Money(ll_core::Money::new(1_800_000, Currency::GHS))));
    }

    #[test]
    fn labelled_parties_stated_end_and_no_renewal() {
        let text = "TENANCY AGREEMENT\nLandlord: Ama Mensah\nTenant: Kofi Boateng\n\
            Commencement date: 2026-01-01. The tenancy ends on 2026-12-31 and shall not renew automatically.\n\
            Either party may end it with one month's notice. Monthly rent of $850.00.";
        let fields = extract_contract(text, Currency::USD);
        assert_eq!(get(&fields, FieldName::Parties), Some(&FieldValue::Text("Ama Mensah; Kofi Boateng".into())));
        assert_eq!(get(&fields, FieldName::EndDate), Some(&date(2026, 12, 31)));
        assert_eq!(get(&fields, FieldName::AutoRenew), Some(&FieldValue::Text("no".into())));
        assert_eq!(get(&fields, FieldName::NoticePeriod), Some(&FieldValue::Text("1 month".into())));
        assert_eq!(get(&fields, FieldName::Total), Some(&FieldValue::Money(ll_core::Money::new(85_000, Currency::USD))));
    }

    #[test]
    fn day_of_form_and_counts_that_are_not_money() {
        let text = "CONSULTING CONTRACT\nEntered into this 5th day of June, 2026 between Ama Consulting and Volta Foods.\n\
            The fee of 3 workshops is agreed separately.";
        let fields = extract_contract(text, Currency::USD);
        assert_eq!(get(&fields, FieldName::DocDate), Some(&date(2026, 6, 5)));
        assert!(get(&fields, FieldName::Total).is_none(), "a count is not a contract value");
    }

    #[test]
    fn notice_deadline_counts_back_from_the_end() {
        let end = Date::new(2027, 2, 28).unwrap();
        assert_eq!(notice_deadline(end, "60 days"), Date::new(2026, 12, 30));
        assert_eq!(notice_deadline(end, "2 weeks"), Date::new(2027, 2, 14));
        assert_eq!(notice_deadline(Date::new(2026, 3, 31).unwrap(), "1 month"), Date::new(2026, 2, 28));
        assert_eq!(notice_deadline(end, "soon"), None);
    }
}
