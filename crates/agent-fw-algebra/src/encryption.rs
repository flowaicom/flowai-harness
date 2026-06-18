//! Encryption service algebra.
//!
//! # Laws
//!
//! - **L1 Roundtrip**: `decrypt(encrypt(p)) == p` for all plaintext `p`
//! - **L2 Fresh-nonce**: Two encryptions of the same plaintext produce different ciphertexts
//! - **L3 Tamper-detection**: Flipping any ciphertext byte causes `DecryptFailed`

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Errors from encryption operations.
#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("encryption failed: {0}")]
    EncryptFailed(String),
    #[error("decryption failed: {0}")]
    DecryptFailed(String),
    #[error("invalid key: {0}")]
    InvalidKey(String),
}

/// Encrypted payload with smart constructor enforcing invariants.
///
/// # Invariants
/// - `nonce` is exactly 12 bytes (96-bit, standard for AES-GCM)
/// - `ciphertext` is non-empty
#[derive(Debug, Clone)]
pub struct EncryptedPayload {
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

impl EncryptedPayload {
    /// Smart constructor: enforces 12-byte nonce and non-empty ciphertext.
    pub fn new(nonce: [u8; 12], ciphertext: Vec<u8>) -> Result<Self, EncryptionError> {
        if ciphertext.is_empty() {
            return Err(EncryptionError::EncryptFailed(
                "ciphertext must not be empty".into(),
            ));
        }
        Ok(Self { nonce, ciphertext })
    }

    /// Access the nonce.
    pub fn nonce(&self) -> &[u8; 12] {
        &self.nonce
    }

    /// Access the ciphertext.
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
}

impl Serialize for EncryptedPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            nonce: &'a [u8; 12],
            ciphertext: &'a [u8],
        }

        Wire {
            nonce: &self.nonce,
            ciphertext: &self.ciphertext,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EncryptedPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            nonce: [u8; 12],
            ciphertext: Vec<u8>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.nonce, wire.ciphertext).map_err(serde::de::Error::custom)
    }
}

/// Async encryption service (object-safe).
///
/// Implementations must satisfy laws L1-L3 documented at the module level.
#[async_trait]
pub trait EncryptionService: Send + Sync {
    /// Encrypt plaintext bytes into an encrypted payload.
    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, EncryptionError>;

    /// Decrypt an encrypted payload back to plaintext bytes.
    async fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, EncryptionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_payload_rejects_empty_ciphertext() {
        let result = EncryptedPayload::new([0u8; 12], vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn encrypted_payload_accepts_valid() {
        let result = EncryptedPayload::new([1u8; 12], vec![42, 43, 44]);
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.nonce(), &[1u8; 12]);
        assert_eq!(p.ciphertext(), &[42, 43, 44]);
    }

    #[test]
    fn encrypted_payload_serde_roundtrip() {
        let payload = EncryptedPayload::new([7u8; 12], vec![1, 2, 3]).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        let roundtrip: EncryptedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.nonce(), &[7u8; 12]);
        assert_eq!(roundtrip.ciphertext(), &[1, 2, 3]);
    }

    #[test]
    fn encrypted_payload_deserialize_rejects_empty_ciphertext() {
        let result: Result<EncryptedPayload, _> =
            serde_json::from_str(r#"{"nonce":[0,0,0,0,0,0,0,0,0,0,0,0],"ciphertext":[]}"#);
        assert!(result.is_err());
    }
}
