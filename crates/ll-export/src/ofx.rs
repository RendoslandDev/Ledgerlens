//! OFX 1.0.2 bank statements: the file QuickBooks Online ("Upload from file") and Xero ("Import
//! a statement") read for bank transactions. One file holds one account.
//!
//! Each transaction gets a stable `FITID` from its source document and row, so importing the same
//! statement twice is recognised as a duplicate by the accounting tool rather than doubling it.

use crate::format_amount;
use ll_core::{Currency, Date, Error, Money, Result, Transaction};
use std::io::Write;

/// The longest payee name OFX 1.0.2 allows; the full text goes in `MEMO`.
const NAME_CHARS: usize = 32;
const MEMO_CHARS: usize = 255;

/// One statement's part of an account's file.
pub struct OfxStatement<'a> {
    /// Identifies the source document; with the row number it forms each `FITID`.
    pub document_key: &'a str,
    pub period_start: Option<Date>,
    pub period_end: Option<Date>,
    pub closing_balance: Option<Money>,
    pub transactions: &'a [Transaction],
}

/// Writes one account's statements as a single OFX statement. `account` is what identifies the
/// account to the importer (the last digits Ledgerlens keeps). Returns the number of transactions.
pub fn export_ofx<W: Write>(
    account: &str,
    currency: Currency,
    statements: &[OfxStatement<'_>],
    today: Date,
    mut output: W,
) -> Result<usize> {
    let mut rows: Vec<(String, &Transaction)> = statements
        .iter()
        .flat_map(|statement| {
            statement.transactions.iter().enumerate()
                .map(move |(row, t)| (format!("{}-{}", statement.document_key, row + 1), t))
        })
        .collect();
    if let Some((_, t)) = rows.iter().find(|(_, t)| t.amount.currency != currency) {
        return Err(Error::Invalid(format!("an OFX file holds one currency; found {} and {}", currency, t.amount.currency)));
    }
    rows.sort_by_key(|(_, t)| t.date); // stable: same-day rows keep statement order

    let start = statements.iter().filter_map(|s| s.period_start).chain(rows.iter().map(|(_, t)| t.date)).min();
    let end = statements.iter().filter_map(|s| s.period_end).chain(rows.iter().map(|(_, t)| t.date)).max();
    // The latest statement's closing balance is the account's balance at the end.
    let ledger = statements
        .iter()
        .filter_map(|s| Some((s.period_end?, s.closing_balance?)))
        .max_by_key(|(date, _)| *date);

    let mut body = String::new();
    body.push_str("OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\nSECURITY:NONE\r\nENCODING:USASCII\r\n\
        CHARSET:1252\r\nCOMPRESSION:NONE\r\nOLDFILEUID:NONE\r\nNEWFILEUID:NONE\r\n\r\n");
    body.push_str("<OFX>\r\n<SIGNONMSGSRSV1><SONRS><STATUS><CODE>0<SEVERITY>INFO</STATUS>");
    body.push_str(&format!("<DTSERVER>{}<LANGUAGE>ENG</SONRS></SIGNONMSGSRSV1>\r\n", ofx_date(today)));
    body.push_str("<BANKMSGSRSV1><STMTTRNRS><TRNUID>1<STATUS><CODE>0<SEVERITY>INFO</STATUS>\r\n");
    body.push_str(&format!(
        "<STMTRS><CURDEF>{}<BANKACCTFROM><BANKID>000000000<ACCTID>{}<ACCTTYPE>CHECKING</BANKACCTFROM>\r\n",
        currency.code(),
        text(account, 22)
    ));
    body.push_str("<BANKTRANLIST>");
    if let (Some(start), Some(end)) = (start, end) {
        body.push_str(&format!("<DTSTART>{}<DTEND>{}", ofx_date(start), ofx_date(end)));
    }
    body.push_str("\r\n");
    for (id, t) in &rows {
        let kind = if t.amount.minor >= 0 { "CREDIT" } else { "DEBIT" };
        body.push_str(&format!(
            "<STMTTRN><TRNTYPE>{kind}<DTPOSTED>{}<TRNAMT>{}<FITID>{}<NAME>{}<MEMO>{}</STMTTRN>\r\n",
            ofx_date(t.date),
            format_amount(t.amount.minor, currency),
            text(id, 255),
            text(&t.description, NAME_CHARS),
            text(&t.description, MEMO_CHARS),
        ));
    }
    body.push_str("</BANKTRANLIST>\r\n");
    if let Some((date, balance)) = ledger {
        body.push_str(&format!(
            "<LEDGERBAL><BALAMT>{}<DTASOF>{}</LEDGERBAL>\r\n",
            format_amount(balance.minor, currency),
            ofx_date(date)
        ));
    }
    body.push_str("</STMTRS></STMTTRNRS></BANKMSGSRSV1>\r\n</OFX>\r\n");
    output.write_all(body.as_bytes())?;
    Ok(rows.len())
}

fn ofx_date(date: Date) -> String {
    format!("{:04}{:02}{:02}", date.year, date.month, date.day)
}

/// ASCII-only, SGML-escaped, single-line text of at most `max` characters before escaping.
fn text(value: &str, max: usize) -> String {
    let ascii: String = value
        .chars()
        .map(fold_to_ascii)
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    let ascii = ascii.trim();
    let ascii = if ascii.is_empty() { "-" } else { ascii };
    ascii.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Latin letters lose their accents; anything else outside ASCII becomes `?`.
fn fold_to_ascii(c: char) -> char {
    if c.is_ascii() { return if c.is_ascii_control() { ' ' } else { c }; }
    let table: [(&str, char); 12] = [
        ("àáâãäåā", 'a'), ("ÀÁÂÃÄÅĀ", 'A'), ("èéêëē", 'e'), ("ÈÉÊËĒ", 'E'), ("ìíîïī", 'i'), ("ÌÍÎÏĪ", 'I'),
        ("òóôõöøō", 'o'), ("ÒÓÔÕÖØŌ", 'O'), ("ùúûüū", 'u'), ("ÙÚÛÜŪ", 'U'), ("çćč", 'c'), ("ñń", 'n'),
    ];
    table.iter().find(|(set, _)| set.contains(c)).map_or('?', |(_, plain)| *plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: u8, minor: i64, description: &str) -> Transaction {
        Transaction {
            date: Date::new(2026, 3, day).unwrap(),
            description: description.into(),
            amount: Money::new(minor, Currency::GHS),
            balance: None,
        }
    }

    #[test]
    fn writes_one_account_with_stable_ids_and_the_closing_balance() {
        let march = [row(5, -120_000, "Rent & service <March>"), row(2, 250_000, "Salary")];
        let statement = OfxStatement {
            document_key: "abc123",
            period_start: Date::new(2026, 3, 1),
            period_end: Date::new(2026, 3, 31),
            closing_balance: Some(Money::new(230_000, Currency::GHS)),
            transactions: &march,
        };
        let mut bytes = Vec::new();
        let count = export_ofx("7890", Currency::GHS, &[statement], Date::new(2026, 4, 2).unwrap(), &mut bytes).unwrap();
        let ofx = String::from_utf8(bytes).unwrap();
        assert_eq!(count, 2);
        assert!(ofx.starts_with("OFXHEADER:100\r\n"));
        assert!(ofx.contains("<CURDEF>GHS<BANKACCTFROM><BANKID>000000000<ACCTID>7890"));
        assert!(ofx.contains("<DTSTART>20260301<DTEND>20260331"));
        // sorted by date; ids keep the statement row number
        let salary = ofx.find("<TRNAMT>2500.00<FITID>abc123-2<NAME>Salary").unwrap();
        let rent = ofx.find("<TRNTYPE>DEBIT<DTPOSTED>20260305<TRNAMT>-1200.00<FITID>abc123-1").unwrap();
        assert!(salary < rent);
        assert!(ofx.contains("<NAME>Rent &amp; service &lt;March&gt;"));
        assert!(ofx.contains("<LEDGERBAL><BALAMT>2300.00<DTASOF>20260331</LEDGERBAL>"));
        assert!(ofx.is_ascii());
    }

    #[test]
    fn text_is_ascii_and_bounded() {
        assert_eq!(text("Café Señor ₵", 32), "Cafe Senor ?");
        assert_eq!(text("a\nb", 32), "a b");
        assert_eq!(text("x".repeat(40).as_str(), NAME_CHARS).len(), 32);
        assert_eq!(text("   ", 32), "-");
    }

    #[test]
    fn mixed_currencies_are_refused() {
        let rows = [row(1, 100, "a"), Transaction { amount: Money::new(1, Currency::USD), ..row(2, 1, "b") }];
        let statement = OfxStatement { document_key: "k", period_start: None, period_end: None, closing_balance: None, transactions: &rows };
        assert!(export_ofx("1", Currency::GHS, &[statement], Date::new(2026, 4, 2).unwrap(), Vec::new()).is_err());
    }
}
