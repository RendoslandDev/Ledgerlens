use ll_core::BBox;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrWord {
    pub text: String,
    /// Normalised page coordinates (0..1).
    pub bbox: BBox,
    pub conf: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
    pub bbox: BBox,
    pub conf: f32,
}

impl OcrLine {
    /// Words joined by single spaces, or by a tab where a large horizontal gap suggests a column break.
    pub fn text(&self) -> String {
        let mut s = String::new();
        let avg_h = self.words.iter().map(|w| w.bbox.height()).sum::<f32>() / self.words.len().max(1) as f32;
        for (i, w) in self.words.iter().enumerate() {
            if i > 0 {
                let gap = w.bbox.x0 - self.words[i - 1].bbox.right();
                s.push(if gap > avg_h * 2.5 { '\t' } else { ' ' });
            }
            s.push_str(&w.text);
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OcrPage {
    pub lines: Vec<OcrLine>,
}

impl OcrPage {
    pub fn text(&self) -> String { self.lines.iter().map(|l| l.text()).collect::<Vec<_>>().join("\n") }
    pub fn mean_confidence(&self) -> f32 {
        let (s, n) = self.lines.iter().flat_map(|l| &l.words).fold((0.0, 0usize), |(s, n), w| (s + w.conf, n + 1));
        if n == 0 { 0.0 } else { s / n as f32 }
    }
}

/// Groups words into reading-order lines. A word joins the line whose vertical band it overlaps
/// most, provided the overlap covers at least half of the shorter of the two. A line's band is its
/// first word's extent and never grows, so tightly stacked rows (receipts printed with no leading)
/// cannot chain into one line.
pub fn group_lines(mut words: Vec<OcrWord>) -> OcrPage {
    words.retain(|w| !w.text.trim().is_empty());
    words.sort_by(|a, b| a.bbox.center_y().total_cmp(&b.bbox.center_y()));

    let mut lines: Vec<(f32, f32, Vec<OcrWord>)> = Vec::new();
    for w in words {
        let (top, bottom) = (w.bbox.y0, w.bbox.bottom());
        let best = lines
            .iter_mut()
            .filter_map(|line| {
                let overlap = bottom.min(line.1) - top.max(line.0);
                let shorter = (bottom - top).min(line.1 - line.0);
                (overlap >= shorter * 0.5).then_some((overlap, line))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        match best {
            Some((_, line)) => line.2.push(w),
            None => lines.push((top, bottom, vec![w])),
        }
    }
    let lines = lines.into_iter().map(|(_, _, words)| words);

    let mut out: Vec<OcrLine> = lines
        .into_iter()
        .map(|mut ws| {
            ws.sort_by(|a, b| a.bbox.x0.total_cmp(&b.bbox.x0));
            let x0 = ws.iter().map(|w| w.bbox.x0).fold(f32::INFINITY, f32::min);
            let y0 = ws.iter().map(|w| w.bbox.y0).fold(f32::INFINITY, f32::min);
            let x1 = ws.iter().map(|w| w.bbox.right()).fold(f32::NEG_INFINITY, f32::max);
            let y1 = ws.iter().map(|w| w.bbox.bottom()).fold(f32::NEG_INFINITY, f32::max);
            let conf = ws.iter().map(|w| w.conf).sum::<f32>() / ws.len() as f32;
            OcrLine { words: ws, bbox: BBox::new(x0, y0, x1, y1), conf }
        })
        .collect();
    out.sort_by(|a, b| a.bbox.y0.total_cmp(&b.bbox.y0));
    OcrPage { lines: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn w(text: &str, x: f32, y: f32) -> OcrWord {
        let width = 0.05 * text.len() as f32 / 3.0;
        OcrWord { text: text.into(), bbox: BBox::new(x, y, x + width, y + 0.02), conf: 0.9 }
    }

    #[test]
    fn tightly_stacked_rows_stay_separate() {
        // Boxes that touch or overlap slightly, as on receipts printed with no line spacing.
        let boxed = |text: &str, x: f32, y: f32, h: f32| OcrWord {
            text: text.into(),
            bbox: BBox::new(x, y, x + 0.1, y + h),
            conf: 0.9,
        };
        let row = |label: &str, amount: &str, y: f32| {
            vec![boxed(label, 0.02, y, 0.018), boxed(amount, 0.5, y + 0.001, 0.017)]
        };
        let mut words = row("Subtotal", "$35.00", 0.189);
        words.extend(row("Tax", "$0.00", 0.204));
        words.extend(row("Total", "$35.00", 0.219));
        let page = group_lines(words);
        assert_eq!(page.text(), "Subtotal\t$35.00\nTax\t$0.00\nTotal\t$35.00");
    }

    #[test]
    fn groups_and_orders() {
        let page = group_lines(vec![w("Total:", 0.1, 0.50), w("Invoice", 0.1, 0.10), w("$10.00", 0.5, 0.501), w("#42", 0.23, 0.101)]);
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[0].text(), "Invoice #42");
        assert!(page.lines[1].text().starts_with("Total:"));
        assert!(page.lines[1].text().ends_with("$10.00"));
    }

    #[test]
    fn large_gap_becomes_tab() {
        let page = group_lines(vec![w("Widget", 0.05, 0.3), w("$5.00", 0.8, 0.3)]);
        assert!(page.lines[0].text().contains('\t'));
    }

    #[test]
    fn drops_blank_words_and_computes_confidence() {
        let mut a = w("a", 0.1, 0.1); a.conf = 0.5;
        let mut b = w("b", 0.3, 0.1); b.conf = 1.0;
        let page = group_lines(vec![a, b, w("  ", 0.5, 0.1)]);
        assert_eq!(page.lines[0].words.len(), 2);
        assert!((page.mean_confidence() - 0.75).abs() < 1e-6);
    }
}
