use crate::currency::Currency;
use serde::{Deserialize, Serialize};

/// Integer minor units + currency. Never floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("no digits in input")]
    NoDigits,
    #[error("more fractional digits than the currency allows")]
    TooPrecise,
    #[error("amount overflows")]
    Overflow,
    #[error("currency mismatch")]
    CurrencyMismatch,
}

/// How a number is written: `En` = 1,234.50   `Eu` = 1.234,50
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberStyle { En, Eu }

impl Money {
    pub const fn new(minor: i64, currency: Currency) -> Self { Self { minor, currency } }

    pub fn parse(input: &str, currency: Currency) -> Result<Money, MoneyError> { parse_amount(input, currency) }

    pub fn to_decimal_string(self) -> String { self.format(NumberStyle::En) }

    pub fn checked_add(self, o: Money) -> Result<Money, MoneyError> {
        if self.currency != o.currency {
             return Err(MoneyError::CurrencyMismatch);
             }
        self.minor.checked_add(o.minor).map(|m| Money::new(m, self.currency)).ok_or(MoneyError::Overflow)
    }

    pub fn checked_sub(self, o: Money) -> Result<Money, MoneyError> {
        if self.currency != o.currency { 
            return Err(MoneyError::CurrencyMismatch);
         }
        self.minor.checked_sub(o.minor).map(|m| Money::new(m, self.currency)).ok_or(MoneyError::Overflow)
    }

    /// True when |a - b| <= tolerance_minor. Used by the "sums match" validator.
    pub fn approx_eq(self, o: Money, tolerance_minor: i64) -> bool {
        self.currency == o.currency && tolerance_minor >= 0 && self.minor.abs_diff(o.minor) <= tolerance_minor as u64
    }

    /// Sum a slice; empty slice yields zero in `currency`.
    pub fn sum(items: &[Money], currency: Currency) -> Result<Money, MoneyError> {
        items.iter().try_fold(Money::new(0, currency), |acc, m| acc.checked_add(*m))
    }

    /// Plain decimal formatting without a currency symbol, e.g. "1,234.50".
    pub fn format(self, style: NumberStyle) -> String {
        let (group, dec) = match style { NumberStyle::En => (',', '.'), NumberStyle::Eu => ('.', ',') };
        let exp = self.currency.exponent() as u32;
        let abs = self.minor.unsigned_abs();
        let div = 10u64.pow(exp);
        let (whole, frac) = (abs / div, abs % div);
        let ws = whole.to_string();
        let mut grouped = String::new();
        for (i, ch) in ws.chars().enumerate() {
            if i > 0 && (ws.len() - i) % 3 == 0 { grouped.push(group); }
            grouped.push(ch);
        }
        let sign = if self.minor < 0 { "-" } else { "" };
        if exp == 0 {
             format!("{sign}{grouped}")
             } else {
                 format!("{sign}{grouped}{dec}{frac:0width$}", width = exp as usize) 
                }
    }
}

