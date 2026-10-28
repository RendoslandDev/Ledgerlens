//! CORD v2 (`naver-clova-ix/cord-v2`, CC BY 4.0) as published on Hugging Face: one parquet file
//! per split with an `image` struct (`bytes`, `path`) and a `ground_truth` JSON string.
//!
//! Only the labels Ledgerlens extracts are kept: total, subtotal, tax, and line-item prices.
//! CORD receipts are Indonesian and carry no currency label, so amounts are compared as numbers
//! (in hundredths of the receipt's currency unit), never by currency.

use ll_core::{money::parse_amount, Currency, Error, Result};
use parquet::{
    file::reader::{FileReader, SerializedFileReader},
    record::{Field, Row},
};
use serde_json::Value;
use std::{fs::File, path::Path};

/// One labelled receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct CordReceipt {
    /// Split and position (`test-000`, `validation-042`), stable across runs of the same file.
    pub id: String,
    pub image: Vec<u8>,
    pub truth: CordTruth,
}

/// Labelled amounts, in hundredths of the currency unit ("60.000" -> 6_000_000).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CordTruth {
    pub total: Option<i64>,
    pub subtotal: Option<i64>,
    pub tax: Option<i64>,
    /// Line prices of the top-level menu entries (sub-items are part of their parent's price).
    pub items: Vec<i64>,
}

/// Reads every receipt in a CORD parquet file, calling `visit` for each in file order. Receipts
/// are streamed one row at a time, so memory stays near one image.
pub fn read_cord_parquet(path: &Path, mut visit: impl FnMut(CordReceipt) -> Result<()>) -> Result<usize> {
    let file = File::open(path)?;
    let reader = SerializedFileReader::new(file).map_err(|e| Error::Invalid(format!("not a readable parquet file: {e}")))?;
    let rows = reader.get_row_iter(None).map_err(|e| Error::Invalid(format!("parquet rows: {e}")))?;
    let split = split_name(path);
    let mut count = 0;
    for (index, row) in rows.enumerate() {
        let row = row.map_err(|e| Error::Invalid(format!("parquet row {index}: {e}")))?;
        let (image, truth) = receipt_columns(&row)
            .ok_or_else(|| Error::Invalid(format!("row {index} has no image bytes or ground truth")))?;
        visit(CordReceipt { id: format!("{split}-{index:03}"), image, truth: parse_truth(&truth)? })?;
        count += 1;
    }
    Ok(count)
}

/// The split a Hugging Face parquet file holds, from its name (`validation-00000-of-00001.parquet`).
pub fn split_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.split('-').next())
        .filter(|split| !split.is_empty())
        .unwrap_or("cord")
        .to_string()
}

fn receipt_columns(row: &Row) -> Option<(Vec<u8>, String)> {
    let mut image = None;
    let mut truth = None;
    for (name, field) in row.get_column_iter() {
        match (name.as_str(), field) {
            ("image", Field::Group(group)) => {
                image = group.get_column_iter().find_map(|(name, field)| match (name.as_str(), field) {
                    ("bytes", Field::Bytes(bytes)) => Some(bytes.data().to_vec()),
                    _ => None,
                });
            }
            ("ground_truth", Field::Str(text)) => truth = Some(text.clone()),
            _ => {}
        }
    }
    Some((image?, truth?))
}

/// Parses CORD's `ground_truth` JSON (`gt_parse.total.total_price`, `gt_parse.sub_total.*`,
/// `gt_parse.menu[].price`). Missing or unreadable labels are left out rather than guessed.
pub fn parse_truth(json: &str) -> Result<CordTruth> {
    let value: Value = serde_json::from_str(json).map_err(|e| Error::Invalid(format!("ground truth: {e}")))?;
    let parse = &value["gt_parse"];
    let menu = match &parse["menu"] {
        Value::Array(items) => items.iter().collect(),
        item @ Value::Object(_) => vec![item],
        _ => Vec::new(),
    };
    Ok(CordTruth {
        total: amount(&parse["total"]["total_price"]),
        subtotal: amount(&parse["sub_total"]["subtotal_price"]),
        tax: amount(&parse["sub_total"]["tax_price"]),
        items: menu.into_iter().filter_map(|item| amount(&item["price"])).collect(),
    })
}

