//! AES-256-GCM encryption service for credential storage.
//!
//! Uses a master key from config (64-char hex → 32 bytes).
//! Random 12-byte nonce per encrypt call via `rand::thread_rng()`.
//!
//! Satisfies EncryptionService laws:
//! - L1 (Roundtrip): `decrypt(encrypt(p)) == p`
//! - L2 (Fresh-nonce): Two encryptions produce different ciphertexts
//! - L3 (Tamper-detection): Flipped ciphertext byte → `DecryptFailed`

use agent_fw_algebra::{EncryptedPayload, EncryptionError, EncryptionService};
use async_trait::async_trait;

/// AES-256-GCM encryption service.
///
/// Master key must be 32 bytes (provided as 64-char hex string).
pub struct AesEncryptionService {
    key: Vec<u8>,
}

impl AesEncryptionService {
    /// Create from a 64-character hex string (32 bytes).
    pub fn from_hex_key(hex_key: &str) -> Result<Self, EncryptionError> {
        let key = hex::decode(hex_key)
            .map_err(|e| EncryptionError::InvalidKey(format!("Invalid hex key: {e}")))?;
        if key.len() != 32 {
            return Err(EncryptionError::InvalidKey(format!(
                "Key must be 32 bytes (64 hex chars), got {} bytes",
                key.len()
            )));
        }
        Ok(Self { key })
    }

    /// Create from raw 32-byte key.
    pub fn from_bytes(key: Vec<u8>) -> Result<Self, EncryptionError> {
        if key.len() != 32 {
            return Err(EncryptionError::InvalidKey(format!(
                "Key must be 32 bytes, got {} bytes",
                key.len()
            )));
        }
        Ok(Self { key })
    }
}

#[async_trait]
impl EncryptionService for AesEncryptionService {
    async fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, EncryptionError> {
        use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
        use rand::RngCore;

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| EncryptionError::EncryptFailed(e.to_string()))?;

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| EncryptionError::EncryptFailed(e.to_string()))?;

        EncryptedPayload::new(nonce_bytes, ciphertext)
    }

    async fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, EncryptionError> {
        use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| EncryptionError::DecryptFailed(e.to_string()))?;
        let nonce = Nonce::from_slice(payload.nonce());

        cipher
            .decrypt(nonce, payload.ciphertext())
            .map_err(|e| EncryptionError::DecryptFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[tokio::test]
    async fn roundtrip_works() {
        let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();

        let plaintext = b"hello world credentials";
        let encrypted = svc.encrypt(plaintext).await.unwrap();
        let decrypted = svc.decrypt(&encrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn rejects_bad_key_length() {
        let result = AesEncryptionService::from_hex_key("0123456789abcdef");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fresh_nonce_per_encrypt() {
        let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
        let plaintext = b"same input";

        let e1 = svc.encrypt(plaintext).await.unwrap();
        let e2 = svc.encrypt(plaintext).await.unwrap();

        assert_ne!(
            e1.nonce(),
            e2.nonce(),
            "Each encrypt must use a fresh nonce"
        );
    }

    #[tokio::test]
    async fn tamper_detection() {
        let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
        let plaintext = b"sensitive data";

        let encrypted = svc.encrypt(plaintext).await.unwrap();
        // Create tampered payload by flipping a byte
        let mut tampered_ct = encrypted.ciphertext().to_vec();
        if let Some(byte) = tampered_ct.first_mut() {
            *byte ^= 0xFF;
        }
        let tampered = EncryptedPayload::new(*encrypted.nonce(), tampered_ct).unwrap();

        let result = svc.decrypt(&tampered).await;
        assert!(result.is_err(), "Tampered ciphertext must fail to decrypt");
    }

    #[tokio::test]
    async fn large_payload_roundtrip() {
        let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
        let plaintext = vec![0xABu8; 10_000];

        let encrypted = svc.encrypt(&plaintext).await.unwrap();
        let decrypted = svc.decrypt(&encrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn empty_plaintext_roundtrip() {
        let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
        let plaintext = b"";

        let encrypted = svc.encrypt(plaintext).await.unwrap();
        let decrypted = svc.decrypt(&encrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn from_bytes_works() {
        let key = vec![0x42u8; 32];
        let svc = AesEncryptionService::from_bytes(key).unwrap();

        let plaintext = b"test data";
        let encrypted = svc.encrypt(plaintext).await.unwrap();
        let decrypted = svc.decrypt(&encrypted).await.unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[tokio::test]
    async fn from_bytes_rejects_wrong_length() {
        let key = vec![0x42u8; 16]; // Too short
        let result = AesEncryptionService::from_bytes(key);
        assert!(result.is_err());
    }

    // =========================================================================
    // Property-based algebraic laws (hegel)
    // =========================================================================

    use hegel::generators;

    /// Law 1: decrypt(encrypt(x)) == Ok(x) for arbitrary payloads.
    #[hegel::test]
    fn encryption_roundtrip_prop(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(generators::vecs(generators::integers::<u8>()).max_size(1023));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
            let encrypted = svc.encrypt(&data).await.unwrap();
            let decrypted = svc.decrypt(&encrypted).await.unwrap();
            assert_eq!(decrypted, data);
        });
    }

    /// Law 2: encrypt(x).nonce != encrypt(x).nonce for same input.
    #[hegel::test]
    fn encryption_fresh_nonce_prop(tc: hegel::TestCase) {
        let data: Vec<u8> = tc.draw(
            generators::vecs(generators::integers::<u8>())
                .min_size(1)
                .max_size(255),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let svc = AesEncryptionService::from_hex_key(TEST_KEY_HEX).unwrap();
            let e1 = svc.encrypt(&data).await.unwrap();
            let e2 = svc.encrypt(&data).await.unwrap();
            assert_ne!(
                e1.nonce(),
                e2.nonce(),
                "Each encrypt must use a fresh nonce"
            );
        });
    }
}
