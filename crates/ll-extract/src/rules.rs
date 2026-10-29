//! Rules tier: regex and layout heuristics over OCR text.
//!
//! Input is the text produced by `ll-ocr` (`\n` between lines, `\t` at column gaps, `\u{c}`
//! between pages). Every extractor here is deliberately conservative: it returns nothing rather
//! than guessing wildly, and reports a prior confidence that calibration can later correct.

use ll_core::{Confidence, Currency, Date, DocKind, ExtractedField, FieldName, FieldValue, LineItem, Money, Tier};
use regex::Regex;
use std::sync::OnceLock;

macro_rules! lazy_re {
    ($vis:vis $name:ident, $pat:expr) => {
        $vis fn $name() -> &'static Regex {
            static C: OnceLock<Regex> = OnceLock::new();
            C.get_or_init(|| {
                let p: String = $pat.to_string();
                Regex::new(&p).unwrap_or_else(|e| panic!("bad regex {p}: {e}"))
            })
        }
    };
}

pub(crate) const DATE_PAT: &str = r"\d{4}[-/.]\d{1,2}[-/.]\d{1,2}|\d{1,2}[-/.]\d{1,2}[-/.]\d{2,4}|[A-Za-z]{3,9}\.?\s+\d{1,2}(?:st|nd|rd|th)?,?\s+\d{4}|\d{1,2}(?:st|nd|rd|th)?\s+[A-Za-z]{3,9}\.?,?\s+\d{4}";

lazy_re!(pub(crate) date_re, DATE_PAT);
lazy_re!(inv_no_re, r"(?i)\b(?:invoice|inv)\b\s*(?:(?:#|no\b\.?|number\b|num\b\.?)\s*[:#.]?|:)\s*([A-Z0-9][A-Z0-9\-/]*)");
lazy_re!(pub(crate) inv_date_re, format!(r"(?i)\b(invoice\s+date|date\s+of\s+issue|issue\s+date|issued(?:\s+on)?|date)\b\s*[:\-]?\s*({DATE_PAT})"));
lazy_re!(due_re, format!(r"(?i)\b(due\s+date|date\s+due|payment\s+due(?:\s+date)?|pay\s+by|due\s+on|due)\b\s*[:\-]?\s*({DATE_PAT})"));
lazy_re!(amount_re, r"(?:US\$|GH₵|[$€£₵])?\s?\(?-?\d(?:[\d.,]*\d)?\)?");
lazy_re!(subtotal_re, r"(?i)\b(?:sub[\s\-]?total|sub\s?tt[li]|net\s+(?:amount|amt|total|sales)|total\s+(?:before\s+(?:tax|vat)|excl(?:uding|\.)?\s*(?:tax|vat)))\b");
lazy_re!(total_strong_re, r"(?i)\b(?:grand\s+total|total\s+due|amount\s+due|balance\s+due|total\s+amount|amount\s+payable|total\s+payable)\b");
lazy_re!(total_tax_re, r"(?i)\btotal\s+(?:sales\s+)?(?:tax|vat|gst)\b");
lazy_re!(total_weak_re, r"(?i)\btotal\b");
// Indonesian receipts: PPN (VAT), "Pajak" (tax), and the restaurant tax PB1, printed as "PB1",
// "Pajak Resto", "P.Rest", or "Resto 10%" (OCR often reads PB1 as "PBI" or "PB 1").
lazy_re!(tax_re, r"(?i)\b(?:sales\s+tax|tax|vat|gst|hst|ppn|pajak|paiak|pb\s?[1il]|p\.\s?rest|resto)\b");
// "Total qty 2", "Total items: 3", "ITEMS :1", "4.00xITEMs": a count of things, not money. OCR
// garbles "qty" ("Rty", "oty", "otý") and "item" ("iten"), so those spellings count too.
lazy_re!(item_count_re, r"(?i)(?:\b|\d|x)(?:q'?ty|quantity|items?|iten|pcs|[ro]t[yý])\b");
lazy_re!(bracketed_re, r"\([^)]*\)");
// Payment and change rows sit next to the total and carry other amounts.
lazy_re!(payment_re, r"(?i)\b(?:cash|change|kembali(?:an)?|tunai|pay|bayar|card|debit|credit|edc)\b");
// Two OCR-split groups of one number: "16, 500", "121 798", "41 ,000", "63, 000.00".
lazy_re!(split_number_re, r"(?:^|[^\d.,])(\d{1,3}(?:[.,]\d{3})*)\s?([.,])?\s([.,])?(\d{3}(?:[.,]\d{1,2})?)(?:$|[^\d.,])");
lazy_re!(tax_id_re, r"(?i)\b(?:tax|vat|gst)\s*(?:id|no\b|number|reg\w*|#)");
lazy_re!(vendor_label_re, r"(?i)^\s*(?:vendor|supplier|seller|from|bill(?:ed)?\s+from|remit\s+to)\s*[:\-]\s*(.+?)\s*$");
lazy_re!(title_re, r"(?i)^(?:tax\s+)?(?:invoice|receipt|statement|bill|credit\s+note|proforma|quotation|quote|page|date|due|total|subtotal|tel|phone|email|e-mail|fax|www|http|ship\s+to|sold\s+to|customer|order|po)\b");
lazy_re!(suffix_re, r"(?i)\b(?:inc|llc|ltd|limited|corp|corporation|gmbh|plc|co|company|enterprises?|traders?|trading|services|solutions|group|holdings|ventures|industries|sa|ag|bv|pty|lda|s\.?r\.?l|s\.?a\.?r\.?l|s\.?p\.?a|s\.?a\.?s|n\.?v|oy|ab|pte|pvt|kk)\b\.?$");
lazy_re!(qty_re, r"(?i)^(\d+(?:\.\d+)?)\s*(?:x|pcs|hrs?|units?)?$");
// "SERVICE AGREEMENT", "Lease Agreement", "Employment Contract": a title line naming the document.
lazy_re!(contract_title_re, r"^(?:[a-z&,\-]+\s+){0,4}(?:agreement|contract)(?:\s+(?:for|of)\s+[a-z ]{1,40})?$");
// Body phrases a contract uses and an invoice or statement does not.
lazy_re!(contract_body_re, r"\b(?:this\s+(?:agreement|contract)\s+is\s+(?:made|entered)|(?:by\s+and\s+)?between\b.{1,200}\band\b.{1,200}\b(?:hereinafter|collectively|the\s+parties)|in\s+witness\s+whereof|the\s+parties\s+agree)");
lazy_re!(kind_title_re, r"^(?:tax\s+|sales\s+|payment\s+|official\s+|cash\s+|commercial\s+|account\s+)?(invoice|receipt|statement)(?:\s+of\s+account)?$");
// A leading quantity on a receipt line: "2 NASI GORENG", "1X M-Caramel Tea", "3 x Bolts".
lazy_re!(lead_qty_re, r"(?i)^(\d{1,3})\s*[x×]?\s+(.*\p{L}.*)$");
// Header and footer rows that carry numbers but are not purchases.
lazy_re!(non_item_re, r"(?i)\b(?:tel|telp|phone|fax|npwp|cashier|kasir|table|meja|no\.?|nomor|order|bill|struk|receipt|trx|ref|server|waiter|operator|pos|terminal|shift|pax|guest|date|tanggal|time|jam|disc(?:ount)?|diskon|promo\s+code|member|point|card|cash|change|kembali|tunai|qty|items?|rounding|pembulatan|service|charge)\b");
lazy_re!(code_re, r"\b(USD|EUR|GBP|GHS|NGN|KES|ZAR|CAD|AUD|JPY|CHF|INR|CNY|IDR)\b");
// "Rp" before an amount ("Rp 25.000", "Rp.25,000"); the letters alone are too common to count.
lazy_re!(rupiah_re, r"\bRp\.?\s?\d");

