//! OpenAI embedding provider.
//!
//! Uses the OpenAI API to generate embeddings via text-embedding-3-small.
//! Implements secure HTTP with reqwest, proper timeouts, and retry logic.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use super::{Embedding, EmbeddingProvider, OPENAI_EMBEDDING_DIM};
use crate::error::{CtxError, Result};

/// OpenAI API endpoint
const OPENAI_API_URL: &str = "https://api.openai.com/v1/embeddings";

/// Request timeout in seconds
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Connection timeout in seconds
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Maximum retry attempts for retryable errors
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (in milliseconds)
const RETRY_BASE_DELAY_MS: u64 = 1000;

/// Global runtime for sync API when not already in an async context.
/// This avoids creating a new runtime per provider instance.
static GLOBAL_RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn get_or_create_runtime() -> &'static Runtime {
    GLOBAL_RUNTIME.get_or_init(|| {
        Runtime::new().expect("Failed to create global tokio runtime for OpenAI provider")
    })
}

/// OpenAI embedding request body.
#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
    encoding_format: &'a str,
}

/// OpenAI API response.
#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Option<Vec<EmbeddingData>>,
    error: Option<ApiError>,
}

/// Individual embedding in the response.
#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
    #[allow(dead_code)]
    index: usize,
}

/// API error response.
#[derive(Deserialize)]
struct ApiError {
    message: String,
    #[allow(dead_code)]
    r#type: Option<String>,
    #[allow(dead_code)]
    code: Option<String>,
}

/// OpenAI embedding provider configuration.
pub struct OpenAIProvider {
    client: Client,
    model: String,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();

        // Build headers with authorization
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", api_key))
                .map_err(|e| CtxError::embedding(format!("Invalid API key format: {}", e)))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Build the HTTP client with timeouts
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .default_headers(headers)
            .build()
            .map_err(|e| CtxError::embedding(format!("Failed to build HTTP client: {}", e)))?;

        Ok(Self {
            client,
            model: "text-embedding-3-small".to_string(),
        })
    }

    /// Create a provider from the OPENAI_API_KEY environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| CtxError::InvalidApiKey)?;
        Self::new(api_key)
    }

    /// Set the model to use for embeddings.
    #[allow(dead_code)]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Make an HTTP request to the OpenAI API with retry logic (sync version).
    /// Uses a global runtime to avoid creating a new runtime per call.
    /// Safe to call from sync context; will panic if called from within an async runtime.
    fn request(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        // Check if we're already in an async context
        if tokio::runtime::Handle::try_current().is_ok() {
            // We're in an async context - this would deadlock with block_on
            // Return an error instead of panicking
            return Err(CtxError::embedding(
                "Cannot call sync embed() from async context. Use embed_async() instead.",
            ));
        }

        let runtime = get_or_create_runtime();
        runtime.block_on(self.request_async(texts))
    }

    /// Make an HTTP request to the OpenAI API with retry logic (async version).
    /// Safe to call from within an async runtime.
    pub async fn request_async(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let request_body = EmbeddingRequest {
            input: texts.to_vec(),
            model: &self.model,
            encoding_format: "float",
        };

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match self.send_request(&request_body).await {
                Ok(embeddings) => return Ok(embeddings),
                Err(e) => {
                    // Check if error is retryable
                    let should_retry = matches!(
                        &e,
                        CtxError::RateLimited(_) | CtxError::Embedding(_)
                    ) || matches!(&e, CtxError::Embedding(msg) if msg.contains("server error"));

                    if should_retry && attempt < MAX_RETRIES - 1 {
                        // Exponential backoff: 1s, 2s, 4s
                        let delay = RETRY_BASE_DELAY_MS * (1 << attempt);
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| CtxError::embedding("Max retries exceeded")))
    }

    /// Async version of embed for use in async contexts (e.g., MCP server).
    #[allow(dead_code)] // Public API for async contexts
    pub async fn embed_async(&self, text: &str) -> Result<Embedding> {
        let mut results = self.request_async(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| CtxError::embedding("Empty response"))
    }

    /// Async version of embed_batch for use in async contexts.
    #[allow(dead_code)] // Public API for async contexts
    pub async fn embed_batch_async(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        const BATCH_SIZE: usize = 100;

        let mut all_embeddings = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(BATCH_SIZE) {
            let embeddings = self.request_async(chunk).await?;
            all_embeddings.extend(embeddings);
        }

        Ok(all_embeddings)
    }

    /// Send a single request to the OpenAI API.
    async fn send_request(&self, request_body: &EmbeddingRequest<'_>) -> Result<Vec<Embedding>> {
        let response = self
            .client
            .post(OPENAI_API_URL)
            .json(request_body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    CtxError::embedding(format!("Request timed out: {}", e))
                } else if e.is_connect() {
                    CtxError::embedding(format!("Connection failed: {}", e))
                } else {
                    CtxError::embedding(e.to_string())
                }
            })?;

        let status = response.status();

        // Handle HTTP status codes
        match status {
            StatusCode::OK => {
                // Parse successful response
                let api_response: EmbeddingResponse = response
                    .json()
                    .await
                    .map_err(|e| CtxError::embedding(format!("Failed to parse response: {}", e)))?;

                self.parse_response(api_response)
            }
            StatusCode::TOO_MANY_REQUESTS => {
                // Rate limited - extract retry-after if available
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60);
                Err(CtxError::RateLimited(retry_after))
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(CtxError::InvalidApiKey),
            s if s.is_server_error() => {
                // 5xx errors - retryable
                let body = response.text().await.unwrap_or_default();
                Err(CtxError::embedding(format!(
                    "server error ({}): {}",
                    status, body
                )))
            }
            _ => {
                // Other client errors
                let body = response.text().await.unwrap_or_default();

                // Try to parse as API error
                if let Ok(error_response) = serde_json::from_str::<EmbeddingResponse>(&body) {
                    if let Some(error) = error_response.error {
                        return Err(CtxError::embedding(error.message));
                    }
                }

                Err(CtxError::embedding(format!("HTTP {}: {}", status, body)))
            }
        }
    }

    /// Parse the OpenAI API response.
    fn parse_response(&self, response: EmbeddingResponse) -> Result<Vec<Embedding>> {
        // Check for API errors
        if let Some(error) = response.error {
            if error.message.contains("rate limit") {
                return Err(CtxError::RateLimited(60));
            }
            if error.message.contains("invalid api key")
                || error.message.contains("Incorrect API key")
            {
                return Err(CtxError::InvalidApiKey);
            }
            return Err(CtxError::embedding(error.message));
        }

        // Extract embeddings
        let data = response
            .data
            .ok_or_else(|| CtxError::embedding("No data in response"))?;

        let mut embeddings = Vec::with_capacity(data.len());

        for item in data {
            let vector = item.embedding;

            if vector.len() != OPENAI_EMBEDDING_DIM {
                return Err(CtxError::DimensionMismatch {
                    expected: OPENAI_EMBEDDING_DIM,
                    actual: vector.len(),
                });
            }

            embeddings.push(Embedding::new(vector));
        }

        Ok(embeddings)
    }
}

