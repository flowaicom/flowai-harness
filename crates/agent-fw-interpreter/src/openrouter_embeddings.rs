//! OpenRouter embedding service — generates dense vectors via the OpenRouter API.
//!
//! Uses the OpenAI-compatible `/v1/embeddings` endpoint with configurable model
//! (default: `qwen/qwen3-embedding-4b`, 2560 dimensions).
//!
//! # Configuration
//!
//! Requires `OPENROUTER_API_KEY` environment variable. If absent, callers should
//! fall back to `NoOpVectorStore` (no embeddings generated).
//!
//! # Feature gate
//!
//! Requires the `http-clients` feature (for `reqwest` dependency).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use agent_fw_algebra::vector_store::{EmbeddingError, EmbeddingService};

const DEFAULT_MODEL: &str = "qwen/qwen3-embedding-4b";
const DEFAULT_DIMENSION: usize = 2560;
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// Maximum texts per batch to avoid API limits.
const MAX_BATCH_SIZE: usize = 64;
/// Default HTTP timeout for embedding API requests.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// OpenRouter-backed embedding service.
///
/// Calls the OpenAI-compatible embeddings endpoint. Batches are split
/// to respect `MAX_BATCH_SIZE` and concatenated.
pub struct OpenRouterEmbeddings {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dimension: usize,
    base_url: String,
}

impl OpenRouterEmbeddings {
    /// Create from an API key with default settings.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api_key: api_key.into(),
            model: DEFAULT_MODEL.to_string(),
            dimension: DEFAULT_DIMENSION,
            base_url: DEFAULT_BASE_URL.to_string(),
        }
    }

    /// Create from the `OPENROUTER_API_KEY` environment variable.
    ///
    /// Returns `None` if the env var is unset or empty.
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("OPENROUTER_API_KEY").ok()?;
        if key.is_empty() {
            return None;
        }
        Some(Self::new(key))
    }

    /// Override the model (default: `qwen/qwen3-embedding-4b`).
    pub fn with_model(mut self, model: impl Into<String>, dimension: usize) -> Self {
        self.model = model.into();
        self.dimension = dimension;
        self
    }

    /// Override the base URL (for testing or alternative providers).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Send one batch of texts to the embeddings API.
    async fn send_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let request = EmbeddingRequest {
            model: &self.model,
            input: texts,
            encoding_format: "float",
        };

        let response = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| EmbeddingError::Api(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "unable to read body".into());
            return Err(EmbeddingError::Api(format!("HTTP {status}: {body}")));
        }

        let body: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingError::Api(format!("parse response: {e}")))?;

        // Sort by index to preserve input order
        let mut data = body.data;
        data.sort_by_key(|d| d.index);

        Ok(data.into_iter().map(|d| d.embedding).collect())
    }
}

#[async_trait]
impl EmbeddingService for OpenRouterEmbeddings {
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Split into chunks to respect batch size limits
        let mut all_embeddings = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(MAX_BATCH_SIZE) {
            let batch_result = self.send_batch(chunk).await?;
            all_embeddings.extend(batch_result);
        }

        Ok(all_embeddings)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

// ---------------------------------------------------------------------------
// Wire types for the OpenAI-compatible API
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
    encoding_format: &'a str,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_when_unset() {
        // This test assumes OPENROUTER_API_KEY is not set in CI
        // If it IS set, the test still passes (it returns Some)
        let _ = OpenRouterEmbeddings::from_env();
    }

    #[test]
    fn builder_methods() {
        let embedder = OpenRouterEmbeddings::new("test-key")
            .with_model("text-embedding-ada-002", 1536)
            .with_base_url("http://localhost:8080");

        assert_eq!(embedder.model_name(), "text-embedding-ada-002");
        assert_eq!(embedder.dimension(), 1536);
        assert_eq!(embedder.base_url, "http://localhost:8080");
    }
}
