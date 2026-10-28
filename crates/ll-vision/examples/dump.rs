//! Prints Vision's raw observations for a file: `cargo run -p ll-vision --example dump -- <file>`.
use ll_ocr::OcrEngine;
use ll_vision::{QuartzRenderer, SourceKind, VisionEngine};
fn main() {
    let path = std::env::args().nth(1).expect("usage: dump <file>");
    let bytes = std::fs::read(&path).expect("readable file");
    let kind = if path.to_lowercase().ends_with(".pdf") { SourceKind::Pdf } else { SourceKind::Raster };
    let page = QuartzRenderer.render_bytes(kind, &bytes, 0, std::env::args().nth(2).and_then(|d| d.parse().ok()).unwrap_or(200)).expect("rendered page");
    println!("{}x{}", page.width(), page.height());
    for word in VisionEngine.recognize(&page).expect("recognised text") {
        let b = word.bbox;
        println!("{:.3} {:.3}-{:.3} h={:.4} conf={:.2} {:?}", b.x0, b.y0, b.y1, b.height(), word.conf, word.text);
    }
}