pub(crate) fn fld(name: FieldName, value: FieldValue, conf: f32) -> ExtractedField {
    ExtractedField { name, value, confidence: Confidence::new(conf), tier: Tier::Rules }
}

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    text.split(['\n', '\u{c}']).map(|l| l.trim_end_matches('\r')).collect()
}

pub(crate) fn cells(line: &str) -> Vec<&str> {
    line.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect()
}

pub(crate) fn first_cell(line: &str) -> &str {
    line.split('\t').map(str::trim).find(|c| !c.is_empty()).unwrap_or("")
}

// ---- document kind ---------------------------------------------------------------------------

/// Classifies from the heading (first lines) first, then falls back to anywhere in the text.
pub fn classify_kind(text: &str) -> DocKind {
    let lower = text.to_lowercase();
    let head_lines: Vec<&str> = split_lines(&lower).into_iter().map(str::trim).filter(|l| !l.is_empty()).take(6).collect();
    // A line that is only a title ("RECEIPT", "TAX INVOICE") outranks a label such as "Invoice No:".
    for line in &head_lines {
        if contract_title_re().is_match(line) { return DocKind::Contract; }
        if let Some(caps) = kind_title_re().captures(line) {
            return match &caps[1] {
                "invoice" => DocKind::Invoice,
                "receipt" => DocKind::Receipt,
                _ => DocKind::Statement,
            };
        }
    }
    // "Payment terms per agreement" on an invoice is not a contract; contract wording is.
    if contract_body_re().is_match(&lower.replace('\n', " ")) && !head_lines.iter().any(|l| l.contains("invoice") || l.contains("receipt")) {
        return DocKind::Contract;
    }
    let head = head_lines.join("\n");
    for hay in [head.as_str(), lower.as_str()] {
        if hay.contains("invoice") { return DocKind::Invoice; }
        if hay.contains("receipt") { return DocKind::Receipt; }
        if hay.contains("statement") { return DocKind::Statement; }
    }
    DocKind::Other
}

// ---- currency --------------------------------------------------------------------------------

