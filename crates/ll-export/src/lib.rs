#![forbid(unsafe_code)]

use csv::WriterBuilder;
use ll_core::{Currency, Error, Result};
use ll_store::InvoiceRow;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTarget {
    Canonical,
    QuickBooks,
    Xero,
}

pub fn export_invoices<W: Write>(
    records: &[InvoiceRow],
    target: ExportTarget,
    output: W,
) -> Result<usize> {
    let mut writer = WriterBuilder::new().from_writer(output);
    let headers: &[&str] = match target {
        ExportTarget::Canonical => &[
            "document_id",
            "vendor",
            "invoice_date",
            "currency",
            "total_minor",
        ],
        ExportTarget::QuickBooks => &["TxnDate", "Name", "Currency", "Amount"],
        ExportTarget::Xero => &["ContactName", "InvoiceDate", "Currency", "Total"],
    };
    writer.write_record(headers).map_err(csv_error)?;

    for record in records {
        let date = record
            .invoice_date
            .map(|date| date.to_string())
            .unwrap_or_default();
        let vendor = spreadsheet_safe(record.vendor.as_deref().unwrap_or_default());
        let currency = spreadsheet_safe(record.currency.as_deref().unwrap_or_default());
        let amount = match (target, record.total_minor) {
            (_, None) => String::new(),
            (ExportTarget::Canonical, Some(minor)) => minor.to_string(),
            (_, Some(minor)) => {
                let currency = record.currency.as_deref().ok_or_else(|| {
                    Error::Invalid("accounting export requires a currency".into())
                })?;
                format_amount(minor, Currency::new(currency)?)
            }
        };
        let fields = match target {
            ExportTarget::Canonical => vec![
                record.document_id.to_string(),
                vendor.clone(),
                date,
                currency.clone(),
                amount,
            ],
            ExportTarget::QuickBooks => {
                vec![date, vendor.clone(), currency.clone(), amount]
            }
            ExportTarget::Xero => vec![vendor, date, currency, amount],
        };
        writer.write_record(fields).map_err(csv_error)?;
    }
    writer.flush().map_err(csv_error)?;
    Ok(records.len())
}

fn csv_error(error: impl std::fmt::Display) -> Error {
    Error::Integration(format!("CSV export: {error}"))
}

fn spreadsheet_safe(value: &str) -> String {
    let first_non_space = value
        .trim_start_matches(|character: char| character.is_whitespace() || character == '\u{feff}')
        .chars()
        .next();
    let has_leading_control = value
        .chars()
        .take_while(|character| character.is_whitespace() || *character == '\u{feff}')
        .any(|character| matches!(character, '\t' | '\r' | '\n' | '\u{feff}'));
    if has_leading_control || matches!(first_non_space, Some('=' | '+' | '-' | '@')) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn format_amount(minor: i64, currency: Currency) -> String {
    let exponent = currency.exponent() as u32;
    let factor = 10u64.pow(exponent);
    let absolute = minor.unsigned_abs();
    let sign = if minor < 0 { "-" } else { "" };
    if exponent == 0 {
        format!("{sign}{absolute}")
    } else {
        format!(
            "{sign}{}.{:0width$}",
            absolute / factor,
            absolute % factor,
            width = exponent as usize
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_core::{Date, DocId};

    fn record(vendor: &str, total: i64) -> InvoiceRow {
        InvoiceRow {
            document_id: DocId::new(),
            vendor: Some(vendor.into()),
            invoice_date: Some(Date::new(2025, 4, 3).unwrap()),
            currency: Some("USD".into()),
            total_minor: Some(total),
        }
    }

    #[test]
    fn canonical_export_preserves_minor_units_and_quotes_vendor_names() {
        let mut bytes = Vec::new();
        assert_eq!(
            export_invoices(
                &[record("Acme, Inc.", 12345)],
                ExportTarget::Canonical,
                &mut bytes
            )
            .unwrap(),
            1
        );
        let mut reader = csv::Reader::from_reader(bytes.as_slice());
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(&row[1], "Acme, Inc.");
        assert_eq!(&row[2], "2025-04-03");
        assert_eq!(&row[4], "12345");
    }

    #[test]
    fn accounting_targets_use_their_expected_column_names() {
        for (target, expected) in [
            (ExportTarget::QuickBooks, "TxnDate,Name,Currency,Amount"),
            (ExportTarget::Xero, "ContactName,InvoiceDate,Currency,Total"),
        ] {
            let mut bytes = Vec::new();
            export_invoices(&[], target, &mut bytes).unwrap();
            assert_eq!(std::str::from_utf8(&bytes).unwrap().trim(), expected);
        }
    }

    #[test]
    fn accounting_amounts_are_decimal_not_minor_units() {
        let mut bytes = Vec::new();
        export_invoices(
            &[record("Acme", 12345)],
            ExportTarget::QuickBooks,
            &mut bytes,
        )
        .unwrap();
        let mut reader = csv::Reader::from_reader(bytes.as_slice());
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(&row[3], "123.45");
    }

    #[test]
    fn accounting_exports_treat_formula_leading_text_as_text() {
        let mut record = record("=HYPERLINK(\"https://example.invalid\")", 100);
        record.currency = Some(" \t=cmd|' /C calc'!A0".into());
        record.total_minor = None;
        let mut bytes = Vec::new();
        export_invoices(&[record], ExportTarget::QuickBooks, &mut bytes).unwrap();
        let mut reader = csv::Reader::from_reader(bytes.as_slice());
        let row = reader.records().next().unwrap().unwrap();
        assert_eq!(&row[1], "'=HYPERLINK(\"https://example.invalid\")");
        assert_eq!(&row[2], "' \t=cmd|' /C calc'!A0");
    }
}
