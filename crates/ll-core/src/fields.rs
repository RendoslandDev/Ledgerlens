use crate::document::DocKind;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

str_enum!(
    /// Backbone enum: classifier labels, evidence rows, review UI, corrections and exporters all use it.
    FieldName {
        Vendor => "vendor", InvoiceNumber => "invoice_number", DocDate => "doc_date",
        DueDate => "due_date", Currency => "currency", Subtotal => "subtotal",
        Tax => "tax", Total => "total", PaymentMethod => "payment_method", Category => "category",
        // Statements: the bank is `Vendor`, the statement date `DocDate`.
        AccountNumber => "account_number", PeriodStart => "period_start", PeriodEnd => "period_end",
        OpeningBalance => "opening_balance", ClosingBalance => "closing_balance",
        // Contracts: the start is `DocDate`, the contract value `Total`.
        Parties => "parties", EndDate => "end_date", NoticePeriod => "notice_period",
        AutoRenew => "auto_renew",
    }
);

impl FieldName {
    pub const ALL: [Self; 19] = [
        Self::Vendor, Self::InvoiceNumber, Self::DocDate, Self::DueDate, Self::Currency,
        Self::Subtotal, Self::Tax, Self::Total, Self::PaymentMethod, Self::Category,
        Self::AccountNumber, Self::PeriodStart, Self::PeriodEnd, Self::OpeningBalance,
        Self::ClosingBalance, Self::Parties, Self::EndDate, Self::NoticePeriod, Self::AutoRenew,
    ];

    #[allow(non_upper_case_globals)]
    pub const InvoiceDate: Self = Self::DocDate;

    /// Fields that must be right for a record to count as correct and skip review.
    /// Shared by the router and by ll-eval so "document-level success" means one thing.
    pub fn is_critical(self, kind: DocKind) -> bool {
        use FieldName::*;
        match kind {
            DocKind::Receipt => matches!(self, Vendor | DocDate | Total),
            DocKind::Invoice => matches!(self, Vendor | InvoiceNumber | DocDate | Total),
            DocKind::Statement => matches!(self, Vendor | PeriodEnd | ClosingBalance),
            DocKind::Contract => matches!(self, Parties | DocDate),
            DocKind::Other => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
    Text(String),
    Money(crate::money::Money),
    Date(crate::date::Date),
}

impl FieldValue {
    pub fn as_text(&self) -> Option<&str> {
        match self { Self::Text(value) => Some(value), _ => None }
    }

    pub fn as_money(&self) -> Option<crate::money::Money> {
        match self { Self::Money(value) => Some(*value), _ => None }
    }

    pub fn as_date(&self) -> Option<crate::date::Date> {
        match self { Self::Date(value) => Some(*value), _ => None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedField {
    pub name: FieldName,
    pub value: FieldValue,
    pub confidence: Confidence,
    pub tier: crate::document::Tier,
}

/// Probability-like score clamped to 0.0..=1.0 (NaN becomes 0.0).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Confidence(f32);

impl Confidence {
    pub const ZERO: Confidence = Confidence(0.0);
    pub const ONE: Confidence = Confidence(1.0);
    pub fn new(v: f32) -> Self { Self(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }) }
    pub fn get(self) -> f32 { self.0 }
    pub fn min(self, o: Self) -> Self { Self(self.0.min(o.0)) }
    pub fn penalize(self, factor: f32) -> Self { Self::new(self.0 * factor) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confidence_is_clamped_and_nan_safe() {
        assert_eq!(Confidence::new(1.7).get(), 1.0);
        assert_eq!(Confidence::new(-1.0).get(), 0.0);
        assert_eq!(Confidence::new(f32::NAN).get(), 0.0);
        assert_eq!(Confidence::new(0.8).penalize(0.5).get(), 0.4);
    }
    #[test]
    fn criticality_depends_on_kind() {
        assert!(FieldName::Total.is_critical(DocKind::Receipt));
        assert!(!FieldName::InvoiceNumber.is_critical(DocKind::Receipt));
        assert!(FieldName::InvoiceNumber.is_critical(DocKind::Invoice));
        assert!(FieldName::ClosingBalance.is_critical(DocKind::Statement));
        assert!(!FieldName::Total.is_critical(DocKind::Statement));
        assert!(FieldName::Parties.is_critical(DocKind::Contract));
    }
    #[test]
    fn field_names_roundtrip() {
        assert_eq!("doc_date".parse::<FieldName>().unwrap(), FieldName::DocDate);
        assert_eq!(FieldName::DueDate.as_str(), "due_date");
    }
}