/// Explicit ISO code (most frequent wins) beats a symbol; `None` when there is no evidence.
pub fn detect_currency(text: &str) -> Option<(Currency, f32)> {
    let mut tally: Vec<(&str, usize)> = Vec::new();
    for m in code_re().find_iter(text) {
        match tally.iter_mut().find(|(c, _)| *c == m.as_str()) {
            Some((_, n)) => *n += 1,
            None => tally.push((m.as_str(), 1)),
        }
    }
    if let Some((code, _)) = tally.iter().copied().reduce(|best, cur| if cur.1 > best.1 { cur } else { best }) {
        return Currency::new(code).ok().map(|c| (c, 0.95));
    }
    let symbols = [("$", Currency::USD), ("€", Currency::EUR), ("£", Currency::GBP), ("₵", Currency::GHS)];
    let mut best: Option<(Currency, usize)> = None;
    let rupiah = rupiah_re().find_iter(text).count();
    if rupiah > 0 { best = Some((Currency::IDR, rupiah)); }
    for (sym, cur) in symbols {
        let n = text.matches(sym).count();
        if n > 0 && best.is_none_or(|(_, bn)| n > bn) { best = Some((cur, n)); }
    }
    best.map(|(c, _)| (c, 0.85))
}

// ---- identifiers and dates -------------------------------------------------------------------

fn invoice_number(lines: &[&str]) -> Option<String> {
    for line in lines {
        for caps in inv_no_re().captures_iter(line) {
            let v = caps[1].trim_end_matches(['-', '/']);
            if v.chars().any(|c| c.is_ascii_digit()) { return Some(v.to_string()); }
        }
    }
    None
}

/// `05/04/2024` could be either 5 April or 4 May; `Date::parse` picks US order, so say so.
pub(crate) fn ambiguous(raw: &str) -> bool {
    let parts: Vec<&str> = raw.split(['-', '/', '.']).collect();
    if parts.len() != 3 || parts[0].len() >= 4 { return false; }
    match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
        (Ok(a), Ok(b)) => a <= 12 && b <= 12 && a != b,
        _ => false,
    }
}

/// Words that, right before a bare "Date", mean it is some other date (payment date, ship date...).
fn other_date_prefix(prefix: &str) -> bool {
    let p = prefix.trim().to_lowercase();
    ["due", "payment", "ship", "shipping", "delivery", "order", "start", "end", "period"].iter().any(|w| p.ends_with(w))
}

pub(crate) fn find_date(lines: &[&str], re: &Regex, invoice_date: bool) -> Option<(Date, f32)> {
    for line in lines {
        for caps in re.captures_iter(line) {
            let label = caps[1].to_ascii_lowercase();
            let start = caps.get(0).map_or(0, |m| m.start());
            if invoice_date && label == "date" && other_date_prefix(&line[..start]) { continue; }
            let raw = &caps[2];
            let Ok(date) = Date::parse(raw) else { continue };
            let mut conf = match (invoice_date, label.as_str()) {
                (true, "date") => 0.85,
                (true, _) => 0.93,
                (false, _) => 0.9,
            };
            if ambiguous(raw) { conf -= 0.1; }
            return Some((date, conf));
        }
    }
    None
}

// ---- amounts ---------------------------------------------------------------------------------

/// Rejoins thousands groups that OCR split with a space ("16, 500" -> "16,500").
fn join_split_numbers(text: &str) -> String {
    let mut out = text.to_string();
    // Each pass joins one split; a number split twice ("1 234 567") needs two.
    for _ in 0..3 {
        let next = split_number_re()
            .replace(&out, |caps: &regex::Captures| {
                let whole = &caps[0];
                let lead = whole.chars().next().filter(|c| !c.is_ascii_digit()).map(String::from).unwrap_or_default();
                let tail = whole.chars().last().filter(|c| !c.is_ascii_digit()).map(String::from).unwrap_or_default();
                let sep = caps.get(2).or(caps.get(3)).map_or(",", |m| m.as_str());
                format!("{lead}{}{sep}{}{tail}", &caps[1], &caps[4])
            })
            .into_owned();
        if next == out {
            break;
        }
        out = next;
    }
    out
}

/// The right-most amount in `rest`, ignoring dates and percentages ("VAT (15%) $18.75" -> 18.75).
pub(crate) fn last_amount(rest: &str, cur: Currency) -> Option<Money> {
    let joined = join_split_numbers(rest);
    let cleaned = date_re().replace_all(&joined, " ");
    let mut best = None;
    for m in amount_re().find_iter(&cleaned) {
        if cleaned[m.end()..].trim_start().starts_with('%') { continue; }
        if let Ok(v) = Money::parse(m.as_str(), cur) { best = Some(v); }
    }
    best
}

enum Summary { Subtotal, TotalTax, Tax, Total { strong: bool } }

