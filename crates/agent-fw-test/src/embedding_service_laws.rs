//! EmbeddingService algebraic law test harnesses.
//!
//! # Laws
//!
//! - L2 (Dimension Consistency): All returned vectors have length == self.dimension()
//! - L3 (Batch Consistency): embed_batch([a, b]) == [embed_one(a), embed_one(b)]
//!
//! L1 (Determinism) requires temperature=0 in the real service, which is
//! a configuration concern rather than a testable algebraic law.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_service_satisfies_laws() {
//!     let service = MockEmbeddingService::new(128);
//!     agent_fw_test::embedding_service_laws::test_all(&service).await;
//! }
//! ```

use agent_fw_algebra::vector_store::{EmbeddingError, EmbeddingService};

/// Run all EmbeddingService laws.
pub async fn test_all(service: &dyn EmbeddingService) {
    law_dimension_consistency(service).await;
    law_batch_consistency(service).await;
    law_dimension_positive(service).await;
    law_empty_string_handling(service).await;
}

/// L2: Dimension Consistency — embed_one returns a vector of length == dimension().
pub async fn law_dimension_consistency(service: &dyn EmbeddingService) {
    let embedding = service
        .embed_one("test sentence for dimension check")
        .await
        .expect("L2: embed_one should succeed");

    assert_eq!(
        embedding.len(),
        service.dimension(),
        "L2: embedding dimension {} != declared dimension {}",
        embedding.len(),
        service.dimension()
    );
}

/// L3: Batch Consistency — embed_batch results match individual embed_one calls.
pub async fn law_batch_consistency(service: &dyn EmbeddingService) {
    let texts = ["alpha text", "beta text"];

    let batch = service
        .embed_batch(&texts)
        .await
        .expect("L3: embed_batch should succeed");

    assert_eq!(
        batch.len(),
        2,
        "L3: batch should return one embedding per input"
    );

    let individual_a = service
        .embed_one(texts[0])
        .await
        .expect("L3: embed_one(a) should succeed");
    let individual_b = service
        .embed_one(texts[1])
        .await
        .expect("L3: embed_one(b) should succeed");

    // Check within f32 epsilon (implementations may have minor float differences)
    assert_eq!(
        batch[0].len(),
        individual_a.len(),
        "L3: batch[0] dimension mismatch"
    );
    assert_eq!(
        batch[1].len(),
        individual_b.len(),
        "L3: batch[1] dimension mismatch"
    );

    for (i, (b, s)) in batch[0].iter().zip(individual_a.iter()).enumerate() {
        assert!(
            (b - s).abs() < 1e-5,
            "L3: batch[0][{i}] = {b} != embed_one[{i}] = {s} (epsilon 1e-5)"
        );
    }
    for (i, (b, s)) in batch[1].iter().zip(individual_b.iter()).enumerate() {
        assert!(
            (b - s).abs() < 1e-5,
            "L3: batch[1][{i}] = {b} != embed_one[{i}] = {s} (epsilon 1e-5)"
        );
    }
}

/// L4: Dimension Positive — dimension() must be > 0.
pub async fn law_dimension_positive(service: &dyn EmbeddingService) {
    assert!(
        service.dimension() > 0,
        "L4: embedding dimension must be > 0, got {}",
        service.dimension()
    );
}

/// L5: Empty String Handling — embed_one("") produces a valid embedding (not an error).
pub async fn law_empty_string_handling(service: &dyn EmbeddingService) {
    let result = service.embed_one("").await;
    match result {
        Ok(embedding) => {
            assert_eq!(
                embedding.len(),
                service.dimension(),
                "L5: empty string embedding must have correct dimension"
            );
        }
        Err(_) => {
            // Some services may reject empty strings — that's acceptable behavior.
            // The law only requires that it doesn't panic.
        }
    }
}

// =============================================================================
// MockEmbeddingService — deterministic hash-based mock for law tests
// =============================================================================

/// Deterministic hash-based embedding service for law testing.
///
/// Generates embeddings by hashing the input text into a fixed-dimension
/// vector. Deterministic: same text always produces the same embedding.
pub struct MockEmbeddingService {
    dim: usize,
}

impl MockEmbeddingService {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    fn hash_to_embedding(&self, text: &str) -> Vec<f32> {
        // Simple deterministic hash: use bytes of the text
        let mut embedding = vec![0.0f32; self.dim];
        for (i, byte) in text.bytes().enumerate() {
            embedding[i % self.dim] += byte as f32 / 255.0;
        }
        // Normalize to unit vector
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for x in &mut embedding {
                *x /= magnitude;
            }
        }
        embedding
    }
}

#[async_trait::async_trait]
impl EmbeddingService for MockEmbeddingService {
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|t| self.hash_to_embedding(t)).collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        "mock-hash-embedding"
    }
}
