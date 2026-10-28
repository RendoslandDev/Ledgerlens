//! Runs the real Vision recogniser over raster fixtures produced by macOS `sips` from a receipt PDF.
#![cfg(target_os = "macos")]

use ll_ocr::recognize_page;
use ll_vision::{QuartzRenderer, SourceKind, VisionEngine};

const PNG: &[u8] = include_bytes!("fixtures/receipt.png");
const JPEG: &[u8] = include_bytes!("fixtures/receipt.jpg");
const SCANNED_PDF: &[u8] = include_bytes!("fixtures/scanned.pdf");

fn read(kind: SourceKind, bytes: &[u8]) -> String {
    let renderer = QuartzRenderer;
    assert_eq!(renderer.page_count_bytes(kind, bytes).unwrap(), 1);
    let page = renderer.render_bytes(kind, bytes, 0, 200).unwrap();
    assert_eq!(page.channels(), 1);
    recognize_page(&VisionEngine, &page).unwrap().text()
}

fn assert_receipt(text: &str) {
    for expected in ["GitHub", "GH-INV-58210", "2026-10-03", "Copilot Business", "35.00"] {
        assert!(text.contains(expected), "missing {expected:?} in OCR text:\n{text}");
    }
    let total = text.lines().find(|line| line.starts_with("Total")).expect("a total line");
    assert!(total.contains('\t'), "the amount is a separate column: {total:?}");
}

#[test]
fn reads_a_png_receipt() {
    assert_receipt(&read(SourceKind::Raster, PNG));
}

#[test]
fn reads_a_compressed_jpeg_receipt() {
    assert_receipt(&read(SourceKind::Raster, JPEG));
}

#[test]
fn reads_a_scanned_pdf_without_a_text_layer() {
    assert_receipt(&read(SourceKind::Pdf, SCANNED_PDF));
}

#[test]
fn rejects_garbage_and_missing_pages() {
    let renderer = QuartzRenderer;
    assert!(renderer.render_bytes(SourceKind::Raster, b"\x89PNG\r\n\x1a\nnope", 0, 200).is_err());
    assert!(renderer.render_bytes(SourceKind::Pdf, b"%PDF-1.4 broken", 0, 200).is_err());
    assert!(renderer.render_bytes(SourceKind::Pdf, SCANNED_PDF, 1, 200).is_err());
    assert!(renderer.render_bytes(SourceKind::Raster, PNG, 1, 200).is_err());
}