/// Recognises a subtotal/tax/total row and returns the byte offset where its label ends. The label
/// must sit at the start of the row and the row must be narrow, so an item row such as
/// "Tax consulting  1  $500  $500" is not mistaken for a summary row.
fn classify_summary(line: &str) -> Option<(Summary, usize)> {
    if cells(line).len() > 3 { return None; }
    // A count in brackets annotates a real total ("TOTAL (2 items) 120,000"); a bare count line
    // ("TOTAL QTY 2") is not money.
    let label = bracketed_re().replace_all(first_cell(line), " ");
    if item_count_re().is_match(&label) && !subtotal_re().is_match(line) { return None; }
    let near_start = |start: usize| line[..start].split_whitespace().count() <= 2;
    if let Some(m) = subtotal_re().find(line) {
        return near_start(m.start()).then_some((Summary::Subtotal, m.end()));
    }
    if tax_id_re().is_match(line) { return None; }
    if let Some(m) = total_strong_re().find(line) {
        return near_start(m.start()).then_some((Summary::Total { strong: true }, m.end()));
    }
    if let Some(m) = total_tax_re().find(line) {
        return near_start(m.start()).then_some((Summary::TotalTax, m.end()));
    }
    if let Some(m) = total_weak_re().find(line) {
        return near_start(m.start()).then_some((Summary::Total { strong: false }, m.end()));
    }
    if let Some(m) = tax_re().find(line) {
        return near_start(m.start()).then_some((Summary::Tax, m.end()));
    }
    None
}

// ---- vendor ----------------------------------------------------------------------------------

fn plausible_vendor(cell: &str) -> bool {
    let n = cell.chars().count();
    if !(2..=60).contains(&n) || title_re().is_match(cell) { return false; }
    let letters = cell.chars().filter(|c| c.is_alphabetic()).count();
    let digits = cell.chars().filter(|c| c.is_ascii_digit()).count();
    letters >= 2 && digits <= letters && !date_re().is_match(cell)
}

pub(crate) fn vendor(lines: &[&str]) -> Option<(String, f32)> {
    for l in lines {
        if let Some(c) = vendor_label_re().captures(l) {
            let v = first_cell(&c[1]);
            if !v.is_empty() { return Some((v.to_string(), 0.9)); }
        }
    }
    let mut seen = 0;
    for l in lines {
        let cell = first_cell(l);
        if cell.is_empty() { continue; }
        seen += 1;
        if seen > 8 { break; }
        if !plausible_vendor(cell) { continue; }
        let conf = match (suffix_re().is_match(cell), seen == 1) {
            (true, true) => 0.9,
            (true, false) => 0.85,
            (false, true) => 0.7,
            (false, false) => 0.6,
        };
        return Some((cell.to_string(), conf));
    }
    None
}

// ---- public entry points ---------------------------------------------------------------------

/// Extracts every field the rules can find. `default_currency` is used to read amounts when the
/// text carries no currency evidence (no `Currency` field is emitted in that case).
pub fn extract_fields(text: &str, default_currency: Currency) -> Vec<ExtractedField> {
    let lines = split_lines(text);
    let detected = detect_currency(text);
    let cur = detected.map_or(default_currency, |(c, _)| c);
    let mut out = Vec::new();

    if let Some((v, c)) = vendor(&lines) { out.push(fld(FieldName::Vendor, FieldValue::Text(v), c)); }
    if let Some(n) = invoice_number(&lines) { out.push(fld(FieldName::InvoiceNumber, FieldValue::Text(n), 0.9)); }
    if let Some((d, c)) = find_date(&lines, inv_date_re(), true) { out.push(fld(FieldName::InvoiceDate, FieldValue::Date(d), c)); }
    if let Some((d, c)) = find_date(&lines, due_re(), false) { out.push(fld(FieldName::DueDate, FieldValue::Date(d), c)); }

    let mut subtotal: Option<Money> = None;
    let mut tax_total: Option<Money> = None;
    let mut tax_parts: Vec<Money> = Vec::new();
    let mut totals: Vec<(f32, Money)> = Vec::new();
    for line in &lines {
        let Some((kind, end)) = classify_summary(line) else { continue };
        // "Total ⇥ Change 8,000" or "Total. Pay Cash 50.000 ⇥ 41,000": amounts in a cell that
        // names a payment belong to the payment, not to the summary row.
        let rest: Vec<&str> = line[end..].split('\t').filter(|cell| !payment_re().is_match(cell)).collect();
        let Some(m) = last_amount(&rest.join("\t"), cur) else { continue };
        match kind {
            Summary::Subtotal => { subtotal.get_or_insert(m); }
            Summary::TotalTax => { tax_total.get_or_insert(m); }
            Summary::Tax => tax_parts.push(m),
            Summary::Total { strong } => totals.push((if strong { 0.95 } else { 0.85 }, m)),
        }
    }

    if let Some(m) = subtotal { out.push(fld(FieldName::Subtotal, FieldValue::Money(m), 0.9)); }
    if let Some(m) = tax_total {
        out.push(fld(FieldName::Tax, FieldValue::Money(m), 0.9));
    } else if tax_parts.len() == 1 {
        out.push(fld(FieldName::Tax, FieldValue::Money(tax_parts[0]), 0.85));
    } else if tax_parts.len() > 1 {
        // several levies (VAT + others) are summed; a mixed-currency set is not guessable
        let sum = tax_parts[1..].iter().try_fold(tax_parts[0], |acc, m| acc.checked_add(*m));
        if let Ok(m) = sum { out.push(fld(FieldName::Tax, FieldValue::Money(m), 0.75)); }
    }
    // strongest label first, then the larger amount ("Total VAT" is smaller than the real total)
    totals.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.minor.cmp(&a.1.minor)));
    if let Some((c, m)) = totals.first() { out.push(fld(FieldName::Total, FieldValue::Money(*m), *c)); }

    if let Some((c, conf)) = detected {
        out.push(fld(FieldName::Currency, FieldValue::Text(c.code().to_string()), conf));
    }
    out
}

