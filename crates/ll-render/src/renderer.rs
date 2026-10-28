use crate::{image::Image, pnm};
use ll_core::{Error, Result};
use std::{fs::File, io::Read, path::Path};

/// Turns a document file into page images. A pdfium-backed implementation lives behind this trait.
pub trait PageRenderer: Send + Sync {
    /// File extensions (lowercase, no dot) this renderer can open.
    fn supports(&self, ext: &str) -> bool;

    fn page_count(&self, path: &Path) -> Result<usize>;
    /// Renders zero-based `page` at `dpi`.
    fn render_page(&self, path: &Path, page: usize, dpi: u32) -> Result<Image>;
}

/// Treats `.pgm` / `.ppm` files as single-page documents. Used for tests and scanner dumps.
pub struct PnmRenderer;

impl PageRenderer for PnmRenderer {
    fn supports(&self, ext: &str) -> bool {
        matches!(ext, "pgm" | "ppm") 
    }


    fn page_count(&self, path: &Path) -> Result<usize> {
        if !path.exists() { 
            return Err(Error::NotFound(path.display().to_string())); 
        }
        Ok(1)
    }


    fn render_page(&self, path: &Path, page: usize, _dpi: u32) -> Result<Image> {
        if page != 0 {
             return Err(Error::NotFound(format!("page {page} of {}", path.display())));
             }

        let file = File::open(path)?;
            if file.metadata()?.len() > pnm::MAX_PNM_FILE_BYTES {
                return Err(Error::validation("PNM file exceeds the size limit"));
            }
            
        let mut bytes = Vec::new();
        file.take(pnm::MAX_PNM_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > pnm::MAX_PNM_FILE_BYTES {
            return Err(Error::validation("PNM file exceeds the size limit"));
        }
        pnm::decode(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_pnm_file() {
        let dir = std::env::temp_dir().join(format!("ll-render-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.pgm");
        let im = Image::new(2, 2, 1, vec![1, 2, 3, 4]).unwrap();
        std::fs::write(&p, pnm::encode(&im)).unwrap();
        let r = PnmRenderer;
        assert!(r.supports("pgm") && !r.supports("pdf"));
        assert_eq!(r.page_count(&p).unwrap(), 1);
        assert_eq!(r.render_page(&p, 0, 200).unwrap(), im);
        assert!(r.render_page(&p, 1, 200).is_err());
        let oversized = dir.join("oversized.pgm");
        File::create(&oversized).unwrap().set_len(pnm::MAX_PNM_FILE_BYTES + 1).unwrap();
        assert!(r.render_page(&oversized, 0, 200).is_err());
        assert!(r.page_count(&dir.join("missing.pgm")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
