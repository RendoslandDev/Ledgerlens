use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use ulid::Ulid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            pub fn new() -> Self { Self(Ulid::new()) }
            pub fn from_ulid(u: Ulid) -> Self { Self(u) }
            pub fn as_ulid(&self) -> Ulid { self.0 }
        }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
        }
        impl FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> { Ok(Self(Ulid::from_string(s)?)) }
        }
    };
}

define_id!(DocumentId);
pub type DocId = DocumentId;
define_id!(PageId);
define_id!(RecordId);
define_id!(VendorId);
define_id!(TransactionId);
define_id!(JobId);
define_id!(DeviceId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_string() {
        let id = DocumentId::new();
        assert_eq!(id, id.to_string().parse().unwrap());
    }
    #[test]
    fn serde_is_a_plain_string() {
        let id = RecordId::new();
        let j = serde_json::to_string(&id).unwrap();
        assert_eq!(j, format!("\"{id}\""));
    }
}
