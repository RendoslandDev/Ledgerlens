//! The macOS implementation. All `unsafe` in Ledgerlens's OCR path lives here, and each block
//! states the invariant it relies on.

use crate::{fit_page, SourceKind, MAX_OCR_PAGES, MAX_PHOTO_SIDE};
use ll_core::{BBox, Error, Result};
use ll_ocr::OcrWord;
use ll_render::Image;
use objc2::{rc::autoreleasepool, AnyThread};
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage,
    CGImageAlphaInfo, CGInterpolationQuality, CGPDFBox, CGPDFDocument, CGPDFPage,
};
use objc2_foundation::{NSArray, NSDictionary};
use objc2_image_io::{
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceShouldCache, kCGImageSourceThumbnailMaxPixelSize, CGImageSource,
};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedTextObservation, VNRequest,
    VNRequestTextRecognitionLevel,
};

pub fn page_count(kind: SourceKind, bytes: &[u8]) -> Result<usize> {
    match kind {
        SourceKind::Pdf => {
            let document = pdf(bytes)?;
            Ok(CGPDFDocument::number_of_pages(Some(&document)).min(MAX_OCR_PAGES))
        }
        SourceKind::Raster => {
            image_source(bytes)?;
            Ok(1)
        }
    }
}

pub fn render(kind: SourceKind, bytes: &[u8], page: usize, dpi: u32) -> Result<Image> {
    autoreleasepool(|_| match kind {
        SourceKind::Pdf => render_pdf_page(bytes, page, dpi),
        SourceKind::Raster if page == 0 => render_photo(bytes),
        SourceKind::Raster => Err(Error::NotFound(format!("page {page} of an image"))),
    })
}

fn pdf(bytes: &[u8]) -> Result<CFRetained<CGPDFDocument>> {
    let data = CFData::from_bytes(bytes);
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .ok_or_else(|| Error::Invalid("could not read the PDF".into()))?;
    let document = CGPDFDocument::with_provider(Some(&provider))
        .ok_or_else(|| Error::Invalid("the PDF could not be opened".into()))?;
    if CGPDFDocument::is_encrypted(Some(&document)) && !CGPDFDocument::is_unlocked(Some(&document)) {
        return Err(Error::Invalid("the PDF is password protected".into()));
    }
    Ok(document)
}

fn render_pdf_page(bytes: &[u8], page: usize, dpi: u32) -> Result<Image> {
    let document = pdf(bytes)?;
    if page >= CGPDFDocument::number_of_pages(Some(&document)).min(MAX_OCR_PAGES) {
        return Err(Error::NotFound(format!("page {}", page + 1)));
    }
    // CoreGraphics pages are one-based.
    let pdf_page = CGPDFDocument::page(Some(&document), page + 1)
        .ok_or_else(|| Error::NotFound(format!("page {}", page + 1)))?;
    let media = CGPDFPage::box_rect(Some(&pdf_page), CGPDFBox::CropBox);
    let rotated = CGPDFPage::rotation_angle(Some(&pdf_page)).rem_euclid(180) == 90;
    let (page_w, page_h) = if rotated {
        (media.size.height, media.size.width)
    } else {
        (media.size.width, media.size.height)
    };
    let (width, height) =
        fit_page(page_w, page_h, dpi).ok_or_else(|| Error::Invalid("the PDF page has no size".into()))?;

    draw_gray(width, height, |context| {
        let target = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(page_w, page_h));
        CGContext::scale_ctm(Some(context), width as f64 / page_w, height as f64 / page_h);
        let transform = CGPDFPage::drawing_transform(
            Some(&pdf_page),
            CGPDFBox::CropBox,
            target,
            0,
            true,
        );
        CGContext::concat_ctm(Some(context), transform);
        CGContext::draw_pdf_page(Some(context), Some(&pdf_page));
    })
}

fn image_source(bytes: &[u8]) -> Result<CFRetained<CGImageSource>> {
    let data = CFData::from_bytes(bytes);
    // SAFETY: `None` is a valid options dictionary.
    let source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| Error::Invalid("the image could not be decoded".into()))?;
    // SAFETY: `source` is a live image source.
    if unsafe { source.count() } == 0 {
        return Err(Error::Invalid("the image contains no frames".into()));
    }
    Ok(source)
}

