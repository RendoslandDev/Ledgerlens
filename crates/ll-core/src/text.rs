use crate::{fields::Confidence, geometry::BBox};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word { pub text: String, pub bbox: BBox, pub confidence: Confidence }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line { pub words: Vec<Word>, pub bbox: BBox }

/// Where the words came from: an embedded PDF text layer, or OCR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSource { Native, Ocr }

/// Handoff type between rendering/OCR and extraction.
/// OCR - Optical Character Recognition
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    pub page_no: u32,
    pub width: u32,
    pub height: u32,
    pub lines: Vec<Line>,
    pub source: TextSource,
}

impl PageText {
    pub fn plain_text(&self) -> String {
        self.lines.iter()
            .map(|l| l.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_text_joins_words_and_lines() {
        let w = |t: &str| Word {
             text: t.into(),
             bbox: BBox::new(0.0, 0.0, 1.0, 1.0), 
             confidence: Confidence::new(0.9) 
            };
        let line = |ws: Vec<Word>| Line { 
            bbox: BBox::new(0.0, 0.0, 1.0, 1.0),
             words: ws 
        };
        let p = PageText { 
            page_no: 1,
            width: 10,
            height: 10,
            source: TextSource::Ocr,
            lines: vec![line(vec![w("Total"), w("12.00")]), 
                        line(vec![w("Thanks")])
                        ] 
        };
        assert_eq!(p.plain_text(), "Total 12.00\nThanks");
    }
}