/// Parse an amount written in either common locale style.
///
/// Handles "1,234.50", "1.234,50", "1 234,50", "(12.00)" (negative), "-5", "¥1,200".
/// Genuinely ambiguous input such as "12,500" on a 2-decimal currency is read as a
/// grouped integer (12500.00); callers with document-level locale context should
/// prefer that context and flag ambiguity as a validation issue.
pub fn parse_amount(input: &str, currency: Currency) -> Result<Money, MoneyError> {
    let s = input.trim();
    let negative = s.starts_with('-') || s.starts_with('−') || (s.starts_with('(') && s.ends_with(')'));
    let body: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    if !body.chars().any(|c| c.is_ascii_digit()) { return Err(MoneyError::NoDigits); }

    let exp = currency.exponent() as u32;
    let digits = |t: &str| t.chars().filter(char::is_ascii_digit).collect::<String>();

    let (int_digits, frac_digits) = match body.rfind(['.', ',']) {
        None => (digits(&body), String::new()),
        Some(i) => {
            let sep = body.as_bytes()[i] as char;
            let (head, after) = (&body[..i], &body[i + 1..]);
            let other = if sep == '.' { ',' } else { '.' };
            let is_decimal = if head.contains(other) { true }
                else if head.contains(sep) { false }
                else { !(after.len() == 3 && exp != 3) };
            if is_decimal { (digits(head), digits(after)) } else { (digits(&body), String::new()) }
        }
    };

    if frac_digits.len() as u32 > exp { return Err(MoneyError::TooPrecise); }
    let frac = format!("{:0<width$}", frac_digits, width = exp as usize);

    let whole: i64 = if int_digits.is_empty() { 0 } else { int_digits.parse().map_err(|_| MoneyError::Overflow)? };
    let frac_val: i64 = if frac.is_empty() { 0 } else { frac.parse().unwrap_or(0) };

    let minor = whole.checked_mul(10_i64.pow(exp)).and_then(|m| m.checked_add(frac_val)).ok_or(MoneyError::Overflow)?;
    Ok(Money::new(if negative { -minor } else { minor }, currency))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use Currency::*;

    fn p(s: &str, c: Currency) -> i64 { parse_amount(s, c).unwrap().minor }

    #[test]
    fn locale_table() {
        assert_eq!(p("1,234.50", USD), 123_450);
        assert_eq!(p("1.234,50", EUR), 123_450);
        assert_eq!(p("1 234,50", EUR), 123_450);
        assert_eq!(p("$12.5", USD), 1_250);
        assert_eq!(p(".99", USD), 99);
        assert_eq!(p("0.99", USD), 99);
        assert_eq!(p("1,234,567", USD), 123_456_700);
        assert_eq!(p("(12.00)", USD), -1_200);
        assert_eq!(p("-5", USD), -500);
        assert_eq!(p("GH₵ 45.00", GHS), 4_500);
    }

    #[test]
    fn rupiah_grouping_reads_as_whole_rupiah() {
        assert_eq!(p("60.000", IDR), 6_000_000);
        assert_eq!(p("Rp 1.234.500", IDR), 123_450_000);
        assert_eq!(p("60,000", IDR), 6_000_000);
    }

    #[test]
    fn zero_decimal_currency() {
        assert_eq!(p("¥1,200", JPY), 1_200);
        assert_eq!(p("1200", JPY), 1_200);
    }

    #[test]
    fn ambiguous_grouping_is_documented_behaviour() {
        assert_eq!(p("12,500", USD), 1_250_000);
    }

    #[test]
    fn errors() {
        assert_eq!(parse_amount("abc", USD), Err(MoneyError::NoDigits));
        assert_eq!(parse_amount("12.3456", USD), Err(MoneyError::TooPrecise));
        assert_eq!(parse_amount("99999999999999999999", USD), Err(MoneyError::Overflow));
    }

    #[test]
    fn arithmetic() {
        let a = Money::new(100, USD);
        assert_eq!(a.checked_add(Money::new(50, USD)).unwrap().minor, 150);
        assert_eq!(a.checked_add(Money::new(1, EUR)), Err(MoneyError::CurrencyMismatch));
        assert_eq!(Money::new(i64::MAX, USD).checked_add(a), Err(MoneyError::Overflow));
        assert!(a.approx_eq(Money::new(101, USD), 1));
        assert!(!a.approx_eq(Money::new(103, USD), 1));
        assert!(Money::new(i64::MIN, USD).approx_eq(Money::new(i64::MIN + 1, USD), 1));
        assert_eq!(Money::sum(&[a, a, a], USD).unwrap().minor, 300);
    }

    #[test]
    fn formatting() {
        assert_eq!(Money::new(123_450, USD).format(NumberStyle::En), "1,234.50");
        assert_eq!(Money::new(123_450, EUR).format(NumberStyle::Eu), "1.234,50");
        assert_eq!(Money::new(-5, USD).format(NumberStyle::En), "-0.05");
        assert_eq!(Money::new(1_200, JPY).format(NumberStyle::En), "1,200");
    }

    proptest! {
        #[test]
        fn format_parse_roundtrip(minor in -10_000_000_000i64..10_000_000_000i64, idx in 0usize..Currency::ALL.len(), eu in any::<bool>()) {
            let cur = Currency::ALL[idx];
            let style = if eu { NumberStyle::Eu } else { NumberStyle::En };
            let m = Money::new(minor, cur);
            let s = m.format(style);
            // Zero-decimal currencies can be ambiguous in EU style ("1.200"); only assert for 2-decimal ones
            // plus zero-decimal in En style, where the grouping rule reads back correctly.
            if cur.exponent() > 0 || (cur.exponent() == 0 && !eu) {
                prop_assert_eq!(parse_amount(&s, cur).unwrap(), m);
            }
        }
    }
}
