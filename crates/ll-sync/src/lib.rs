#![forbid(unsafe_code)]

use chacha20poly1305::{
    Key, XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, OsRng, Payload},
};
use ll_core::{DeviceId, Error, Result};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

const ENVELOPE_VERSION: u8 = 1;
const AAD: &[u8] = b"ledgerlens-sync-v1";
const TABLES: &[&str] = &[
    "documents",
    "pages",
    "vendors",
    "records",
    "fields",
    "line_items",
    "field_evidence",
    "corrections",
    "bank_transactions",
    "matches",
    "chunks",
];

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SyncKey([u8; 32]);

impl SyncKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridTimestamp {
    pub physical_ms: u64,
    pub logical: u32,
    pub device_id: DeviceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Insert,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub sequence: u64,
    pub timestamp: HybridTimestamp,
    pub table_name: String,
    pub row_id: String,
    pub kind: OperationKind,
    pub patch: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub version: u8,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

pub fn encrypt_operation(key: &SyncKey, operation: &Operation) -> Result<EncryptedEnvelope> {
    validate_operation(operation)?;
    let plaintext =
        serde_json::to_vec(operation).map_err(|error| Error::Invalid(error.to_string()))?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    let mut nonce_bytes = [0u8; 24];
    let mut random = OsRng;
    random
        .try_fill_bytes(&mut nonce_bytes)
        .map_err(|error| Error::Integration(format!("OS randomness unavailable: {error}")))?;
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: &plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| Error::Integration("failed to encrypt sync operation".into()))?;
    Ok(EncryptedEnvelope {
        version: ENVELOPE_VERSION,
        nonce: nonce_bytes,
        ciphertext,
    })
}

pub fn decrypt_operation(key: &SyncKey, envelope: &EncryptedEnvelope) -> Result<Operation> {
    if envelope.version != ENVELOPE_VERSION {
        return Err(Error::Invalid(format!(
            "unsupported sync envelope version {}",
            envelope.version
        )));
    }
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&envelope.nonce),
            Payload {
                msg: &envelope.ciphertext,
                aad: AAD,
            },
        )
        .map_err(|_| Error::Integration("sync operation authentication failed".into()))?;
    let operation: Operation = serde_json::from_slice(&plaintext)
        .map_err(|_| Error::Invalid("invalid sync operation payload".into()))?;
    validate_operation(&operation)?;
    Ok(operation)
}

fn validate_operation(operation: &Operation) -> Result<()> {
    if !TABLES.contains(&operation.table_name.as_str()) {
        return Err(Error::Invalid(format!(
            "sync is not allowed for table {}",
            operation.table_name
        )));
    }
    if operation.row_id.trim().is_empty() {
        return Err(Error::Invalid("sync operation row_id is empty".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> Operation {
        let device_id = DeviceId::new();
        Operation {
            sequence: 7,
            timestamp: HybridTimestamp {
                physical_ms: 1_700_000_000_000,
                logical: 2,
                device_id,
            },
            table_name: "records".into(),
            row_id: "01H00000000000000000000000".into(),
            kind: OperationKind::Update,
            patch: serde_json::json!({"total_minor": 12345}),
        }
    }

    #[test]
    fn encrypted_envelope_roundtrips_and_hides_the_operation() {
        let key = SyncKey::from_bytes([7; 32]);
        let operation = operation();
        let envelope = encrypt_operation(&key, &operation).unwrap();
        assert!(!envelope.ciphertext.windows(6).any(|part| part == b"12345"));
        assert_eq!(decrypt_operation(&key, &envelope).unwrap(), operation);
    }

    #[test]
    fn authentication_rejects_tampering_and_wrong_keys() {
        let key = SyncKey::from_bytes([7; 32]);
        let mut envelope = encrypt_operation(&key, &operation()).unwrap();
        envelope.ciphertext[0] ^= 1;
        assert!(decrypt_operation(&key, &envelope).is_err());

        let envelope = encrypt_operation(&key, &operation()).unwrap();
        assert!(decrypt_operation(&SyncKey::from_bytes([8; 32]), &envelope).is_err());
    }

    #[test]
    fn unsupported_tables_and_versions_are_rejected() {
        let key = SyncKey::from_bytes([7; 32]);
        let mut invalid_operation = operation();
        invalid_operation.table_name = "sqlite_master".into();
        assert!(encrypt_operation(&key, &invalid_operation).is_err());

        let mut envelope = encrypt_operation(&key, &operation()).unwrap();
        envelope.version = 2;
        assert!(decrypt_operation(&key, &envelope).is_err());
    }
}
