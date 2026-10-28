use crate::layout::{group_lines, OcrPage, OcrWord};
use ll_core::{BBox, Confidence, Error, Result};
use ll_render::Image;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Anything that can turn one page image into positioned words (normalised 0..1 coordinates).
///
/// A production engine (detector + CTC recogniser running on an `ll-infer` session) implements
/// this; callers should go through [`recognize_page`] rather than calling `recognize` directly so
/// that output is sanitised and grouped into lines.
pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &str;
    fn recognize(&self, page: &Image) -> Result<Vec<OcrWord>>;
}

/// Runs `engine` on `img`, drops unusable words, clamps boxes/confidences into range, and groups
/// the result into reading-order lines.
pub fn recognize_page(engine: &dyn OcrEngine, img: &Image) -> Result<OcrPage> {
    if img.width() == 0 || img.height() == 0 {
        return Err(Error::validation("cannot OCR an empty image"));
    }
    let words = engine.recognize(img)?.into_iter().filter_map(sanitize).collect();
    Ok(group_lines(words))
}

fn sanitize(mut w: OcrWord) -> Option<OcrWord> {
    let b = w.bbox;
    if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) || b.width() <= 0.0 || b.height() <= 0.0 {
        return None;
    }
    let x0 = b.x0.clamp(0.0, 1.0);
    let y0 = b.y0.clamp(0.0, 1.0);
    let x1 = b.x1.clamp(0.0, 1.0);
    let y1 = b.y1.clamp(0.0, 1.0);
    if x1 <= x0 || y1 <= y0 {
        return None; // entirely off-page
    }
    w.bbox = BBox::new(x0, y0, x1, y1);
    w.conf = Confidence::new(w.conf).get();
    Some(w)
}

/// Deterministic engine for tests: returns pre-baked words, one page per call, in call order.
/// Errors once the script runs out so a test that OCRs more pages than expected fails loudly.
pub struct ScriptedEngine {
    pages: Vec<Vec<OcrWord>>,
    next: AtomicUsize,
}

impl ScriptedEngine {
    pub fn new(pages: Vec<Vec<OcrWord>>) -> Self {
        ScriptedEngine { pages, next: AtomicUsize::new(0) }
    }

    /// Builds pages from plain text. Each `\n` starts a new line, each `\t` becomes a wide column
    /// gap, and words are laid out left to right, so `recognize_page` reproduces the input text
    /// (blank lines are kept as vertical space but do not survive line grouping).
    pub fn from_text(pages: &[&str]) -> Self {
        // One shared character width so every page fits inside the 0..1 box, even long lines.
        const TAB_GAP: f32 = 0.08;
        let width_units = |line: &str| -> (f32, f32) {
            let tabs = line.matches('\t').count() as f32;
            let chars = line.chars().filter(|c| *c != '\t').count() as f32;
            (chars, tabs)
        };
        let mut cw = 0.012f32;
        for line in pages.iter().flat_map(|p| p.lines()) {
            let (chars, tabs) = width_units(line);
            let avail = 0.9 - tabs * TAB_GAP;
            if chars > 0.0 && avail > 0.0 { cw = cw.min(avail / chars); }
        }
        cw = cw.max(0.001);

        let baked = pages
            .iter()
            .map(|page| {
                let n = page.lines().count().max(1) as f32;
                let step = (0.9 / n).min(0.03);
                let h = (step * 0.66).min(0.02);
                let mut words = Vec::new();
                for (row, line) in page.lines().enumerate() {
                    let y = 0.05 + row as f32 * step;
                    let mut x = 0.05f32;
                    for (ci, cell) in line.split('\t').enumerate() {
                        if ci > 0 { x += TAB_GAP; }
                        for (wi, word) in cell.split_whitespace().enumerate() {
                            if wi > 0 { x += cw * 0.5; }
                            let w = word.chars().count() as f32 * cw;
                            words.push(OcrWord { text: word.to_string(), bbox: BBox::new(x, y, x + w, y + h), conf: 0.95 });
                            x += w;
                        }
                    }
                }
                words
            })
            .collect();
        ScriptedEngine::new(baked)
    }

