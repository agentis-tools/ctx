//! OpenAI embedding provider.
//!
//! Uses the OpenAI API to generate embeddings via text-embedding-3-small.

use std::io::{Read, Write};
use std::net::TcpStream;

use super::{Embedding, EmbeddingError, EmbeddingProvider, Result, OPENAI_EMBEDDING_DIM};

/// OpenAI embedding provider configuration.
pub struct OpenAIProvider {
    api_key: String,
    model: String,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider with the given API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "text-embedding-3-small".to_string(),
        }
    }

    /// Create a provider from the OPENAI_API_KEY environment variable.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| EmbeddingError::InvalidApiKey)?;
        Ok(Self::new(api_key))
    }

    /// Set the model to use for embeddings.
    #[allow(dead_code)]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Make an HTTP request to the OpenAI API.
    fn request(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        // Build the request body
        let input_json = texts
            .iter()
            .map(|t| format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",");

        let body = format!(
            r#"{{"input":[{}],"model":"{}","encoding_format":"float"}}"#,
            input_json, self.model
        );

        // Make HTTPS request using native TLS
        let response = self.https_post("api.openai.com", "/v1/embeddings", &body)?;

        // Parse the response
        self.parse_response(&response)
    }

    /// Make an HTTPS POST request.
    fn https_post(&self, host: &str, path: &str, body: &str) -> Result<String> {
        // For simplicity, we'll use a basic HTTP request format
        // In production, you'd want to use a proper HTTP client like reqwest

        // Use native-tls for HTTPS
        let connector = native_tls::TlsConnector::new()
            .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

        let stream = TcpStream::connect(format!("{}:443", host))
            .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

        let mut stream = connector
            .connect(host, stream)
            .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

        // Build HTTP request
        let request = format!(
            "POST {} HTTP/1.1\r\n\
             Host: {}\r\n\
             Authorization: Bearer {}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            path,
            host,
            self.api_key,
            body.len(),
            body
        );

        stream
            .write_all(request.as_bytes())
            .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|e| EmbeddingError::NetworkError(e.to_string()))?;

        // Extract body from HTTP response
        if let Some(idx) = response.find("\r\n\r\n") {
            Ok(response[idx + 4..].to_string())
        } else {
            Err(EmbeddingError::NetworkError("Invalid HTTP response".into()))
        }
    }

    /// Parse the OpenAI API response.
    fn parse_response(&self, response: &str) -> Result<Vec<Embedding>> {
        // Handle chunked transfer encoding by finding the JSON start
        let json_start = response
            .find('{')
            .ok_or_else(|| EmbeddingError::ApiError("No JSON in response".into()))?;
        let response = &response[json_start..];

        // Parse as JSON
        let value: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| EmbeddingError::SerializationError(format!("JSON parse error: {}", e)))?;

        // Check for errors
        if let Some(error) = value.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");

            if message.contains("rate limit") {
                return Err(EmbeddingError::RateLimited(60));
            }
            if message.contains("invalid api key") || message.contains("Incorrect API key") {
                return Err(EmbeddingError::InvalidApiKey);
            }
            return Err(EmbeddingError::ApiError(message.to_string()));
        }

        // Extract embeddings
        let data = value
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| EmbeddingError::ApiError("No data in response".into()))?;

        let mut embeddings = Vec::with_capacity(data.len());

        for item in data {
            let vector = item
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| EmbeddingError::ApiError("No embedding in response".into()))?
                .iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect::<Vec<_>>();

            if vector.len() != OPENAI_EMBEDDING_DIM {
                return Err(EmbeddingError::DimensionMismatch {
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
            .ok_or_else(|| EmbeddingError::ApiError("Empty response".into()))
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
        let provider = OpenAIProvider::new("test-key");
        assert_eq!(provider.name(), "openai");
        assert_eq!(provider.dimension(), OPENAI_EMBEDDING_DIM);
    }

    // Integration test - requires OPENAI_API_KEY
    #[test]
    #[ignore]
    fn test_embed_real() {
        let provider = OpenAIProvider::from_env().expect("OPENAI_API_KEY not set");
        let embedding = provider.embed("Hello, world!").expect("Embedding failed");
        assert_eq!(embedding.dim(), OPENAI_EMBEDDING_DIM);
    }
}
