//! Turns OCR text into validated, confidence-scored fields: invoices and receipts, statements
//! (with their transactions), and the key terms of contracts.
//!
//! Flow: [`FieldExtractor`]s (rules today, a model tier later) propose fields; candidates are
//! calibrated and merged (`Human` always wins, agreeing tiers corroborate); [`validate`] checks
//! cross-field consistency; validation feeds back into confidence; low-confidence fields route
//! the document to review.

pub mod calibrate;
pub mod contract;
pub mod pipeline;
pub mod rules;
pub mod statement;
pub mod validate;

pub use calibrate::{adjust_with_validation, expected_calibration_error, needs_review, PlattScaler};
pub use pipeline::{merge, Extraction, FieldExtractor, Pipeline, RulesExtractor};
pub use contract::notice_deadline;
pub use validate::{
    required_fields, totals_consistent, validate, validate_for, Issue, Severity, CONTRACT_FIELDS, REQUIRED_FIELDS,
    STATEMENT_FIELDS,
};