fn parse_qty(s: &str) -> Option<f64> {
    qty_re().captures(s.trim()).and_then(|c| c[1].parse().ok())
}

fn parse_item(cells: &[&str], cur: Currency) -> Option<LineItem> {
    let desc = cells[0];
    if !desc.chars().any(|c| c.is_alphabetic()) { return None; }
    let amount_cell = *cells.last()?;
    if !amount_cell.chars().any(|c| c.is_ascii_digit()) { return None; }
    let amount = Money::parse(amount_cell, cur).ok()?;
    let (quantity, unit_price) = match cells.len() {
        2 => (1.0, amount),
        3 => match parse_qty(cells[1]) {
            Some(q) if q > 0.0 => (q, Money::new((amount.minor as f64 / q).round() as i64, amount.currency)),
            Some(_) => (1.0, amount),
            None => (1.0, Money::parse(cells[1], cur).ok()?),
        },
        n => (parse_qty(cells[1])?, Money::parse(cells[n - 2], cur).ok()?),
    };
    Some(LineItem { description: desc.to_string(), quantity, unit_price, amount })
}

fn is_item_header(line: &str) -> bool {
    let l = line.to_lowercase();
    (l.contains("description") || l.contains("item")) && ["qty", "quantity", "amount", "price", "total"].iter().any(|k| l.contains(k))
}

/// Reads the item table: rows between a header line ("Description / Qty / Amount") and the first
/// subtotal/tax/total row. Rows are `desc \t amount`, `desc \t qty \t amount`, or
/// `desc \t qty \t unit \t amount`.
///
/// Receipts often print items with no header ("NASI GORENG ⇥ 25.000"). Then every row above the
/// first summary row that reads as a purchase is an item: a description with letters, a positive
/// amount in its own column, and no header or payment wording.
pub fn extract_line_items(text: &str, default_currency: Currency) -> Vec<LineItem> {
    let lines = split_lines(text);
    let cur = detect_currency(text).map_or(default_currency, |(c, _)| c);
    let Some(h) = lines.iter().position(|l| is_item_header(l)) else {
        return headerless_items(&lines, cur);
    };
    let mut out = Vec::new();
    for line in &lines[h + 1..] {
        if classify_summary(line).is_some() { break; }
        let c = cells(line);
        if c.len() < 2 { continue; }
        if let Some(item) = parse_item(&c, cur) { out.push(item); }
    }
    out
}

