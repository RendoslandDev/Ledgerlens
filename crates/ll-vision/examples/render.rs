//! Writes the grayscale page Vision sees as PGM: `render -- <file> <out.pgm>`.
use ll_vision::{QuartzRenderer, SourceKind};
fn main() {
    let mut args = std::env::args().skip(1);
    let (input, output) = (args.next().expect("input"), args.next().expect("output"));
    let bytes = std::fs::read(&input).expect("readable input");
    let kind = if input.to_lowercase().ends_with(".pdf") { SourceKind::Pdf } else { SourceKind::Raster };
    let page = QuartzRenderer.render_bytes(kind, &bytes, 0, 200).expect("rendered page");
    std::fs::write(output, ll_render::pnm::encode(&page)).expect("written");
}
