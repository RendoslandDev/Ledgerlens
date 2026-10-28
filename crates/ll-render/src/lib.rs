//! Image buffers, preprocessing for OCR/vision models, and the [`PageRenderer`] seam.
//!
//! A pdfium-backed renderer implements [`PageRenderer`]; this crate ships a PNM renderer so the
//! pipeline is testable without native libraries.

pub mod image;
pub mod pnm;
pub mod preprocess;
pub mod renderer;

pub use image::Image;
pub use preprocess::{to_tensor_chw, Preprocess};
pub use renderer::{PageRenderer, PnmRenderer};