/// A labelled amount in hundredths. A few labels are lists (the value printed twice); the first
/// readable one is used.
fn amount(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => hundredths(text),
        Value::Array(values) => values.iter().find_map(amount),
        _ => None,
    }
}

/// Reads an amount with the same rules Ledgerlens uses ("60.000" and "60,000" are both sixty
/// thousand), expressed in hundredths so currencies with and without decimals compare.
pub fn hundredths(text: &str) -> Option<i64> {
    parse_amount(text, Currency::USD).ok().map(|money| money.minor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_truth_amounts_are_read_like_receipts_print_them() {
        let truth = parse_truth(
            r#"{"gt_parse":{"menu":[{"nm":"NASI GORENG","cnt":"1","price":"25.000"},
                {"nm":"ES TEH","cnt":"2","price":"10,000","sub":[{"nm":"LESS SUGAR"}]}],
              "sub_total":{"subtotal_price":"35.000","tax_price":"3.500"},
              "total":{"total_price":"38.500","cashprice":"50.000"}}}"#,
        )
        .unwrap();
        assert_eq!(truth.total, Some(3_850_000));
        assert_eq!(truth.subtotal, Some(3_500_000));
        assert_eq!(truth.tax, Some(350_000));
        assert_eq!(truth.items, [2_500_000, 1_000_000]);
    }

    /// Writes a two-receipt file with the Hugging Face CORD schema and reads it back.
    #[test]
    fn parquet_rows_become_receipts() {
        use parquet::{
            data_type::{ByteArray, ByteArrayType},
            file::{properties::WriterProperties, writer::SerializedFileWriter},
            schema::parser::parse_message_type,
        };
        use std::sync::Arc;

        let schema = parse_message_type(
            "message schema {
                optional group image { optional binary bytes; optional binary path (UTF8); }
                optional binary ground_truth (UTF8);
            }",
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!("test-ll-eval-cord-{}.parquet", std::process::id()));
        let file = File::create(&path).unwrap();
        let mut writer = SerializedFileWriter::new(file, Arc::new(schema), Arc::new(WriterProperties::builder().build())).unwrap();
        let mut group = writer.next_row_group().unwrap();
        let columns: [(&[&[u8]], i16); 3] = [
            (&[b"\xFF\xD8\xFFjpeg-1", b"\x89PNGpng-2"], 2),
            (&[b"0.jpg", b"1.png"], 2),
            (&[
                br#"{"gt_parse":{"total":{"total_price":"12.000"}}}"#,
                br#"{"gt_parse":{"menu":[{"price":"5,000"}],"total":{"total_price":"5,000"}}}"#,
            ], 1),
        ];
        for (values, level) in columns {
            let mut column = group.next_column().unwrap().unwrap();
            let values: Vec<ByteArray> = values.iter().map(|value| ByteArray::from(value.to_vec())).collect();
            column.typed::<ByteArrayType>().write_batch(&values, Some(&[level, level]), None).unwrap();
            column.close().unwrap();
        }
        group.close().unwrap();
        writer.close().unwrap();

        let mut receipts = Vec::new();
        let count = read_cord_parquet(&path, |receipt| {
            receipts.push(receipt);
            Ok(())
        })
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(count, 2);
        assert_eq!(receipts[0].id, "test-000");
        assert_eq!(receipts[0].image, b"\xFF\xD8\xFFjpeg-1");
        assert_eq!(receipts[0].truth.total, Some(1_200_000));
        assert_eq!(receipts[1].truth.items, [500_000]);
        assert!(read_cord_parquet(Path::new("Cargo.toml"), |_| Ok(())).is_err());
        assert_eq!(split_name(Path::new("/d/validation-00000-of-00001.parquet")), "validation");
    }

    #[test]
    fn missing_single_and_listed_labels() {
        let truth = parse_truth(
            r#"{"gt_parse":{"menu":{"nm":"KOPI","price":"18.000"},"total":{"total_price":["27.000","27.000"]}}}"#,
        )
        .unwrap();
        assert_eq!((truth.total, truth.subtotal, truth.tax), (Some(2_700_000), None, None));
        assert_eq!(truth.items, [1_800_000]);
        assert!(parse_truth("not json").is_err());
    }
}
