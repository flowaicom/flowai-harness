//! No-op encryption for development and tests.
//!
//! Stores plaintext bytes as the encrypted payload ciphertext with a dummy nonce.
//! This keeps the persistence contract honest without pretending to provide
//! confidentiality.

use async_trait::async_trait;

use agent_fw_algebra::{EncryptedPayload, EncryptionError, EncryptionService};

/// Development-only encryption interpreter that stores plaintext verbatim.
pub struct NoOpEncryptionService;

impl NoOpEncryptionService {
    pub fn new() -> Self {
        tracing::warn!(
            "Using NoOpEncryptionService — secrets are stored in plaintext. Do NOT use in production."
        );
        Self
    }
}

impl Default for NoOpEncryptionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EncryptionService for NoOpEncryptionService {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, EncryptionError> {
        EncryptedPayload::new([0u8; 12], plaintext.to_vec())
    }

    async fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, EncryptionError> {
        Ok(payload.ciphertext().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrips_plaintext() {
        let svc = NoOpEncryptionService::new();
        let encrypted = svc
            .encrypt(br#"{"username":"u","password":"p"}"#)
            .await
            .unwrap();
        let decrypted = svc.decrypt(&encrypted).await.unwrap();
        assert_eq!(decrypted, br#"{"username":"u","password":"p"}"#);
    }

    #[tokio::test]
    async fn produced_payload_is_serializable() {
        let svc = NoOpEncryptionService::new();
        let encrypted = svc.encrypt(b"secret").await.unwrap();
        let json = serde_json::to_string(&encrypted).unwrap();
        let roundtrip: EncryptedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.ciphertext(), b"secret");
        assert_eq!(roundtrip.nonce(), &[0u8; 12]);
    }
}
