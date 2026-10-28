//! Turns page images into structured text: engine trait, CTC decoding, and line/column layout.

pub mod ctc;
pub mod engine;
pub mod layout;

pub use ctc::ctc_greedy_decode;
pub use engine::{recognize_page, OcrEngine, ScriptedEngine};
pub use layout::{group_lines, OcrLine, OcrPage, OcrWord};
