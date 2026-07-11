//! Ollama embedding provider.
//!
//! Uses a local (or remote) [Ollama](https://ollama.com) server to generate
//! embeddings via its `/api/embed` endpoint. This gives high-quality embeddings
//! that run fully offline, without the fastembed model-download constraints and
//! without OpenAI's per-call cost.
//!
//! Unlike OpenAI/fastembed, the embedding dimension is model-dependent
//! (`nomic-embed-text` = 768, `mxbai-embed-large` = 1024, `qwen3-embedding:8b`
//! = 4096, …), so it is probed from the model on construction rather than being
//! a compile-time constant.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use super::{Embedding, EmbeddingProvider};
use crate::error::{CtxError, Result};

/// Default Ollama host when `OLLAMA_HOST` is unset.
const DEFAULT_HOST: &str = "http://localhost:11434";

/// Default embedding model when `OLLAMA_EMBED_MODEL` is unset.
const DEFAULT_MODEL: &str = "nomic-embed-text";

const REQUEST_TIMEOUT_SECS: u64 = 60;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_DELAY_MS: u64 = 500;

/// Global runtime for the sync API when not already in an async context.
static GLOBAL_RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn get_or_create_runtime() -> &'static Runtime {
    GLOBAL_RUNTIME.get_or_init(|| {
        Runtime::new().expect("Failed to create global tokio runtime for Ollama provider")
    })
}

/// Ollama `/api/embed` request body. `input` accepts one or many texts.
#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

/// Ollama `/api/embed` response body.
#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Option<Vec<Vec<f32>>>,
    error: Option<String>,
}

/// Ollama embedding provider.
pub struct OllamaProvider {
    client: Client,
    host: String,
    model: String,
    /// Embedding dimension, probed from the model at construction.
    dimension: usize,
}

/// Read the configured host from `OLLAMA_HOST`, normalizing a bare `host:port`
/// (Ollama's own convention) into a URL.
fn configured_host() -> String {
    match std::env::var("OLLAMA_HOST") {
        Ok(h) if !h.is_empty() => {
            if h.starts_with("http://") || h.starts_with("https://") {
                h
            } else {
                format!("http://{}", h)
            }
        }
        _ => DEFAULT_HOST.to_string(),
    }
}

