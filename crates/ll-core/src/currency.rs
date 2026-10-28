use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency { USD, EUR, GBP, GHS, NGN, KES, ZAR, CAD, AUD, JPY, INR, CNY, IDR }

impl Currency {
    pub const ALL: [Currency; 13] = [
        Currency::USD, Currency::EUR, Currency::GBP, Currency::GHS, Currency::NGN, Currency::KES,
        Currency::ZAR, Currency::CAD, Currency::AUD, Currency::JPY, Currency::INR, Currency::CNY,
        Currency::IDR,
    ];

    /// Number of minor-unit decimal places (JPY = 0, most others = 2). IDR follows ISO 4217 (2)
    /// even though receipts print whole rupiah, so exports stay standard.
    pub const fn exponent(self) -> u8 {
        match self { Currency::JPY => 0, _ => 2 }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Currency::USD => "USD", Currency::EUR => "EUR", Currency::GBP => "GBP",
            Currency::GHS => "GHS", Currency::NGN => "NGN", Currency::KES => "KES",
            Currency::ZAR => "ZAR", Currency::CAD => "CAD", Currency::AUD => "AUD",
            Currency::JPY => "JPY", Currency::INR => "INR", Currency::CNY => "CNY",
            Currency::IDR => "IDR",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        let up = s.trim().to_ascii_uppercase();
        Self::ALL.into_iter().find(|c| c.code() == up)
    }

    pub fn new(s: &str) -> crate::error::Result<Self> {
        Self::from_code(s).ok_or_else(|| crate::error::Error::Invalid(format!("unknown currency code: {s}")))
    }

    /// Symbols are ambiguous ("$" is USD/CAD/AUD), so return every candidate.
    pub fn from_symbol(s: &str) -> &'static [Currency] {
        match s.trim() {
            "$" => &[Currency::USD, Currency::CAD, Currency::AUD],
            "€" => &[Currency::EUR],
            "£" => &[Currency::GBP],
            "GH₵" | "₵" => &[Currency::GHS],
            "₦" => &[Currency::NGN],
            "R" => &[Currency::ZAR],
            "¥" => &[Currency::JPY, Currency::CNY],
            "₹" => &[Currency::INR],
            "Rp" | "Rp." => &[Currency::IDR],
            _ => &[],
        }
    }
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.code()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_roundtrip() {
        for c in Currency::ALL { assert_eq!(Currency::from_code(c.code()), Some(c)); }
        assert_eq!(Currency::from_code(" usd "), Some(Currency::USD));
        assert_eq!(Currency::from_code("XXX"), None);
    }
    #[test]
    fn dollar_sign_is_ambiguous() { assert!(Currency::from_symbol("$").len() > 1); }
}