fn headerless_items(lines: &[&str], cur: Currency) -> Vec<LineItem> {
    // Without a summary row there is no way to tell items from the rest of the page.
    let Some(end) = lines.iter().position(|l| classify_summary(l).is_some()) else { return Vec::new() };
    let mut out = Vec::new();
    for line in &lines[..end] {
        let joined = join_split_numbers(line);
        let c = cells(&joined);
        if c.len() < 2 || c.len() > 4 || non_item_re().is_match(c[0]) || date_re().is_match(&joined) { continue; }
        // The description needs words, not just a code ("901016").
        if c[0].chars().filter(|ch| ch.is_alphabetic()).count() < 3 { continue; }
        let mut row: Vec<&str> = c.clone();
        let mut quantity = None;
        let description;
        if let Some(caps) = lead_qty_re().captures(c[0]) {
            quantity = caps[1].parse::<f64>().ok().filter(|q| *q > 0.0);
            // "1X M-Caramel Tea @28,000": the unit price after "@" is not part of the name.
            description = caps[2].split('@').next().unwrap_or("").trim().to_string();
        } else {
            description = c[0].split('@').next().unwrap_or("").trim().to_string();
        }
        if description.chars().filter(|ch| ch.is_alphabetic()).count() < 3 { continue; }
        row[0] = &description;
        let Some(mut item) = parse_item(&row, cur) else { continue };
        if item.amount.minor <= 0 { continue; }
        if let Some(q) = quantity.filter(|_| row.len() == 2) {
            item.quantity = q;
            item.unit_price = Money::new((item.amount.minor as f64 / q).round() as i64, item.amount.currency);
        }
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "ACME Trading Ltd\nINVOICE\nInvoice No: INV-2024-0042\nInvoice Date: March 5, 2024\nDue Date: 2024-04-04\n\
        Description\tQty\tUnit Price\tAmount\nWidget\t2\t$50.00\t$100.00\nGadget\t1\t$25.00\t$25.00\n\
        Subtotal\t$125.00\nVAT (15%)\t$18.75\nTotal\t$143.75";

    fn get(fields: &[ExtractedField], n: FieldName) -> Option<&ExtractedField> { fields.iter().find(|f| f.name == n) }
    fn usd(minor: i64) -> Money { Money::new(minor, Currency::USD) }

    #[test]
    fn extracts_a_full_invoice() {
        let f = extract_fields(SAMPLE, Currency::USD);
        assert_eq!(get(&f, FieldName::Vendor).unwrap().value, FieldValue::Text("ACME Trading Ltd".into()));
        assert_eq!(get(&f, FieldName::InvoiceNumber).unwrap().value, FieldValue::Text("INV-2024-0042".into()));
        assert_eq!(get(&f, FieldName::InvoiceDate).unwrap().value, FieldValue::Date(Date::new(2024, 3, 5).unwrap()));
        assert_eq!(get(&f, FieldName::DueDate).unwrap().value, FieldValue::Date(Date::new(2024, 4, 4).unwrap()));
        assert_eq!(get(&f, FieldName::Subtotal).unwrap().value, FieldValue::Money(usd(12500)));
        assert_eq!(get(&f, FieldName::Tax).unwrap().value, FieldValue::Money(usd(1875)));
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(usd(14375)));
        assert_eq!(get(&f, FieldName::Currency).unwrap().value, FieldValue::Text("USD".into()));
        assert!(f.iter().all(|x| x.tier == Tier::Rules));
    }

    #[test]
    fn due_date_is_not_mistaken_for_invoice_date() {
        let f = extract_fields("Acme Ltd\nDue Date: 2024-04-04\nPayment Date: 2024-05-01\nInvoice Date: 2024-03-05", Currency::USD);
        assert_eq!(get(&f, FieldName::InvoiceDate).unwrap().value, FieldValue::Date(Date::new(2024, 3, 5).unwrap()));
        assert_eq!(get(&f, FieldName::DueDate).unwrap().value, FieldValue::Date(Date::new(2024, 4, 4).unwrap()));
        // with only a due date present there must be no invoice date at all
        let g = extract_fields("Acme Ltd\nDue Date: 2024-04-04", Currency::USD);
        assert!(get(&g, FieldName::InvoiceDate).is_none());
    }

    #[test]
    fn bare_date_label_and_ambiguity_penalty() {
        let clear = extract_fields("Acme Ltd\nDate: 2024-03-05", Currency::USD);
        let amb = extract_fields("Acme Ltd\nDate: 03/05/2024", Currency::USD);
        let unambiguous_slash = extract_fields("Acme Ltd\nDate: 25/03/2024", Currency::USD);
        let (c, a, u) = (
            get(&clear, FieldName::InvoiceDate).unwrap().confidence.get(),
            get(&amb, FieldName::InvoiceDate).unwrap().confidence.get(),
            get(&unambiguous_slash, FieldName::InvoiceDate).unwrap(),
        );
        assert!(a < c, "ambiguous dd/mm should lose confidence ({a} vs {c})");
        assert_eq!(u.value, FieldValue::Date(Date::new(2024, 3, 25).unwrap()));
        assert!((u.confidence.get() - c).abs() < 1e-6);
    }

    #[test]
    fn tax_id_lines_are_not_tax_amounts() {
        let f = extract_fields("Acme Ltd\nTax ID: 123456789\nVAT No. GB123456\nTotal\t$10.00", Currency::USD);
        assert!(get(&f, FieldName::Tax).is_none());
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(usd(1000)));
    }

    #[test]
    fn percent_only_tax_line_yields_nothing() {
        let f = extract_fields("Acme Ltd\nVAT 15%\nTotal\t$10.00", Currency::USD);
        assert!(get(&f, FieldName::Tax).is_none());
    }

    #[test]
    fn european_amounts_and_codes() {
        let f = extract_fields("Muster GmbH\nGesamt\nTotal\t1.234,56 EUR", Currency::USD);
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(Money::new(123456, Currency::EUR)));
        assert_eq!(get(&f, FieldName::Currency).unwrap().value, FieldValue::Text("EUR".into()));
    }

    #[test]
    fn total_prefers_strong_label_then_larger_amount() {
        let f = extract_fields("Acme Ltd\nTotal VAT\t$15.00\nTotal\t$100.00\nAmount Due\t$115.00", Currency::USD);
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(usd(11500)));
        assert_eq!(get(&f, FieldName::Tax).unwrap().value, FieldValue::Money(usd(1500)));
        let g = extract_fields("Acme Ltd\nTotal VAT\t$15.00\nTotal\t$100.00", Currency::USD);
        assert_eq!(get(&g, FieldName::Total).unwrap().value, FieldValue::Money(usd(10000)));
    }

    #[test]
    fn multiple_tax_lines_are_summed() {
        let f = extract_fields("Acme Ltd\nSubtotal\t$100.00\nSales Tax\t$5.00\nGST\t$10.00\nTotal\t$115.00", Currency::USD);
        assert_eq!(get(&f, FieldName::Tax).unwrap().value, FieldValue::Money(usd(1500)));
    }

    #[test]
    fn total_line_ignores_dates_on_the_same_line() {
        let f = extract_fields("Acme Ltd\nTotal due by 2024-04-01\t$50.00", Currency::USD);
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(usd(5000)));
    }

    #[test]
    fn currency_evidence() {
        assert_eq!(detect_currency("all in GHS, ₵5 fee"), Some((Currency::GHS, 0.95)));
        assert_eq!(detect_currency("₵5 and ₵6, $1"), Some((Currency::GHS, 0.85)));
        assert_eq!(detect_currency("no marks 12.00"), None);
        assert_eq!(detect_currency("TOTAL Rp 38.500\nCASH Rp.50,000"), Some((Currency::IDR, 0.85)));
        assert_eq!(detect_currency("Total IDR 38.500"), Some((Currency::IDR, 0.95)));
        assert_eq!(detect_currency("Rpt 4 of 9"), None, "letters alone are not a currency");
        // no evidence -> default used for parsing, but no Currency field emitted
        let f = extract_fields("Acme Ltd\nTotal\t12.00", Currency::GHS);
        assert_eq!(get(&f, FieldName::Total).unwrap().value, FieldValue::Money(Money::new(1200, Currency::GHS)));
        assert!(get(&f, FieldName::Currency).is_none());
    }

    #[test]
    fn a_title_line_outranks_labels() {
        assert_eq!(classify_kind("GitHub, Inc.\nRECEIPT\nInvoice No: 58210\nTotal $35.00"), DocKind::Receipt);
        assert_eq!(classify_kind("Acme Ltd\nTax Invoice\nReceipt No: 9"), DocKind::Invoice);
        assert_eq!(classify_kind("Bank\nStatement of Account\nInvoice ref 4"), DocKind::Statement);
        assert_eq!(classify_kind("Acme\nInvoice No: 4\nThanks"), DocKind::Invoice, "labels still classify");
    }

    #[test]
    fn vendor_heuristics() {
        let labelled = extract_fields("INVOICE\nFrom: Globex Inc\tPage 1", Currency::USD);
        let v = get(&labelled, FieldName::Vendor).unwrap();
        assert_eq!(v.value, FieldValue::Text("Globex Inc".into()));
        assert!((v.confidence.get() - 0.9).abs() < 1e-6);

        let plain = extract_fields("Sunrise Bakery\nInvoice #7", Currency::USD);
        assert!(get(&plain, FieldName::Vendor).unwrap().confidence.get() < 0.85, "no company suffix -> unsure");

        for name in ["Hotel Baixa Lisboa Lda", "Fratelli Rossi S.r.l", "Nordvik Oy", "Kumar Traders Pvt. Ltd."] {
            let found = extract_fields(&format!("{name}\nInvoice #7"), Currency::EUR);
            let vendor = get(&found, FieldName::Vendor).unwrap();
            assert!(vendor.confidence.get() >= 0.85, "{name} has an international company suffix");
        }

        let skips_titles = extract_fields("TAX INVOICE\nDate: 2024-01-01\nInitech Corp", Currency::USD);
        assert_eq!(get(&skips_titles, FieldName::Vendor).unwrap().value, FieldValue::Text("Initech Corp".into()));
        assert!(extract_fields("12345\n2024-01-01", Currency::USD).iter().all(|f| f.name != FieldName::Vendor));
    }

    #[test]
    fn invoice_number_needs_a_digit_and_a_label() {
        let f = |t: &str| get(&extract_fields(t, Currency::USD), FieldName::InvoiceNumber).map(|x| x.value.clone());
        assert_eq!(f("Invoice #42"), Some(FieldValue::Text("42".into())));
        assert_eq!(f("Invoice Number 12345"), Some(FieldValue::Text("12345".into())));
        assert_eq!(f("Invoice: A-7/9"), Some(FieldValue::Text("A-7/9".into())));
        assert_eq!(f("Invoice November 2024"), None);
        assert_eq!(f("Invoice Date: 2024-01-01"), None);
        assert_eq!(f("INV-001"), None);
    }

    #[test]
    fn line_items_variants_and_stop_at_summary() {
        let items = extract_line_items(SAMPLE, Currency::USD);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], LineItem { description: "Widget".into(), quantity: 2.0, unit_price: usd(5000), amount: usd(10000) });

        let t = "Item\tAmount\nConsulting\t$300.00\nTax consulting\t1\t$500.00\t$500.00\nSubtotal\t$800.00";
        let it = extract_line_items(t, Currency::USD);
        assert_eq!(it.len(), 2, "an item mentioning 'tax' must not end the table");
        assert_eq!(it[0].quantity, 1.0);

        let three = extract_line_items("Description\tQty\tAmount\nBolts\t4\t$10.00\nSetup fee\t$5.00\t$5.00\nTotal\t$15.00", Currency::USD);
        assert_eq!(three[0].unit_price, usd(250)); // 10.00 / 4
        assert_eq!(three[1].quantity, 1.0);        // middle cell is a price, not a quantity
        assert_eq!(three[1].unit_price, usd(500));

        assert!(extract_line_items("no table here\nTotal\t$1.00", Currency::USD).is_empty());
    }

    #[test]
    fn receipt_ocr_quirks() {
        let idr = |minor: i64| Money::new(minor, Currency::IDR);
        let f = |t: &str| extract_fields(t, Currency::USD);
        // Split thousands are rejoined; count lines and payment cells are not the total.
        let r = f("Warung\nSUBTOTAL\t15,000\nTax\t1, 500\nTotal quantity\t3\nTOTAL\t16, 500\nCASH\t20,000");
        assert_eq!(get(&r, FieldName::Total).unwrap().value, FieldValue::Money(usd(1_650_000)));
        assert_eq!(get(&r, FieldName::Tax).unwrap().value, FieldValue::Money(usd(150_000)));
        let r = f("Warung\nSUBTOTAL\t23,000\nTOTAL QTY\t2 ITEM 2 PCS\nTotal Rty 21");
        assert!(get(&r, FieldName::Total).is_none(), "item counts are never a total");
        let r = f("Cafe\nTOTAL (2 item)\t120,000");
        assert_eq!(get(&r, FieldName::Total).unwrap().value, FieldValue::Money(usd(12_000_000)));
        let r = f("Cafe\nTOTAL (Qty=4.00)240.000");
        assert_eq!(get(&r, FieldName::Total).unwrap().value, FieldValue::Money(usd(24_000_000)));
        let r = f("Cafe\nTotal\tChange :8,000\nTotal. Pay Cash 50.000\t41 ,000");
        assert_eq!(get(&r, FieldName::Total).unwrap().value, FieldValue::Money(usd(4_100_000)));
        assert_eq!(join_split_numbers("TOTAL 121 798"), "TOTAL 121,798");
        assert_eq!(join_split_numbers("TOTAL 80,500 100,000"), "TOTAL 80,500 100,000", "two amounts stay apart");
        assert_eq!(join_split_numbers("63, 000.00"), "63,000.00");

        // Indonesian tax and subtotal labels, and the rupiah symbol.
        let r = f("Resto\nSUBTTL\tRp 50.000\nPB1 10%\tRp 5.000\nTOTAL\tRp 55.000");
        assert_eq!(get(&r, FieldName::Subtotal).unwrap().value, FieldValue::Money(idr(5_000_000)));
        assert_eq!(get(&r, FieldName::Tax).unwrap().value, FieldValue::Money(idr(500_000)));
        assert_eq!(get(&r, FieldName::Currency).unwrap().value, FieldValue::Text("IDR".into()));
        for label in ["Pajak Resto (10%)", "P.Rest 10 %", "PPN 11%", "PBI"] {
            let r = f(&format!("Resto\nSub Total\t10,000\n{label}\t1,000\nTotal\t11,000"));
            assert_eq!(get(&r, FieldName::Tax).unwrap().value, FieldValue::Money(usd(100_000)), "{label}");
        }
        let r = f("Shop\nNET AMT\t7,273\nTOTAL\t8,000");
        assert_eq!(get(&r, FieldName::Subtotal).unwrap().value, FieldValue::Money(usd(727_300)));
    }

    #[test]
    fn headerless_receipt_items() {
        let text = "KOPI KENANGAN\nJl. Sudirman 12\nTable\t12\nCashier\tAndi\n12/03/2026 10:21\n\
            1X M-Caramel Black Tea @28,000\t28,000\nLess Ice\n2 NASI GORENG\t50.000\nJ. STB PROMO\t17500\n\
            901016\t-TICKET CP\nDisc item\t-5,000\nSubtotal\t95,500\nTotal\t95,500\nCash\t100,000";
        let items = extract_line_items(text, Currency::IDR);
        let names: Vec<&str> = items.iter().map(|item| item.description.as_str()).collect();
        assert_eq!(names, ["M-Caramel Black Tea", "NASI GORENG", "J. STB PROMO"]);
        assert_eq!(items[1].quantity, 2.0);
        assert_eq!(items[1].unit_price, Money::new(2_500_000, Currency::IDR));
        assert_eq!(items[2].amount, Money::new(1_750_000, Currency::IDR));
        // No summary row: nothing on the page can be told apart as an item.
        assert!(extract_line_items("Shop\nCoffee\t4.50", Currency::USD).is_empty());
    }

    #[test]
    fn kind_classification() {
        assert_eq!(classify_kind("ACME\nTAX INVOICE\n..."), DocKind::Invoice);
        assert_eq!(classify_kind("Corner Shop\nRECEIPT\nthanks"), DocKind::Receipt);
        assert_eq!(classify_kind("Bank\nAccount Statement"), DocKind::Statement);
        assert_eq!(classify_kind("random note"), DocKind::Other);
        // heading wins over a later mention
        assert_eq!(classify_kind("Shop\nReceipt\nl1\nl2\nl3\nl4\nl5\nsee our invoice terms"), DocKind::Receipt);
        // but a body mention is used when the heading says nothing
        assert_eq!(classify_kind("a\nb\nc\nd\ne\nf\nplease pay this invoice"), DocKind::Invoice);
    }
}