/// Read the configured model from `OLLAMA_EMBED_MODEL`, else the default.
fn configured_model() -> String {
    std::env::var("OLLAMA_EMBED_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

impl OllamaProvider {
    /// Create a provider from the environment (`OLLAMA_HOST`, `OLLAMA_EMBED_MODEL`,
    /// optional `OLLAMA_API_KEY` bearer token), probing the model's dimension
    /// synchronously. Use [`OllamaProvider::from_env_async`] from async contexts.
    pub fn from_env() -> Result<Self> {
        Self::with_model(configured_model())
    }

    /// Create a provider for a specific model, probing its embedding dimension
    /// (synchronous — do not call from within an async runtime).
    pub fn with_model(model: impl Into<String>) -> Result<Self> {
        let mut provider = Self::new_unprobed(model)?;
        let probe = provider.request(&["dimension probe"])?;
        provider.dimension = Self::dimension_from_probe(&provider.model, probe)?;
        Ok(provider)
    }

    /// Async constructor for use inside an async runtime (e.g. the MCP server),
    /// where the synchronous probe would deadlock.
    pub async fn from_env_async() -> Result<Self> {
        let mut provider = Self::new_unprobed(configured_model())?;
        let probe = provider.request_async(&["dimension probe"]).await?;
        provider.dimension = Self::dimension_from_probe(&provider.model, probe)?;
        Ok(provider)
    }

    /// Build the client/config without probing the dimension (left as 0).
    fn new_unprobed(model: impl Into<String>) -> Result<Self> {
        let model = model.into();
        let host = configured_host();

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        // Optional bearer token for authenticated / remote Ollama hosts.
        if let Ok(token) = std::env::var("OLLAMA_API_KEY") {
            if !token.is_empty() {
                let value = HeaderValue::from_str(&format!("Bearer {}", token)).map_err(|e| {
                    CtxError::embedding(format!("Invalid OLLAMA_API_KEY format: {}", e))
                })?;
                headers.insert(AUTHORIZATION, value);
            }
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .default_headers(headers)
            .build()
            .map_err(|e| CtxError::embedding(format!("Failed to build HTTP client: {}", e)))?;

        Ok(Self {
            client,
            host,
            model,
            dimension: 0,
        })
    }

    fn dimension_from_probe(model: &str, probe: Vec<Embedding>) -> Result<usize> {
        probe
            .first()
            .map(|e| e.vector.len())
            .filter(|d| *d > 0)
            .ok_or_else(|| {
                CtxError::embedding(format!(
                    "Ollama model '{}' returned no embedding on probe",
                    model
                ))
            })
    }

    /// Async single-text embedding for use inside an async runtime.
    pub async fn embed_async(&self, text: &str) -> Result<Embedding> {
        self.request_async(&[text])
            .await?
            .pop()
            .ok_or_else(|| CtxError::embedding("Empty response"))
    }

    /// The endpoint URL for embeddings.
    fn embed_url(&self) -> String {
        format!("{}/api/embed", self.host.trim_end_matches('/'))
    }

    /// Synchronous request with retry. Errors (rather than deadlocking) if called
    /// from within an async runtime.
    fn request(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(CtxError::embedding(
                "Cannot call sync embed() from async context. Use request_async() instead.",
            ));
        }
        get_or_create_runtime().block_on(self.request_async(texts))
    }

    /// Async request with retry/backoff for transient failures.
    pub async fn request_async(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let body = OllamaEmbedRequest {
            model: &self.model,
            input: texts.to_vec(),
        };

        let mut last_error = None;
        for attempt in 0..MAX_RETRIES {
            match self.send_request(&body).await {
                Ok(embeddings) => return Ok(embeddings),
                Err(e) => {
                    // Retry transient connection / server errors, not "model not
                    // found" or malformed input.
                    let retryable = matches!(&e, CtxError::Embedding(msg)
                        if msg.contains("server error")
                            || msg.contains("timed out")
                            || msg.contains("Connection"));
                    if retryable && attempt < MAX_RETRIES - 1 {
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

    /// Send a single `/api/embed` request and map the outcome.
    async fn send_request(&self, body: &OllamaEmbedRequest<'_>) -> Result<Vec<Embedding>> {
        let response = self
            .client
            .post(self.embed_url())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    CtxError::embedding(format!("Request timed out: {}", e))
                } else if e.is_connect() {
                    CtxError::embedding(format!(
                        "Connection to Ollama at {} failed: {}. Is `ollama serve` running?",
                        self.host, e
                    ))
                } else {
                    CtxError::embedding(e.to_string())
                }
            })?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let parsed: OllamaEmbedResponse = response
                    .json()
                    .await
                    .map_err(|e| CtxError::embedding(format!("Failed to parse response: {}", e)))?;
                self.parse_response(parsed)
            }
            StatusCode::NOT_FOUND => {
                // Model not pulled (Ollama returns 404 with an error body).
                Err(CtxError::ModelNotFound(format!(
                    "Ollama model '{}' not found. Pull it with: ollama pull {}",
                    self.model, self.model
                )))
            }
            s if s.is_server_error() => {
                let body = response.text().await.unwrap_or_default();
                Err(CtxError::embedding(format!(
                    "server error ({}): {}",
                    status, body
                )))
            }
            _ => {
                let body = response.text().await.unwrap_or_default();
                // Prefer a structured {"error": ...} message when present.
                if let Ok(parsed) = serde_json::from_str::<OllamaEmbedResponse>(&body) {
                    if let Some(err) = parsed.error {
                        return Err(Self::classify_error(&self.model, err));
                    }
                }
                Err(CtxError::embedding(format!("HTTP {}: {}", status, body)))
            }
        }
    }

    /// Map an Ollama error string to the most specific `CtxError`.
    fn classify_error(model: &str, message: String) -> CtxError {
        if message.contains("not found") || message.contains("try pulling") {
            CtxError::ModelNotFound(format!(
                "Ollama model '{}' not found. Pull it with: ollama pull {}",
                model, model
            ))
        } else {
            CtxError::embedding(message)
        }
    }

    /// Parse a successful `/api/embed` body into embeddings.
    fn parse_response(&self, response: OllamaEmbedResponse) -> Result<Vec<Embedding>> {
        if let Some(err) = response.error {
            return Err(Self::classify_error(&self.model, err));
        }
        let embeddings = response
            .embeddings
            .ok_or_else(|| CtxError::embedding("No embeddings in Ollama response"))?;
        if embeddings.is_empty() {
            return Err(CtxError::embedding(
                "Ollama returned an empty embeddings list",
            ));
        }
        // Once the dimension is known, enforce consistency across responses.
        if self.dimension != 0 {
            for vector in &embeddings {
                if vector.len() != self.dimension {
                    return Err(CtxError::DimensionMismatch {
                        expected: self.dimension,
                        actual: vector.len(),
                    });
                }
            }
        }
        Ok(embeddings.into_iter().map(Embedding::new).collect())
    }
}

impl EmbeddingProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Embedding> {
        self.request(&[text])?
            .pop()
            .ok_or_else(|| CtxError::embedding("Empty response"))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        // Ollama accepts an array input directly; chunk defensively for very
        // large batches to bound request size.
        const BATCH_SIZE: usize = 64;
        let mut all = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH_SIZE) {
            all.extend(self.request(chunk)?);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_normalizes_bare_host_port() {
        std::env::set_var("OLLAMA_HOST", "localhost:11434");
        assert_eq!(configured_host(), "http://localhost:11434");
        std::env::set_var("OLLAMA_HOST", "http://example.com:11434");
        assert_eq!(configured_host(), "http://example.com:11434");
        std::env::remove_var("OLLAMA_HOST");
        assert_eq!(configured_host(), DEFAULT_HOST);
    }

    #[test]
    fn model_defaults_when_unset() {
        std::env::remove_var("OLLAMA_EMBED_MODEL");
        assert_eq!(configured_model(), DEFAULT_MODEL);
        std::env::set_var("OLLAMA_EMBED_MODEL", "mxbai-embed-large");
        assert_eq!(configured_model(), "mxbai-embed-large");
        std::env::remove_var("OLLAMA_EMBED_MODEL");
    }

    /// Build a provider without a probe so `parse_response`/dimension logic can be
    /// unit-tested offline.
    fn offline_provider(dimension: usize) -> OllamaProvider {
        OllamaProvider {
            client: Client::new(),
            host: DEFAULT_HOST.to_string(),
            model: "test-model".to_string(),
            dimension,
        }
    }

    #[test]
    fn parse_response_success() {
        let provider = offline_provider(3);
        let parsed = OllamaEmbedResponse {
            embeddings: Some(vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]]),
            error: None,
        };
        let out = provider.parse_response(parsed).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].vector, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn parse_response_dimension_mismatch() {
        let provider = offline_provider(3);
        let parsed = OllamaEmbedResponse {
            embeddings: Some(vec![vec![0.1, 0.2]]), // wrong dim
            error: None,
        };
        assert!(matches!(
            provider.parse_response(parsed).unwrap_err(),
            CtxError::DimensionMismatch {
                expected: 3,
                actual: 2
            }
        ));
    }

    #[test]
    fn parse_response_model_not_found() {
        let provider = offline_provider(0);
        let parsed = OllamaEmbedResponse {
            embeddings: None,
            error: Some("model \"foo\" not found, try pulling it first".to_string()),
        };
        assert!(matches!(
            provider.parse_response(parsed).unwrap_err(),
            CtxError::ModelNotFound(_)
        ));
    }

    #[test]
    fn parse_response_empty() {
        let provider = offline_provider(0);
        let parsed = OllamaEmbedResponse {
            embeddings: Some(vec![]),
            error: None,
        };
        assert!(provider.parse_response(parsed).is_err());
    }
}
