//! On-device OCR for Apple platforms (macOS and iOS). [`QuartzRenderer`] turns PDF pages and photos into page images with
//! CoreGraphics/ImageIO, and [`VisionEngine`] reads them with Apple's Vision text recogniser.
//! Nothing leaves the machine: both frameworks run locally.
//!
//! On other platforms both types exist but report that OCR is unavailable, so callers can treat
//! "no OCR" as an ordinary outcome.

use ll_core::{Error, Result};
use ll_ocr::{OcrEngine, OcrWord};
use ll_render::{Image, PageRenderer};
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod mac;

/// Pages beyond this are not rendered; paperwork totals live on the first pages.
pub const MAX_OCR_PAGES: usize = 10;
/// Upper bound on one rendered page, in pixels (about 16 megapixels).
pub const MAX_PAGE_PIXELS: usize = 16_000_000;
/// Longest side a photo is scaled to before recognition.
pub const MAX_PHOTO_SIDE: usize = 4096;

/// What kind of bytes are being rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Pdf,
    /// PNG, JPEG, or TIFF (any format ImageIO decodes).
    Raster,
}

/// True when this build can run OCR (macOS and iOS).
pub const fn is_available() -> bool {
    cfg!(any(target_os = "macos", target_os = "ios"))
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn unavailable<T>() -> Result<T> {
    Err(Error::Inference("on-device OCR needs Apple Vision (macOS or iOS)".into()))
}

/// Renders documents to grayscale page images with Quartz.
pub struct QuartzRenderer;

impl QuartzRenderer {
    pub fn page_count_bytes(&self, kind: SourceKind, bytes: &[u8]) -> Result<usize> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            mac::page_count(kind, bytes)
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            let _ = (kind, bytes);
            unavailable()
        }
    }

    /// Renders zero-based `page`. PDFs are rasterised at `dpi` (capped at [`MAX_PAGE_PIXELS`]);
    /// photos are decoded upright and scaled to at most [`MAX_PHOTO_SIDE`].
    pub fn render_bytes(&self, kind: SourceKind, bytes: &[u8], page: usize, dpi: u32) -> Result<Image> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            mac::render(kind, bytes, page, dpi)
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            let _ = (kind, bytes, page, dpi);
            unavailable()
        }
    }
}

fn kind_for(ext: &str) -> Option<SourceKind> {
    match ext {
        "pdf" => Some(SourceKind::Pdf),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" => Some(SourceKind::Raster),
        _ => None,
    }
}

fn kind_of(path: &Path) -> Result<SourceKind> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .and_then(|ext| kind_for(&ext))
        .ok_or_else(|| Error::Invalid(format!("unsupported document: {}", path.display())))
}

impl PageRenderer for QuartzRenderer {
    fn supports(&self, ext: &str) -> bool {
        is_available() && kind_for(ext).is_some()
    }

    fn page_count(&self, path: &Path) -> Result<usize> {
        self.page_count_bytes(kind_of(path)?, &std::fs::read(path)?)
    }

    fn render_page(&self, path: &Path, page: usize, dpi: u32) -> Result<Image> {
        self.render_bytes(kind_of(path)?, &std::fs::read(path)?, page, dpi)
    }
}

/// Apple Vision's accurate text recogniser. Each recognised text run becomes one [`OcrWord`]
/// (Vision already splits runs at column gaps), so `ll_ocr::recognize_page` rebuilds lines and
/// tab-separated columns from the boxes.
pub struct VisionEngine;

impl OcrEngine for VisionEngine {
    fn name(&self) -> &str {
        "apple-vision"
    }

    fn recognize(&self, page: &Image) -> Result<Vec<OcrWord>> {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            mac::recognize(page)
        }
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        {
            let _ = page;
            unavailable()
        }
    }
}

/// Size of a page rendered at `dpi`, scaled down to fit [`MAX_PAGE_PIXELS`].
pub(crate) fn fit_page(width_pt: f64, height_pt: f64, dpi: u32) -> Option<(usize, usize)> {
    if !(width_pt.is_finite() && height_pt.is_finite()) || width_pt <= 0.0 || height_pt <= 0.0 {
        return None;
    }
    let mut scale = f64::from(dpi.clamp(72, 600)) / 72.0;
    let pixels = width_pt * height_pt * scale * scale;
    if pixels > MAX_PAGE_PIXELS as f64 {
        scale *= (MAX_PAGE_PIXELS as f64 / pixels).sqrt();
    }
    let width = (width_pt * scale).floor() as usize;
    let height = (height_pt * scale).floor() as usize;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_fit_the_pixel_budget() {
        assert_eq!(fit_page(612.0, 792.0, 200), Some((1700, 2200)));
        let (w, h) = fit_page(14_400.0, 14_400.0, 300).unwrap();
        assert!(w * h <= MAX_PAGE_PIXELS);
        assert_eq!(fit_page(0.0, 792.0, 200), None);
        assert_eq!(fit_page(f64::NAN, 792.0, 200), None);
    }

    #[test]
    fn extensions_map_to_sources() {
        assert_eq!(kind_for("pdf"), Some(SourceKind::Pdf));
        assert_eq!(kind_for("jpeg"), Some(SourceKind::Raster));
        assert_eq!(kind_for("docx"), None);
    }
}
