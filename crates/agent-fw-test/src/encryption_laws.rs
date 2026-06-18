//! EncryptionService algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1 (Roundtrip): `decrypt(encrypt(p)) == p` for all plaintext `p`
//! - L2 (Fresh-nonce): Two encryptions of the same plaintext produce different ciphertexts
//! - L3 (Tamper-detection): Flipping any ciphertext byte causes `DecryptFailed`
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_encryption_satisfies_laws() {
//!     let svc = MyEncryptionService::new();
//!     agent_fw_test::encryption_laws::test_all(&svc).await;
//! }
//! ```

use agent_fw_algebra::encryption::{EncryptedPayload, EncryptionError, EncryptionService};

/// Run all EncryptionService laws against the given implementation.
pub async fn test_all(svc: &dyn EncryptionService) {
    law_roundtrip(svc).await;
    law_fresh_nonce(svc).await;
    law_tamper_detection(svc).await;
}

/// L1: Roundtrip — `decrypt(encrypt(plaintext)) == plaintext` for all plaintext.
pub async fn law_roundtrip(svc: &dyn EncryptionService) {
    let plaintexts: &[&[u8]] = &[
        b"hello world",
        b"",
        b"\x00\x01\x02\xff",
        b"a longer plaintext with various characters: !@#$%^&*()",
    ];

    for plaintext in plaintexts {
        let encrypted = svc
            .encrypt(plaintext)
            .await
            .expect("L1 Roundtrip: encrypt must succeed");
        let decrypted = svc
            .decrypt(&encrypted)
            .await
            .expect("L1 Roundtrip: decrypt must succeed");
        assert_eq!(
            decrypted.as_slice(),
            *plaintext,
            "L1 Roundtrip: decrypt(encrypt(p)) must equal p"
        );
    }
}

/// L2: Fresh-nonce — two encryptions of the same plaintext produce different ciphertexts.
pub async fn law_fresh_nonce(svc: &dyn EncryptionService) {
    let plaintext = b"same plaintext for nonce test";

    let enc1 = svc
        .encrypt(plaintext)
        .await
        .expect("L2 Fresh-nonce: first encrypt must succeed");
    let enc2 = svc
        .encrypt(plaintext)
        .await
        .expect("L2 Fresh-nonce: second encrypt must succeed");

    // Either the nonce or the ciphertext (or both) must differ
    let same_nonce = enc1.nonce() == enc2.nonce();
    let same_ciphertext = enc1.ciphertext() == enc2.ciphertext();

    assert!(
        !same_nonce || !same_ciphertext,
        "L2 Fresh-nonce: two encryptions of the same plaintext must produce different ciphertexts \
         (nonces equal: {same_nonce}, ciphertexts equal: {same_ciphertext})"
    );
}

/// L3: Tamper-detection — flipping any ciphertext byte causes decrypt to fail.
pub async fn law_tamper_detection(svc: &dyn EncryptionService) {
    let plaintext = b"tamper detection test payload";

    let encrypted = svc
        .encrypt(plaintext)
        .await
        .expect("L3 Tamper-detection: encrypt must succeed");

    // Flip the first byte of the ciphertext
    let mut tampered_bytes = encrypted.ciphertext().to_vec();
    assert!(
        !tampered_bytes.is_empty(),
        "L3 Tamper-detection: ciphertext must not be empty"
    );
    tampered_bytes[0] ^= 0xFF;

    let tampered = EncryptedPayload::new(*encrypted.nonce(), tampered_bytes)
        .expect("L3 Tamper-detection: tampered payload construction must succeed");

    let result = svc.decrypt(&tampered).await;
    assert!(
        result.is_err(),
        "L3 Tamper-detection: decrypting tampered ciphertext must fail"
    );
    match result.unwrap_err() {
        EncryptionError::DecryptFailed(_) => {} // expected
        other => panic!(
            "L3 Tamper-detection: expected DecryptFailed, got {:?}",
            other
        ),
    }
}