    /// How many pages have been requested so far.
    pub fn calls(&self) -> usize { self.next.load(Ordering::SeqCst) }
}

impl OcrEngine for ScriptedEngine {
    fn name(&self) -> &str { "scripted" }

    fn recognize(&self, _page: &Image) -> Result<Vec<OcrWord>> {
        let i = self.next.fetch_add(1, Ordering::SeqCst);
        self.pages
            .get(i)
            .cloned()
            .ok_or_else(|| Error::Inference(format!("scripted engine exhausted: call {} but only {} pages", i + 1, self.pages.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Image { Image::filled(8, 8, 1, 255).unwrap() }

    fn word(text: &str, x: f32, y: f32, w: f32, h: f32, conf: f32) -> OcrWord {
        OcrWord { text: text.into(), bbox: BBox::new(x, y, x + w, y + h), conf }
    }

    #[test]
    fn from_text_round_trips_through_layout() {
        let text = "ACME Corp\nInvoice #42\nWidget\t2\t$10.00\nTotal:\t$20.00";
        let e = ScriptedEngine::from_text(&[text]);
        let page = recognize_page(&e, &blank()).unwrap();
        assert_eq!(page.text(), text);
        assert_eq!(page.lines.len(), 4);
        assert!((page.mean_confidence() - 0.95).abs() < 1e-6);
    }

    #[test]
    fn long_lines_still_fit_on_the_page() {
        let text = format!("{}\nshort", "word ".repeat(60).trim());
        let e = ScriptedEngine::from_text(&[text.as_str()]);
        let page = recognize_page(&e, &blank()).unwrap();
        assert!(page.lines.iter().all(|l| l.bbox.right() <= 1.0 + 1e-4));
        assert_eq!(page.lines[0].words.len(), 60);
        assert_eq!(page.lines.len(), 2);
    }

    #[test]
    fn serves_pages_in_order_then_errors() {
        let e = ScriptedEngine::from_text(&["one", "two"]);
        assert_eq!(recognize_page(&e, &blank()).unwrap().text(), "one");
        assert_eq!(recognize_page(&e, &blank()).unwrap().text(), "two");
        assert_eq!(e.calls(), 2);
        let err = recognize_page(&e, &blank()).unwrap_err();
        assert!(matches!(err, Error::Inference(_)), "{err}");
    }

    #[test]
    fn sanitizes_bad_engine_output() {
        let e = ScriptedEngine::new(vec![vec![
            word("ok", 0.1, 0.1, 0.1, 0.02, 1.7),           // confidence clamped to 1
            word("nan", f32::NAN, 0.1, 0.1, 0.02, 0.9),     // dropped
            word("flat", 0.1, 0.3, 0.0, 0.02, 0.9),         // zero width: dropped
            word("off", 1.5, 0.3, 0.1, 0.02, 0.9),          // entirely off-page: dropped
            word("edge", 0.95, 0.5, 0.2, 0.02, f32::NAN),   // clipped to page, NaN conf -> 0
        ]]);
        let page = recognize_page(&e, &blank()).unwrap();
        let words: Vec<&OcrWord> = page.lines.iter().flat_map(|l| &l.words).collect();
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["ok", "edge"]);
        assert_eq!(words[0].conf, 1.0);
        assert_eq!(words[1].conf, 0.0);
        assert!((words[1].bbox.right() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_empty_image_without_calling_engine() {
        let e = ScriptedEngine::from_text(&["x"]);
        let empty = Image::new(0, 0, 1, vec![]).unwrap();
        assert!(recognize_page(&e, &empty).is_err());
        assert_eq!(e.calls(), 0);
    }

    #[test]
    fn works_as_trait_object() {
        let e: Box<dyn OcrEngine> = Box::new(ScriptedEngine::from_text(&["hello world"]));
        assert_eq!(e.name(), "scripted");
        assert_eq!(recognize_page(e.as_ref(), &blank()).unwrap().text(), "hello world");
    }
}
