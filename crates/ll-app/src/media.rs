//! File admission: what a document is decided by its bytes, never by its name.

use ll_core::{Error, Result};
use serde::Serialize;

/// Largest document accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_FILE_NAME_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "lowercase")]
pub enum MediaType {
    Pdf,
    Png,
    Jpeg,
    Tiff,
}

impl MediaType {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Tiff => "image/tiff",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Tiff => "tiff",
        }
    }

    pub fn from_mime(mime: &str) -> Result<Self> {
        [Self::Pdf, Self::Png, Self::Jpeg, Self::Tiff]
            .into_iter()
            .find(|media| media.mime() == mime)
            .ok_or_else(|| Error::Invalid(format!("unknown stored media type: {mime}")))
    }

    pub fn is_image(self) -> bool {
        !matches!(self, Self::Pdf)
    }
}

/// Identifies a supported document from its magic bytes and rejects everything else, including
/// empty and oversized input.
pub fn sniff(bytes: &[u8]) -> Result<MediaType> {
    if bytes.is_empty() {
        return Err(Error::Invalid("the file is empty".into()));
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::Invalid(format!(
            "the file is larger than the {} MB limit",
            MAX_FILE_BYTES / (1024 * 1024)
        )));
    }
    // PDF allows up to 1 KB of leading junk before the header.
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|window| window == b"%PDF-") {
        return Ok(MediaType::Pdf);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(MediaType::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(MediaType::Jpeg);
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Ok(MediaType::Tiff);
    }
    Err(Error::Invalid(
        "unsupported file: expected a PDF, PNG, JPEG, or TIFF document".into(),
    ))
}

/// A display name safe to store and show: no directories, no control characters, bounded length.
pub fn clean_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_FILE_NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "document".into()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_decides_the_type() {
        assert_eq!(sniff(b"%PDF-1.7\n...").unwrap(), MediaType::Pdf);
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest").unwrap(), MediaType::Png);
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap(), MediaType::Jpeg);
        assert_eq!(sniff(b"II*\0rest").unwrap(), MediaType::Tiff);
        assert!(sniff(b"MZ\x90\0 executable named invoice.pdf").is_err());
        assert!(sniff(b"<html><script>").is_err());
        assert!(sniff(b"").is_err());
    }

    #[test]
    fn oversized_files_are_rejected_before_parsing() {
        let mut big = b"%PDF-1.7".to_vec();
        big.resize(MAX_FILE_BYTES + 1, b' ');
        assert!(sniff(&big).is_err());
    }

    #[test]
    fn file_names_lose_paths_controls_and_leading_dots() {
        assert_eq!(clean_file_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_file_name("C:\\Users\\me\\inv.pdf"), "inv.pdf");
        assert_eq!(clean_file_name("bad\u{0}\nname.pdf"), "badname.pdf");
        assert_eq!(clean_file_name("..."), "document");
        assert_eq!(clean_file_name(".hidden.pdf"), "hidden.pdf");
        assert_eq!(clean_file_name(&"a".repeat(500)).len(), 120);
    }
}
