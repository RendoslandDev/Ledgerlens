//! Text for the rules tier, from a PDF's own text layer or, when there is none, from on-device OCR
//! (Apple Vision on macOS). Both are shaped like `ll-ocr` output (`\n` between lines, `\t` at
//! column gaps, `\u{c}` between pages) so the rules read them the same way.

use crate::media::MediaType;
use std::{
    panic::{self, AssertUnwindSafe},
    sync::mpsc,
    thread,
    time::Duration,
};

/// Stored text is capped; extraction only ever needs the first pages of a paperwork document.
pub const MAX_TEXT_CHARS: usize = 200_000;
/// Fewer alphanumeric characters than this means the PDF is a scan with no usable text layer.
const MIN_USEFUL_CHARS: usize = 20;
/// A hostile PDF can make a parser loop; after this long the document is marked unreadable.
const PARSE_TIMEOUT: Duration = Duration::from_secs(20);
/// OCR of up to `ll_vision::MAX_OCR_PAGES` pages normally takes seconds; this bounds a hang.
const OCR_TIMEOUT: Duration = Duration::from_secs(90);
const OCR_DPI: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    /// The PDF's own text layer: exact characters.
    TextLayer,
    /// On-device OCR: characters can be misread.
    Ocr,
}

impl TextSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextLayer => "text_layer",
            Self::Ocr => "ocr",
        }
    }

    /// Reads the stored column value; anything unknown is treated as "no source".
    pub fn from_stored(value: &str) -> Option<Self> {
        [Self::TextLayer, Self::Ocr].into_iter().find(|source| source.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextOutcome {
    Text(String, TextSource),
    /// Readable, but nothing to read: a blank page, or a scan on a system without OCR.
    NoText,
    /// The parser or recogniser rejected or crashed on the file.
    Unreadable(String),
}

pub fn extract(media: MediaType, bytes: &[u8]) -> TextOutcome {
    if media.is_image() {
        return ocr(media, bytes);
    }
    match text_layer(bytes) {
        TextOutcome::NoText => ocr(media, bytes),
        // Quartz opens some PDFs the text-layer parser cannot; keep the parser's reason if not.
        TextOutcome::Unreadable(reason) => match ocr(media, bytes) {
            text @ TextOutcome::Text(..) => text,
            _ => TextOutcome::Unreadable(reason),
        },
        text => text,
    }
}

/// Runs `work` on a worker thread so a panic is contained and a hang is abandoned.
fn isolated<T: Send + 'static>(
    name: &str,
    timeout: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _ = sender.send(panic::catch_unwind(AssertUnwindSafe(work)));
        })
        .map_err(|_| "could not start a worker thread".to_string())?;
    match receiver.recv_timeout(timeout) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err("the reader failed on this file".into()),
        Err(_) => Err("reading the file took too long".into()),
    }
}

fn ocr(media: MediaType, bytes: &[u8]) -> TextOutcome {
    if !ll_vision::is_available() {
        return TextOutcome::NoText;
    }
    let kind = if media == MediaType::Pdf {
        ll_vision::SourceKind::Pdf
    } else {
        ll_vision::SourceKind::Raster
    };
    let owned = bytes.to_vec();
    let result = isolated("ll-ocr", OCR_TIMEOUT, move || -> ll_core::Result<String> {
        let renderer = ll_vision::QuartzRenderer;
        let pages = renderer.page_count_bytes(kind, &owned)?;
        let mut texts = Vec::with_capacity(pages);
        for page in 0..pages {
            let image = renderer.render_bytes(kind, &owned, page, OCR_DPI)?;
            texts.push(ll_ocr::recognize_page(&ll_vision::VisionEngine, &image)?.text());
        }
        Ok(texts.join("\u{c}"))
    });
    match result {
        Ok(Ok(raw)) => useful(normalize(&raw), TextSource::Ocr),
        Ok(Err(error)) => TextOutcome::Unreadable(format!("the document could not be read: {error}")),
        Err(reason) => TextOutcome::Unreadable(reason),
    }
}

fn useful(text: String, source: TextSource) -> TextOutcome {
    if text.chars().filter(|c| c.is_alphanumeric()).count() < MIN_USEFUL_CHARS {
        TextOutcome::NoText
    } else {
        TextOutcome::Text(text, source)
    }
}

fn text_layer(bytes: &[u8]) -> TextOutcome {
    // PDF parsers are large and the input is untrusted: parse in isolation.
    let owned = bytes.to_vec();
    let result = isolated("ll-pdf-text", PARSE_TIMEOUT, move || {
        pdf_extract::extract_text_from_mem(&owned).map_err(|error| error.to_string())
    });
    match result {
        Ok(Ok(raw)) => useful(normalize(&raw), TextSource::TextLayer),
        Ok(Err(error)) => TextOutcome::Unreadable(format!("the PDF could not be read: {error}")),
        Err(reason) => TextOutcome::Unreadable(reason),
    }
}

/// Trims each line, turns runs of two or more spaces into a column break, drops blank runs, and
/// caps the length.
pub fn normalize(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.split(['\n', '\r']) {
        let mut out = String::new();
        let mut spaces = 0usize;
        for character in line.trim().chars() {
            if character == ' ' || character == '\u{a0}' {
                spaces += 1;
                continue;
            }
            if character == '\t' {
                spaces = 2;
                continue;
            }
            if spaces == 1 {
                out.push(' ');
            } else if spaces > 1 {
                out.push('\t');
            }
            spaces = 0;
            if !character.is_control() || character == '\u{c}' {
                out.push(character);
            }
        }
        if !(out.is_empty() && lines.last().is_some_and(|last: &String| last.is_empty())) {
            lines.push(out);
        }
    }
    let text = lines.join("\n").trim().to_string();
    match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_spacing_becomes_columns() {
        let raw = "  Widget    2     $50.00  \n\n\n\nTotal Due        GHS 1,725.00   \n";
        assert_eq!(normalize(raw), "Widget\t2\t$50.00\n\nTotal Due\tGHS 1,725.00");
    }

    #[test]
    fn undecodable_images_are_unreadable_where_ocr_exists() {
        let outcome = extract(MediaType::Png, b"\x89PNG\r\n\x1a\nnot really");
        if ll_vision::is_available() {
            assert!(matches!(outcome, TextOutcome::Unreadable(_)), "{outcome:?}");
        } else {
            assert_eq!(outcome, TextOutcome::NoText);
        }
    }

    #[test]
    fn malformed_pdfs_are_reported_not_fatal() {
        let outcome = extract(MediaType::Pdf, b"%PDF-1.7\n1 0 obj << /Type /Catalog >> garbage");
        assert!(matches!(outcome, TextOutcome::Unreadable(_)), "{outcome:?}");
    }

    #[test]
    fn text_is_capped() {
        let long = "word ".repeat(MAX_TEXT_CHARS);
        assert_eq!(normalize(&long).chars().count(), MAX_TEXT_CHARS);
    }
}
