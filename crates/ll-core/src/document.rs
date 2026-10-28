use serde::{Deserialize, Serialize};
use std::str::FromStr;

str_enum!(DocKind { Receipt => "receipt", Invoice => "invoice", Contract => "contract", Statement => "statement", Other => "other" });
str_enum!(DocStatus { Queued => "queued", Processing => "processing", NeedsReview => "needs_review", Extracted => "extracted", Approved => "approved", Done => "done", Failed => "failed" });
str_enum!(Tier { Fast => "fast", Vlm => "vlm", Rules => "rules", Model => "model", Human => "human" });

impl DocStatus {
    pub fn is_terminal(self) -> bool { matches!(self, Self::Extracted | Self::Approved | Self::Done | Self::Failed) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn as_str_from_str_agree_with_serde() {
        for k in [DocKind::Receipt, DocKind::Invoice, DocKind::Contract, DocKind::Statement, DocKind::Other] {
            assert_eq!(k.as_str().parse::<DocKind>().unwrap(), k);
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{}\"", k.as_str()));
        }
        assert!("nope".parse::<DocStatus>().is_err());
    }
    #[test]
    fn terminal_states() {
        assert!(DocStatus::Done.is_terminal());
        assert!(!DocStatus::NeedsReview.is_terminal());
    }
}