impl EmbeddingProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn dimension(&self) -> usize {
        OPENAI_EMBEDDING_DIM
    }

    fn embed(&self, text: &str) -> Result<Embedding> {
        let mut results = self.request(&[text])?;
        results
            .pop()
            .ok_or_else(|| CtxError::embedding("Empty response"))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        // OpenAI supports batch embedding up to ~8000 tokens
        // For simplicity, we'll batch in chunks of 100
        const BATCH_SIZE: usize = 100;

        let mut all_embeddings = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(BATCH_SIZE) {
            let embeddings = self.request(chunk)?;
            all_embeddings.extend(embeddings);
        }

        Ok(all_embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_creation() {
        // Note: This will fail without a valid API key format, but tests the builder
        let result = OpenAIProvider::new("test-key-12345");
        assert!(result.is_ok());

        let provider = result.unwrap();
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.dimension(), OPENAI_EMBEDDING_DIM);
    }

    #[test]
    fn test_parse_response_success() {
        let provider = OpenAIProvider::new("test-key").unwrap();

        let response = EmbeddingResponse {
            data: Some(vec![EmbeddingData {
                embedding: vec![0.0; OPENAI_EMBEDDING_DIM],
                index: 0,
            }]),
            error: None,
        };

        let result = provider.parse_response(response);
        assert!(result.is_ok());
        let embeddings = result.unwrap();
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].vector.len(), OPENAI_EMBEDDING_DIM);
    }

    #[test]
    fn test_parse_response_error() {
        let provider = OpenAIProvider::new("test-key").unwrap();

        let response = EmbeddingResponse {
            data: None,
            error: Some(ApiError {
                message: "Incorrect API key provided".to_string(),
                r#type: None,
                code: None,
            }),
        };

        let result = provider.parse_response(response);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CtxError::InvalidApiKey));
    }

    #[test]
    fn test_parse_response_rate_limited() {
        let provider = OpenAIProvider::new("test-key").unwrap();

        let response = EmbeddingResponse {
            data: None,
            error: Some(ApiError {
                message: "rate limit exceeded".to_string(),
                r#type: None,
                code: None,
            }),
        };

        let result = provider.parse_response(response);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CtxError::RateLimited(_)));
    }

    #[test]
    fn test_parse_response_dimension_mismatch() {
        let provider = OpenAIProvider::new("test-key").unwrap();

        let response = EmbeddingResponse {
            data: Some(vec![EmbeddingData {
                embedding: vec![0.0; 100], // Wrong dimension
                index: 0,
            }]),
            error: None,
        };

        let result = provider.parse_response(response);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CtxError::DimensionMismatch {
                expected: 1536,
                actual: 100
            }
        ));
    }

    // Integration test - requires OPENAI_API_KEY
    #[test]
    #[ignore]
    fn test_embed_real() {
        let provider = OpenAIProvider::from_env().expect("OPENAI_API_KEY not set");
        let embedding = provider.embed("Hello, world!").expect("Embedding failed");
        assert_eq!(embedding.dim(), OPENAI_EMBEDDING_DIM);
    }

    #[test]
    #[ignore]
    fn test_embed_batch_real() {
        let provider = OpenAIProvider::from_env().expect("OPENAI_API_KEY not set");
        let texts = vec!["Hello", "World", "Test"];
        let embeddings = provider
            .embed_batch(&texts)
            .expect("Batch embedding failed");
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.dim(), OPENAI_EMBEDDING_DIM);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_embed_async_real() {
        let provider = OpenAIProvider::from_env().expect("OPENAI_API_KEY not set");
        let embedding = provider
            .embed_async("Hello, world!")
            .await
            .expect("Async embedding failed");
        assert_eq!(embedding.dim(), OPENAI_EMBEDDING_DIM);
    }
}
