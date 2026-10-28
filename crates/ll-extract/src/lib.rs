//! Turns OCR text into validated, confidence-scored invoice fields.
//!
//! Flow: [`FieldExtractor`]s (rules today, a model tier later) propose fields; candidates are
//! calibrated and merged (`Human` always wins, agreeing tiers corroborate); [`validate`] checks
//! cross-field consistency; validation feeds back into confidence; low-confidence fields route
//! the document to review.

pub mod calibrate;
pub mod pipeline;
pub mod rules;
pub mod validate;

pub use calibrate::{adjust_with_validation, expected_calibration_error, needs_review, PlattScaler};
pub use pipeline::{merge, Extraction, FieldExtractor, Pipeline, RulesExtractor};
pub use validate::{totals_consistent, validate, Issue, Severity, REQUIRED_FIELDS};