fn render_photo(bytes: &[u8]) -> Result<Image> {
    let source = image_source(bytes)?;
    // Decode the first frame upright (EXIF orientation applied) and no larger than the cap.
    let max_side = CFNumber::new_i64(MAX_PHOTO_SIDE as i64);
    let keys: [&CFString; 4] = unsafe {
        // SAFETY: these are ImageIO's exported constant keys, valid for the process lifetime.
        [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceThumbnailMaxPixelSize,
            kCGImageSourceShouldCache,
        ]
    };
    let yes = CFBoolean::new(true);
    let no = CFBoolean::new(false);
    let values: [&objc2_core_foundation::CFType; 4] = [yes.as_ref(), yes.as_ref(), max_side.as_ref(), no.as_ref()];
    let options = CFDictionary::from_slices(&keys, &values);
    // SAFETY: the dictionary maps ImageIO option keys to the CF types ImageIO documents for them.
    let image = unsafe { source.thumbnail_at_index(0, Some(options.as_opaque())) }
        .ok_or_else(|| Error::Invalid("the image could not be decoded".into()))?;
    let (width, height) = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    if width == 0 || height == 0 {
        return Err(Error::Invalid("the image is empty".into()));
    }
    draw_gray(width, height, |context| {
        CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::High);
        let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width as f64, height as f64));
        CGContext::draw_image(Some(context), rect, Some(&image));
    })
}

/// Paints into an 8-bit grayscale bitmap that starts white, and returns it as an [`Image`].
fn draw_gray(width: usize, height: usize, paint: impl FnOnce(&CGContext)) -> Result<Image> {
    let mut pixels = vec![255u8; width.checked_mul(height).ok_or_else(|| Error::Invalid("page too large".into()))?];
    let space = CGColorSpace::new_device_gray().ok_or_else(|| Error::Inference("no gray color space".into()))?;
    {
        // SAFETY: `pixels` holds exactly `height` rows of `width` bytes (8-bit gray, no alpha) and
        // outlives `context`, which is dropped at the end of this block before `pixels` moves.
        let context = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width,
                Some(&space),
                CGImageAlphaInfo::None.0,
            )
        }
        .ok_or_else(|| Error::Inference("could not create a drawing surface".into()))?;
        paint(&context);
        CGContext::flush(Some(&context));
    }
    Image::new(width, height, 1, pixels)
}

pub fn recognize(page: &Image) -> Result<Vec<OcrWord>> {
    autoreleasepool(|_| {
        let image = to_cg_image(page)?;
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        request.setAutomaticallyDetectsLanguage(true);

        let options = NSDictionary::new();
        // SAFETY: an empty dictionary is a valid options argument.
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(VNImageRequestHandler::alloc(), &image, &options)
        };
        let as_request: &VNRequest = &request;
        let requests = NSArray::from_slice(&[as_request]);
        handler
            .performRequests_error(&requests)
            .map_err(|error| Error::Inference(format!("text recognition failed: {}", error.localizedDescription())))?;

        let results = request.results().unwrap_or_default();
        let mut words = Vec::new();
        for observation in results.iter() {
            let Ok(text_observation) = observation.downcast::<VNRecognizedTextObservation>() else {
                continue;
            };
            let candidates = text_observation.topCandidates(1);
            let Some(best) = candidates.firstObject() else { continue };
            let text = best.string().to_string();
            if text.trim().is_empty() {
                continue;
            }
            // SAFETY: plain property reads on a live observation.
            let (rect, confidence) = unsafe { (text_observation.boundingBox(), best.confidence()) };
            // Vision boxes are normalised with the origin at the bottom left; ll-ocr wants top left.
            let (x, y, w, h) = (rect.origin.x as f32, rect.origin.y as f32, rect.size.width as f32, rect.size.height as f32);
            words.push(OcrWord { text, bbox: BBox::new(x, 1.0 - (y + h), x + w, 1.0 - y), conf: confidence });
        }
        Ok(words)
    })
}

fn to_cg_image(page: &Image) -> Result<CFRetained<CGImage>> {
    let (space, alpha) = match page.channels() {
        1 => (CGColorSpace::new_device_gray(), CGImageAlphaInfo::None),
        3 => (CGColorSpace::new_device_rgb(), CGImageAlphaInfo::None),
        4 => (CGColorSpace::new_device_rgb(), CGImageAlphaInfo::NoneSkipLast),
        other => return Err(Error::Invalid(format!("cannot OCR a {other}-channel image"))),
    };
    let space = space.ok_or_else(|| Error::Inference("no color space".into()))?;
    let data = CFData::from_bytes(page.data());
    let provider = CGDataProvider::with_cf_data(Some(&data))
        .ok_or_else(|| Error::Inference("could not wrap the page image".into()))?;
    let channels = page.channels();
    // SAFETY: the provider holds `width * height * channels` bytes laid out row by row, matching
    // the geometry passed here; `decode` is null, which CoreGraphics accepts.
    unsafe {
        CGImage::new(
            page.width(),
            page.height(),
            8,
            8 * channels,
            page.width() * channels,
            Some(&space),
            objc2_core_graphics::CGBitmapInfo(alpha.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| Error::Inference("could not build the page image".into()))
}
